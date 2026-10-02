//! Native channel-strip routing editor. Accepted edits keep the output running.
use super::*;
use resonara_core::{Bus, ChannelRouting, Destination, Insert, InsertKind, Send};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum RoutingTarget {
    Track(usize),
    Bus(BusId),
}
impl RoutingTarget {
    pub(super) fn get(self, project: &Project) -> Option<&ChannelRouting> {
        match self {
            Self::Track(index) => project.tracks.get(index).map(|t| &t.routing),
            Self::Bus(id) => project.bus(id).map(|b| &b.routing),
        }
    }
    pub(super) fn get_mut(self, project: &mut Project) -> Option<&mut ChannelRouting> {
        match self {
            Self::Track(index) => project.tracks.get_mut(index).map(|t| &mut t.routing),
            Self::Bus(id) => project.bus_mut(id).map(|b| &mut b.routing),
        }
    }
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum RoutingMenu {
    Output(RoutingTarget),
    Send(RoutingTarget),
}
#[derive(Clone)]
pub(super) struct SendControl {
    gain: State<f32>,
    pub(super) dragging: State<bool>,
    pub(super) focused: State<bool>,
}
impl Channel {
    pub(super) fn new(id: u64) -> Self {
        Self {
            gain: state(id, 1.),
            gain_normalized: state(id + 9, fader::gain_to_fraction(1.)),
            pan: state(id + 1, 0.),
            meter: state(id + 2, "Peak −∞ dBFS".into()),
            peak: state(id + 6, meter::StereoMeter::default()),
            focused: state(id + 7, false),
            pan_focused: state(id + 8, false),
            dragging_gain: state(id + 3, false),
            dragging_pan: state(id + 4, false),
            canvas: SgfxCanvasHandle::new(),
            playhead_mesh: SgfxMeshHandle::new(),
            frame: state(id + 5, Arc::new(SgfxCanvasFrame::new(0, BG))),
            mesh: SgfxMesh::new(vec![]),
        }
    }
}
impl Daw {
    pub(super) fn refresh_bus_channels(m: &mut Model) {
        while m.bus_channels.len() < m.project.buses.len() {
            let id = 1_000_000 + m.bus_channels.len() as u64 * 10;
            m.bus_channels.push(Channel::new(id));
        }
        m.bus_channels.truncate(m.project.buses.len());
        for (channel, bus) in m.bus_channels.iter().zip(&m.project.buses) {
            channel.gain.set(bus.gain);
            channel
                .gain_normalized
                .set(fader::gain_to_fraction(bus.gain));
            channel.pan.set(bus.pan);
        }
    }
    pub(super) fn choose_bus(&self, id: BusId) {
        self.insert_focus.borrow_mut().clear();
        if self.busy() {
            return;
        }
        self.finish_mix();
        self.routing_menu.set(None);
        let mut m = self.model.borrow_mut();
        if m.project.bus(id).is_some() {
            m.selected_bus = Some(id);
            m.clip = None;
        }
        drop(m);
        self.inspector.set(true);
        self.refresh(false);
    }
    pub(super) fn add_bus(&self) {
        self.create_aux_route(None);
    }
    /// Logic-style flow: a chosen new bus creates its receiving Aux strip in
    /// the same transaction. Aux submixes and send returns share the same DSP.
    pub(super) fn create_aux_route(&self, connection: Option<RoutingMenu>) {
        self.create_aux_route_at(connection, None);
    }
    pub(super) fn create_aux_route_at(
        &self,
        connection: Option<RoutingMenu>,
        replace_send: Option<usize>,
    ) {
        if self.busy() {
            return;
        }
        let mut project = self.model.borrow().project.clone();
        let mut number = 1;
        while project
            .buses
            .iter()
            .any(|b| b.name == format!("Aux {number}"))
        {
            number += 1;
        }
        let id = project.add_bus(format!("Aux {number}"), BusKind::Aux);
        if let Some(connection) = connection {
            let (RoutingMenu::Output(target) | RoutingMenu::Send(target)) = connection;
            let Some(routing) = target.get_mut(&mut project) else {
                return;
            };
            match connection {
                RoutingMenu::Output(_) => routing.output = Destination::Bus(id),
                RoutingMenu::Send(_) => {
                    if let Some(index) = replace_send {
                        let Some(send) = routing.sends.get_mut(index) else {
                            return;
                        };
                        send.target = id;
                    } else {
                        routing.sends.push(Send {
                            target: id,
                            gain: 0.25,
                            pre_fader: false,
                            enabled: true,
                        });
                    }
                }
            }
        }
        if let Err(error) = project.validate_routing() {
            self.status
                .set(format!("Could not add aux channel: {error}"));
            return;
        }
        self.edit("Create aux channel and bus", |m| {
            m.project = project;
            if connection.is_none() {
                m.selected_bus = Some(id);
                m.clip = None;
            }
            Ok(())
        });
        self.inspector.set(true);
    }
    pub(super) fn delete_bus(&self, id: BusId) {
        self.edit("Delete bus and repair routes", |m| {
            if !m.project.remove_bus(id) {
                return Err("Bus no longer exists".into());
            }
            m.selected_bus = None;
            m.project.validate_routing()
        });
    }
    pub(super) fn bus_mix(&self, id: BusId, gain: Option<f32>, pan: Option<f32>, toggle: bool) {
        let mut m = self.model.borrow_mut();
        if m.io.is_some() || m.picker.is_some() {
            return;
        }
        let Some(index) = m.project.buses.iter().position(|b| b.id == id) else {
            return;
        };
        if m.mixer_before.is_none() {
            m.mixer_before = Some(Self::snapshot(&m));
        }
        let bus = &mut m.project.buses[index];
        if let Some(value) = gain {
            bus.gain = value.clamp(0., 2.);
        }
        if let Some(value) = pan {
            bus.pan = value.clamp(-1., 1.);
        }
        if toggle {
            bus.mute = !bus.mute;
        }
        if let Some(audio) = &m.audio {
            if let Some(control) = audio.controls.buses.get(index) {
                control.set(&m.project.buses[index]);
            }
        }
        drop(m);
        if toggle {
            self.finish_mix();
        }
        self.refresh(false);
    }
    pub(super) fn routing_edit(
        &self,
        target: RoutingTarget,
        label: &str,
        change: impl FnOnce(&mut ChannelRouting) -> Result<()>,
    ) {
        if self.busy() {
            return;
        }
        // Build and validate a candidate before replacing the graph. No-op and
        // rejected edits leave the live stream, history and effect state alone.
        let mut candidate = self.model.borrow().project.clone();
        let Some(routing) = target.get_mut(&mut candidate) else {
            return;
        };
        let before = routing.clone();
        if let Err(error) = change(routing) {
            self.status.set(format!("Could not {label}: {error}"));
            return;
        }
        if *routing == before {
            self.routing_menu.set(None);
            return;
        }
        let next = routing.clone();
        if let Err(error) = candidate.validate_routing() {
            self.status.set(format!("Could not {label}: {error}"));
            return;
        }
        self.edit(label, |m| {
            *target
                .get_mut(&mut m.project)
                .ok_or("Channel no longer exists")? = next;
            Ok(())
        });
    }
    pub(super) fn set_output(&self, target: RoutingTarget, output: Destination) {
        self.routing_menu.set(None);
        if target
            .get(&self.model.borrow().project)
            .is_some_and(|r| r.output == output)
        {
            return;
        }
        self.routing_edit(target, "Change output routing", |routing| {
            routing.output = output;
            Ok(())
        });
    }
    pub(super) fn add_send(&self, target: RoutingTarget, bus: BusId) {
        self.routing_edit(target, "Add send", |routing| {
            if routing.sends.iter().any(|s| s.target == bus) {
                return Err("This channel already sends to that bus".into());
            }
            routing.sends.push(Send {
                target: bus,
                gain: 0.25,
                pre_fader: false,
                enabled: true,
            });
            Ok(())
        });
    }
    pub(super) fn add_insert(&self, target: RoutingTarget, kind: InsertKind) {
        self.routing_edit(target, "Add insert", |routing| {
            routing.inserts.push(Insert {
                kind,
                bypass: false,
            });
            Ok(())
        });
    }
    pub(super) fn move_insert(&self, target: RoutingTarget, index: usize, direction: isize) {
        let destination = index as isize + direction;
        let valid = target.get(&self.model.borrow().project).is_some_and(|r| {
            index < r.inserts.len() && destination >= 0 && (destination as usize) < r.inserts.len()
        });
        if !valid {
            return;
        }
        self.routing_edit(target, "Move insert", |routing| {
            routing.inserts.swap(index, destination as usize);
            Ok(())
        });
    }
    pub(super) fn toggle_insert(&self, target: RoutingTarget, index: usize) {
        self.routing_edit(target, "Bypass insert", |routing| {
            let insert = routing
                .inserts
                .get_mut(index)
                .ok_or("Insert no longer exists")?;
            insert.bypass = !insert.bypass;
            Ok(())
        });
    }
    pub(super) fn remove_insert(&self, target: RoutingTarget, index: usize) {
        self.routing_edit(target, "Remove insert", |routing| {
            if index >= routing.inserts.len() {
                return Err("Insert no longer exists".into());
            }
            routing.inserts.remove(index);
            Ok(())
        });
    }
    pub(super) fn edit_insert_value(&self, target: RoutingTarget, index: usize) {
        if self.busy() {
            return;
        }
        let m = self.model.borrow();
        let Some(insert) = target.get(&m.project).and_then(|r| r.inserts.get(index)) else {
            return;
        };
        let value = insert_input(&insert.kind);
        self.routing_value.set(value);
        self.dialog_error.set(String::new());
        self.routing_menu.set(None);
        self.dialog.set(Dialog::InsertValue(target, index));
    }
    pub(super) fn submit_insert_value(&self, target: RoutingTarget, index: usize) {
        let kind = target
            .get(&self.model.borrow().project)
            .and_then(|r| r.inserts.get(index))
            .map(|i| i.kind.clone());
        if kind
            .as_ref()
            .is_some_and(|kind| self.routing_value.get().trim() == insert_input(kind))
        {
            self.dialog.set(Dialog::None);
            return;
        }
        let parsed = parse_insert(kind, &self.routing_value.get());
        match parsed {
            Ok(kind) => {
                self.dialog.set(Dialog::None);
                self.routing_edit(target, "Set insert parameter", |routing| {
                    routing
                        .inserts
                        .get_mut(index)
                        .ok_or("Insert no longer exists")?
                        .kind = kind;
                    Ok(())
                });
            }
            Err(error) => self.dialog_error.set(error.into()),
        }
    }
    pub(super) fn insert_value_dialog(&self, target: RoutingTarget, index: usize) -> AnyView {
        let kind = target
            .get(&self.model.borrow().project)
            .and_then(|r| r.inserts.get(index))
            .map(|i| i.kind.clone());
        let (title, help) = match kind {
            Some(InsertKind::Gain { .. }) => ("Gain · dB", "Enter −120 (silence) to +12 dB"),
            Some(InsertKind::OnePole { .. }) => (
                "Low-pass · coefficient",
                "Enter 0 to 0.9999. Higher values give more smoothing.",
            ),
            Some(InsertKind::Delay { .. }) => (
                "Delay · samples",
                "Enter 1 to 1,000,000 device-rate samples. Feed-forward, 100% wet.",
            ),
            Some(InsertKind::Clap { .. }) => ("CLAP", "Use the generic plugin editor"),
            None => (
                "Insert unavailable",
                "Close this editor and select an existing insert.",
            ),
        };
        let submit = self.clone();
        let cancel = self.clone();
        self.insert_popup(AnyView::new(vstack!{
            label(title).font_size(22.), caption(help),
            ui::field(self.routing_value.clone()).on_submit(move||submit.submit_insert_value(target,index)).on_cancel(move||cancel.dialog.set(Dialog::None)).autofocus(true).frame_width(300.).input_guard(),
            Text::from_state(self.dialog_error.clone()).font_size(12.).color(GOLD),
            row!{self.button("Cancel","Discard parameter input · Escape",|s|s.dialog.set(Dialog::None)),self.button("Apply","Apply insert parameter",move|s|s.submit_insert_value(target,index))}.spacing(8.)
        }.alignment(Alignment::TopLeading).spacing(14.).padding(24.)), 480.)
    }
    pub(super) fn routing_button(
        &self,
        text: impl Into<String>,
        help: &'static str,
        action: impl Fn(Self) + 'static,
    ) -> AnyView {
        let s = self.clone();
        let status = self.status.clone();
        AnyView::new(
            ui::compact_button(text)
                .font_size(10.)
                .on_click(move || action(s.clone()))
                .on_hover(move || status.set(help.into()))
                .frame_height(24.),
        )
    }
    fn toggle_routing_menu(&self, menu: RoutingMenu) {
        if self.busy() {
            return;
        }
        self.routing_menu
            .set(if self.routing_menu.get() == Some(menu) {
                None
            } else {
                Some(menu)
            });
    }
    pub(super) fn routing_rack_height(&self, project: &Project, target: RoutingTarget) -> f32 {
        let Some(routing) = target.get(project) else {
            return 0.;
        };
        let menu = match self.routing_menu.get() {
            Some(RoutingMenu::Output(t) | RoutingMenu::Send(t)) if t == target => {
                (project.buses.len() + 2) as f32 * 28.
            }
            _ => 0.,
        };
        177. + routing.inserts.len() as f32 * 26. + routing.sends.len() as f32 * 26. + menu
    }
    pub(super) fn routing_rack(&self, project: &Project, target: RoutingTarget) -> AnyView {
        let Some(routing) = target.get(project) else {
            return AnyView::new(Spacer::new());
        };
        let output_name = destination_name(project, routing.output);
        let mut rows: Vec<Box<dyn View>> = vec![
            Box::new(Rectangle::new().fill(LINE).frame(190., 1.)),
            Box::new(caption("OUTPUT")),
            Box::new(
                self.routing_button(
                    format!("{}  ▾", ui::elide(&output_name, 23)),
                    "Choose this channel's main output",
                    move |s| s.toggle_routing_menu(RoutingMenu::Output(target)),
                )
                .frame_width(190.),
            ),
        ];
        if self.routing_menu.get() == Some(RoutingMenu::Output(target)) {
            rows.push(Box::new(
                self.routing_button("Stereo Out", "Route directly to the master", move |s| {
                    s.set_output(target, Destination::Master)
                })
                .frame_width(190.),
            ));
            for bus in &project.buses {
                if target == RoutingTarget::Bus(bus.id) {
                    continue;
                }
                let id = bus.id;
                rows.push(Box::new(
                    self.routing_button(
                        destination_name(project, Destination::Bus(id)),
                        "Route to this bus; feedback cycles are rejected",
                        move |s| s.set_output(target, Destination::Bus(id)),
                    )
                    .frame_width(190.),
                ));
            }
            rows.push(Box::new(
                self.routing_button(
                    "New Bus → Aux",
                    "Create an aux channel and route this output to its bus",
                    move |s| s.create_aux_route(Some(RoutingMenu::Output(target))),
                )
                .frame_width(190.),
            ));
        }
        rows.push(Box::new(
            row! {caption("INSERTS"),Spacer::new(),caption("pre-fader").font_size(9.)}
                .frame_width(190.),
        ));
        let mut slots: Vec<Box<dyn View>> = Vec::new();
        for (index, insert) in routing.inserts.iter().enumerate() {
            let name = match &insert.kind {
                InsertKind::Clap { plugin } => {
                    if resonara_core::plugins::is_available(plugin) {
                        plugin.name.clone()
                    } else {
                        format!("{} (missing)", plugin.name)
                    }
                }
                kind => insert_label(kind).0.to_owned(),
            };
            let open = self.clone();
            let bypass = self.clone();
            let context = self.clone();
            slots.push(Box::new(
                insert_slot::InsertSlot::new(
                    index + 1,
                    name,
                    insert.bypass,
                    false,
                    self.insert_slot_focus(target, index),
                    move || open.open_insert_editor(target, index),
                    move || bypass.toggle_insert(target, index),
                    move || context.open_insert_actions(target, index),
                )
                .frame(190., 26.),
            ));
        }
        let add = self.clone();
        slots.push(Box::new(
            insert_slot::InsertSlot::empty(
                routing.inserts.len() + 1,
                self.insert_slot_focus(target, routing.inserts.len()),
                move || add.open_insert_picker(target),
            )
            .frame(190., 26.),
        ));
        rows.push(Box::new(
            VStack::new(Children(slots))
                .spacing(0.)
                .frame_width(190.)
                .border(LINE, 1.),
        ));
        rows.push(Box::new(caption("SENDS")));
        let mut send_slots: Vec<Box<dyn View>> = Vec::new();
        for (index, send) in routing.sends.iter().enumerate() {
            let control = self.send_control(target, index, send.gain);
            let change = self.clone();
            let open = self.clone();
            let context = self.clone();
            send_slots.push(Box::new(
                send_slot::SendSlot::new(
                    destination_name(project, Destination::Bus(send.target)),
                    send_level_text(send.gain),
                    send.enabled,
                    control.gain,
                    control.dragging,
                    control.focused,
                    move |value| change.send_gain(target, index, value),
                    move || open.open_send_actions(target, index),
                    move || context.open_send_actions(target, index),
                )
                .frame(190., 26.),
            ));
        }
        let add = self.clone();
        send_slots.push(Box::new(
            send_slot::SendSlot::empty(
                self.send_control(target, routing.sends.len(), 0.).focused,
                move || add.open_send_picker(target, None),
            )
            .frame(190., 26.),
        ));
        rows.push(Box::new(
            VStack::new(Children(send_slots))
                .spacing(0.)
                .frame_width(190.)
                .border(LINE, 1.),
        ));
        AnyView::new(
            VStack::new(Children(rows))
                .alignment(Alignment::TopLeading)
                .spacing(4.)
                .frame_width(190.),
        )
    }
    fn send_control(&self, target: RoutingTarget, index: usize, gain: f32) -> SendControl {
        let mut controls = self.send_controls.borrow_mut();
        let control = controls
            .entry((target, index))
            .or_insert_with(|| SendControl {
                gain: State::new(scarlet_ui::state::generate_state_id(), gain),
                dragging: State::new(scarlet_ui::state::generate_state_id(), false),
                focused: State::new(scarlet_ui::state::generate_state_id(), false),
            });
        control.gain.set(gain);
        control.clone()
    }
    pub(super) fn send_gain(&self, target: RoutingTarget, index: usize, value: f32) {
        if !value.is_finite() || self.busy() {
            return;
        }
        let value = value.clamp(0., 2.);
        let mut m = self.model.borrow_mut();
        let Some(previous) = target
            .get(&m.project)
            .and_then(|r| r.sends.get(index))
            .map(|s| s.gain)
        else {
            return;
        };
        if previous == value {
            return;
        }
        let control_index = match target {
            RoutingTarget::Track(track) => m.project.send_control_index(Some(track), None, index),
            RoutingTarget::Bus(bus) => m.project.send_control_index(None, Some(bus), index),
        };
        if m.mixer_before.is_none() {
            m.mixer_before = Some(Self::snapshot(&m));
        }
        target.get_mut(&mut m.project).unwrap().sends[index].gain = value;
        if let Some(control) = m
            .audio
            .as_ref()
            .and_then(|a| control_index.and_then(|i| a.controls.send_gains.get(i)))
        {
            control.store(value.to_bits(), Ordering::Relaxed);
        }
        drop(m);
        if let Some(control) = self.send_controls.borrow().get(&(target, index)) {
            control.gain.set(value);
        }
        self.changed();
    }
    pub(super) fn edit_send(&self, target: RoutingTarget, index: usize, action: SendEdit) {
        self.routing_edit(target, "Edit send", |routing| {
            let send = routing
                .sends
                .get_mut(index)
                .ok_or("Send no longer exists")?;
            match action {
                SendEdit::Enable => send.enabled = !send.enabled,
                SendEdit::PrePost => send.pre_fader = !send.pre_fader,
                SendEdit::Remove => {
                    routing.sends.remove(index);
                }
            }
            Ok(())
        });
    }
    pub(super) fn bus_strip(
        &self,
        bus: &Bus,
        c: &Channel,
        fader_height: f32,
        strip_height: f32,
        selected: bool,
    ) -> AnyView {
        let id = bus.id;
        let select = self.clone();
        let status = self.status.clone();
        let full_name = bus.name.clone();
        AnyView::new(vstack!{
            Rectangle::new().fill(ACCENT).frame(90.,3.),
            ui::button(ui::elide(&bus.name,14)).font_size(10.).on_click(move||select.choose_bus(id)).frame(92.,24.).on_hover(move||status.set(full_name.clone())),
            self.channel_strip_controls(RoutingTarget::Bus(id),c,bus.gain,bus.pan,bus.mute,None,fader_height,false)
        }.spacing(2.).padding(4.).frame(100.,strip_height-2.).background(if selected{RAISED}else{PANEL}).border(LINE,1.))
    }
}
#[derive(Clone, Copy)]
pub(super) enum SendEdit {
    Enable,
    PrePost,
    Remove,
}
pub(super) fn destination_name(project: &Project, destination: Destination) -> String {
    match destination {
        Destination::Master => "Stereo Out".into(),
        Destination::Bus(id) => project.bus(id).map_or_else(
            || "Missing bus".into(),
            |b| format!("Bus {} → {}", id.0, b.name),
        ),
    }
}
fn insert_label(kind: &InsertKind) -> (&'static str, String) {
    match kind {
        InsertKind::Gain { gain } => ("Gain", ui::db(*gain)),
        InsertKind::OnePole { coefficient } => ("Low-pass", format!("Coef {coefficient:.3}")),
        InsertKind::Delay { frames } => ("Delay", format!("{frames} samples")),
        InsertKind::Clap { plugin } => ("CLAP", plugin.name.clone()),
    }
}
fn parse_insert(
    kind: Option<InsertKind>,
    value: &str,
) -> std::result::Result<InsertKind, &'static str> {
    match kind {
        Some(InsertKind::Gain { .. }) => value
            .trim()
            .parse::<f32>()
            .ok()
            .filter(|v| v.is_finite() && (-120. ..=12.).contains(v))
            .map(|db| InsertKind::Gain {
                gain: if db <= -120. {
                    0.
                } else {
                    10f32.powf(db / 20.)
                },
            })
            .ok_or("Enter a finite gain between −120 and +12 dB"),
        Some(InsertKind::OnePole { .. }) => value
            .trim()
            .parse::<f32>()
            .ok()
            .filter(|v| v.is_finite() && (0. ..=0.9999).contains(v))
            .map(|coefficient| InsertKind::OnePole { coefficient })
            .ok_or("Enter a coefficient between 0 and 0.9999"),
        Some(InsertKind::Delay { .. }) => value
            .trim()
            .parse::<usize>()
            .ok()
            .filter(|v| (1..=1_000_000).contains(v))
            .map(|frames| InsertKind::Delay { frames })
            .ok_or("Enter an integer between 1 and 1,000,000 samples"),
        Some(InsertKind::Clap { .. }) => Err("Use the CLAP parameter editor"),
        None => Err("Insert no longer exists"),
    }
}

fn insert_input(kind: &InsertKind) -> String {
    match *kind {
        InsertKind::Gain { gain } => {
            if gain > 0. {
                format!("{:.2}", 20. * gain.log10())
            } else {
                "-120".into()
            }
        }
        InsertKind::OnePole { coefficient } => format!("{coefficient:.4}"),
        InsertKind::Delay { frames } => frames.to_string(),
        InsertKind::Clap { .. } => String::new(),
    }
}

pub(super) fn send_level_text(gain: f32) -> String {
    if gain <= 0.00001 {
        "−∞".into()
    } else {
        format!("{:.1}", 20. * gain.log10())
    }
}
