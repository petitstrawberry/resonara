//! Insert slot popups and a generic CLAP parameter editor. Never used by audio callbacks.
use super::*;
use resonara_core::{InsertKind, plugins};

impl Daw {
    pub(super) fn clear_control_focus(&self) {
        self.focus.set(false);
        self.inspector_fader_focus.set(false);
        self.inspector_pan_focus.set(false);
        for state in self.insert_focus.borrow().values() {
            state.set(false);
        }
        for control in self.send_controls.borrow().values() {
            control.focused.set(false);
        }
        let m = self.model.borrow();
        for channel in m.channels.iter().chain(&m.bus_channels) {
            channel.focused.set(false);
            channel.pan_focused.set(false);
        }
    }
    pub(super) fn insert_slot_focus(&self, target: RoutingTarget, slot: usize) -> State<bool> {
        self.insert_focus
            .borrow_mut()
            .entry((target, slot))
            .or_insert_with(|| State::new(scarlet_ui::state::generate_state_id(), false))
            .clone()
    }
    pub(super) fn open_insert_picker(&self, target: RoutingTarget) {
        if self.busy() {
            return;
        }
        self.clear_control_focus();
        self.routing_menu.set(None);
        self.menu_choice.set(0);
        self.dialog_error.set(String::new());
        self.dialog.set(Dialog::InsertPicker(target));
    }
    pub(super) fn open_insert_actions(&self, target: RoutingTarget, slot: usize) {
        if self.busy() {
            return;
        }
        self.clear_control_focus();
        self.routing_menu.set(None);
        self.menu_choice.set(0);
        self.dialog.set(Dialog::InsertActions(target, slot));
    }
    pub(super) fn open_insert_editor(&self, target: RoutingTarget, slot: usize) {
        if self.busy() {
            return;
        }
        self.clear_control_focus();
        let kind = target
            .get(&self.model.borrow().project)
            .and_then(|r| r.inserts.get(slot))
            .map(|i| i.kind.clone());
        if let Some(InsertKind::Clap { plugin }) = kind {
            *self.plugin_fields.borrow_mut() = plugin
                .parameters
                .iter()
                .filter(|p| !p.hidden && !p.read_only)
                .map(|p| {
                    (
                        p.id,
                        State::new(scarlet_ui::state::generate_state_id(), p.value.to_string()),
                    )
                })
                .collect();
            self.dialog_error.set(String::new());
            self.dialog.set(Dialog::ClapEditor(target, slot));
        } else {
            self.edit_insert_value(target, slot);
        }
    }
    pub(super) fn insert_popup(&self, content: AnyView, width: f32) -> AnyView {
        let cancel = self.clone();
        AnyView::new(
            ZStack::new(Children(vec![
                Box::new(
                    Rectangle::new()
                        .fill(Color::rgba(0., 0., 0., 0.48))
                        .frame(self.size.get().width, self.size.get().height)
                        .on_click(move || cancel.dialog.set(Dialog::None)),
                ),
                Box::new(
                    Surface::overlay(content)
                        .fill(PANEL)
                        .border_color(LINE)
                        .frame_width(width),
                ),
            ]))
            .alignment(Alignment::Center)
            .frame(self.size.get().width, self.size.get().height),
        )
    }
    pub(super) fn insert_menu_item(
        &self,
        label: &str,
        index: usize,
        action: impl Fn(Self) + 'static,
    ) -> AnyView {
        self.insert_menu_item_width(label, index, 250., action)
    }
    fn insert_menu_item_width(
        &self,
        label: &str,
        index: usize,
        width: f32,
        action: impl Fn(Self) + 'static,
    ) -> AnyView {
        let s = self.clone();
        AnyView::new(
            ui::button(label)
                .background_color(if self.menu_choice.get() == index {
                    RAISED
                } else {
                    PANEL
                })
                .font_size(11.)
                .padding(4.)
                .on_click(move || action(s.clone()))
                .frame(width, 26.),
        )
    }
    pub(super) fn insert_picker_dialog(&self, target: RoutingTarget) -> AnyView {
        self.insert_popup(AnyView::new(vstack!{
            label("Add insert").font_size(17.),caption("BUILT-IN").font_size(10.),
            self.insert_menu_item("Gain",0,move|s|s.select_insert_kind(target,0)),
            self.insert_menu_item("Low-pass",1,move|s|s.select_insert_kind(target,1)),
            self.insert_menu_item("Delay",2,move|s|s.select_insert_kind(target,2)),
            caption("CLAP · NATIVE EFFECT").font_size(10.),
            self.insert_menu_item("Resonara Gain",3,move|s|s.select_insert_kind(target,3)),
            self.insert_menu_item("Installed CLAP effects…",4,move|s|s.select_insert_kind(target,4)),
            Text::from_state(self.dialog_error.clone()).font_size(11.).color(GOLD).frame_width(250.),
            self.insert_menu_item("Cancel",5,|s|s.dialog.set(Dialog::None)),
        }.alignment(Alignment::TopLeading).spacing(4.).padding(14.)),278.)
    }
    pub(super) fn select_insert_kind(&self, target: RoutingTarget, kind: usize) {
        if self.busy() {
            return;
        }
        if kind == 4 {
            self.open_clap_picker(target);
            return;
        }
        if kind == 5 {
            self.dialog.set(Dialog::None);
            return;
        }
        let selected = match kind {
            0 => InsertKind::Gain { gain: 1. },
            1 => InsertKind::OnePole { coefficient: 0.5 },
            2 => InsertKind::Delay { frames: 4800 },
            3 => match plugins::load_bundled_gain() {
                Ok(plugin) => InsertKind::Clap { plugin },
                Err(error) => {
                    self.dialog_error.set(format!("CLAP unavailable: {error}"));
                    return;
                }
            },
            _ => return,
        };
        self.dialog.set(Dialog::None);
        self.add_insert(target, selected);
    }
    pub(super) fn insert_actions_dialog(&self, target: RoutingTarget, slot: usize) -> AnyView {
        let info = target
            .get(&self.model.borrow().project)
            .and_then(|r| r.inserts.get(slot))
            .map(|i| {
                (
                    match &i.kind {
                        InsertKind::Gain { .. } => "Gain".into(),
                        InsertKind::OnePole { .. } => "Low-pass".into(),
                        InsertKind::Delay { .. } => "Delay".into(),
                        InsertKind::Clap { plugin } => plugin.name.clone(),
                    },
                    i.bypass,
                )
            });
        let (name, bypassed) = info.unwrap_or(("Insert unavailable".into(), false));
        self.insert_popup(AnyView::new(vstack!{
            label(format!("{}  {}",slot+1,name)).font_size(15.),
            self.insert_menu_item("Edit…",0,move|s|s.insert_action(target,slot,0)),
            self.insert_menu_item(if bypassed{"Enable"}else{"Bypass"},1,move|s|s.insert_action(target,slot,1)),
            self.insert_menu_item("Move earlier",2,move|s|s.insert_action(target,slot,2)),
            self.insert_menu_item("Move later",3,move|s|s.insert_action(target,slot,3)),
            self.insert_menu_item("Remove",4,move|s|s.insert_action(target,slot,4)),
            self.insert_menu_item("Cancel",5,|s|s.dialog.set(Dialog::None)),
        }.alignment(Alignment::TopLeading).spacing(2.).padding(14.)),278.)
    }
    pub(super) fn insert_action(&self, target: RoutingTarget, slot: usize, action: usize) {
        self.dialog.set(Dialog::None);
        match action {
            0 => self.open_insert_editor(target, slot),
            1 => self.toggle_insert(target, slot),
            2 => self.move_insert(target, slot, -1),
            3 => self.move_insert(target, slot, 1),
            4 => self.remove_insert(target, slot),
            _ => {}
        }
    }
    pub(super) fn handle_insert_popup_key(&self, key: KeyCode) -> bool {
        let count = match self.dialog.get() {
            Dialog::InsertPicker(_) => 6,
            Dialog::InsertActions(..) => 6,
            Dialog::ClapPicker(_) => self.plugin_catalog.borrow().effects.len() + 2,
            _ => return false,
        };
        match key {
            KeyCode::Down => self.menu_choice.set((self.menu_choice.get() + 1) % count),
            KeyCode::Up => self
                .menu_choice
                .set((self.menu_choice.get() + count - 1) % count),
            KeyCode::Enter => match self.dialog.get() {
                Dialog::InsertPicker(target) => {
                    self.select_insert_kind(target, self.menu_choice.get())
                }
                Dialog::InsertActions(target, slot) => {
                    self.insert_action(target, slot, self.menu_choice.get())
                }
                Dialog::ClapPicker(target) => self.select_clap(target, self.menu_choice.get()),
                _ => {}
            },
            KeyCode::Escape => self.dialog.set(Dialog::None),
            _ => {}
        }
        true
    }
    pub(super) fn open_clap_picker(&self, target: RoutingTarget) {
        if self.busy() {
            return;
        }
        self.clear_control_focus();
        *self.plugin_catalog.borrow_mut() = plugins::scan_installed();
        self.menu_choice.set(0);
        self.dialog_error.set(String::new());
        self.dialog.set(Dialog::ClapPicker(target));
    }
    pub(super) fn select_clap(&self, target: RoutingTarget, index: usize) {
        if self.busy() {
            return;
        }
        let catalog = self.plugin_catalog.borrow();
        if index == catalog.effects.len() {
            drop(catalog);
            self.open_clap_picker(target);
            return;
        }
        let Some(choice) = catalog.effects.get(index).cloned() else {
            self.dialog.set(Dialog::None);
            return;
        };
        drop(catalog);
        match plugins::load_installed(&choice) {
            Ok(plugin) => {
                self.dialog.set(Dialog::None);
                self.add_insert(target, InsertKind::Clap { plugin });
            }
            Err(error) => self
                .dialog_error
                .set(format!("Could not load {}: {error}", choice.name)),
        }
    }
    pub(super) fn clap_picker_dialog(&self, target: RoutingTarget) -> AnyView {
        let catalog = self.plugin_catalog.borrow();
        let mut effects: Vec<Box<dyn View>> = Vec::new();
        for (index, choice) in catalog.effects.iter().enumerate() {
            let s = self.clone();
            effects.push(Box::new(
                ui::button(format!("{} · {}", choice.name, choice.vendor))
                    .background_color(if self.menu_choice.get() == index {
                        RAISED
                    } else {
                        PANEL
                    })
                    .font_size(12.)
                    .padding(6.)
                    .on_click(move || s.select_clap(target, index))
                    .frame(440., 30.),
            ));
            effects.push(Box::new(
                caption(format!("{} · {}", choice.library, choice.plugin_id)).font_size(10.),
            ));
        }
        let count = catalog.effects.len();
        let warnings = catalog.warnings.join("\n");
        self.insert_popup(AnyView::new(vstack!{
            label("Installed CLAP effects").font_size(20.),
            caption("Stereo audio effects · native OS and CPU required").font_size(11.),
            ScrollView::new(VStack::new(Children(effects)).spacing(4.).alignment(Alignment::TopLeading))
                .scroll_to_index((self.menu_choice.get() < count).then_some(self.menu_choice.get()), 52.)
                .frame(440.,240.),
            caption(if count==0 { "No effects found. Install in the CLAP folder or set CLAP_PATH." } else { "Select an effect to add it to this channel." }).frame_width(440.),
            ScrollView::new(Text::new(warnings).font_size(11.).color(GOLD).frame_width(440.)).frame(440.,60.),
            Text::from_state(self.dialog_error.clone()).font_size(11.).color(GOLD).frame_width(440.),
            row!{
                self.insert_menu_item_width("Rescan",count,215.,move|s|s.open_clap_picker(target)),
                self.insert_menu_item_width("Cancel",count+1,215.,|s|s.dialog.set(Dialog::None))
            }.spacing(10.)
        }.alignment(Alignment::TopLeading).spacing(10.).padding(22.)),484.)
    }
    pub(super) fn clap_editor_dialog(&self, target: RoutingTarget, slot: usize) -> AnyView {
        let plugin = target
            .get(&self.model.borrow().project)
            .and_then(|r| r.inserts.get(slot))
            .and_then(|i| {
                if let InsertKind::Clap { plugin } = &i.kind {
                    Some(plugin.clone())
                } else {
                    None
                }
            });
        let Some(plugin) = plugin else {
            return self.insert_popup(
                AnyView::new(label("Plugin no longer exists").padding(24.)),
                420.,
            );
        };
        let mut rows: Vec<Box<dyn View>> = vec![
            Box::new(label(&plugin.name).font_size(20.)),
            Box::new(caption("CLAP · GENERIC PARAMETERS").font_size(10.)),
        ];
        if !plugins::is_available(&plugin) {
            rows.push(Box::new(
                Text::new(
                    "Plugin missing. Saved state is retained; playback bypasses this insert.",
                )
                .font_size(11.)
                .color(GOLD)
                .frame_width(380.),
            ));
        }
        let mut parameter_rows: Vec<Box<dyn View>> = Vec::new();
        for parameter in &plugin.parameters {
            if parameter.hidden {
                continue;
            }
            if parameter.read_only {
                parameter_rows.push(Box::new(row!{label(&parameter.name).font_size(12.).frame_width(140.),caption(format!("{} (read-only)",parameter.value)).font_size(12.)}.spacing(8.)));
                continue;
            }
            let Some((_, value)) = self
                .plugin_fields
                .borrow()
                .iter()
                .find(|(id, _)| *id == parameter.id)
                .cloned()
            else {
                continue;
            };
            let submit = self.clone();
            parameter_rows.push(Box::new(row!{label(&parameter.name).font_size(12.).frame_width(140.),ui::field(value).on_submit(move||submit.submit_clap_parameters(target,slot)).frame_width(130.).input_guard(),caption(format!("{} … {}",parameter.min,parameter.max)).font_size(10.)}.spacing(8.)));
        }
        rows.push(Box::new(
            ScrollView::new(
                VStack::new(Children(parameter_rows))
                    .spacing(12.)
                    .alignment(Alignment::TopLeading),
            )
            .frame(
                426.,
                (plugin.parameters.iter().filter(|p| !p.hidden).count() as f32 * 38.)
                    .clamp(38., 260.),
            ),
        ));
        rows.push(Box::new(
            Text::from_state(self.dialog_error.clone())
                .font_size(11.)
                .color(GOLD)
                .frame_width(380.),
        ));
        rows.push(Box::new(row!{self.button("Cancel","Close without changing plugin state",|s|s.dialog.set(Dialog::None)),self.button("Apply","Save parameter values and plugin state",move|s|s.submit_clap_parameters(target,slot))}.spacing(8.)));
        self.insert_popup(
            AnyView::new(
                VStack::new(Children(rows))
                    .alignment(Alignment::TopLeading)
                    .spacing(12.)
                    .padding(22.),
            ),
            470.,
        )
    }
    pub(super) fn submit_clap_parameters(&self, target: RoutingTarget, slot: usize) {
        let plugin = target
            .get(&self.model.borrow().project)
            .and_then(|r| r.inserts.get(slot))
            .and_then(|i| {
                if let InsertKind::Clap { plugin } = &i.kind {
                    Some(plugin.clone())
                } else {
                    None
                }
            });
        let Some(mut plugin) = plugin else {
            return;
        };
        let values = self
            .plugin_fields
            .borrow()
            .iter()
            .map(|(id, text)| (*id, text.get()))
            .collect::<Vec<_>>();
        let mut edits = Vec::new();
        for (id, text) in values {
            let Some(parameter) = plugin.parameters.iter().find(|p| p.id == id) else {
                continue;
            };
            let Some(value) = text
                .trim()
                .parse::<f64>()
                .ok()
                .filter(|v| v.is_finite() && *v >= parameter.min && *v <= parameter.max)
            else {
                self.dialog_error.set(format!(
                    "{} must be between {} and {}",
                    parameter.name, parameter.min, parameter.max
                ));
                return;
            };
            if value != parameter.value {
                edits.push((id, value));
            }
        }
        if edits.is_empty() {
            self.dialog.set(Dialog::None);
            return;
        }
        // Prepare state on a separate inactive instance on the UI thread.
        // Only a successful routing commit replaces the active graph; transport continues.
        // Failure preserves saved state, history, and ongoing playback.
        match plugins::set_parameters(&plugin, &edits) {
            Ok(updated) => plugin = updated,
            Err(error) => {
                self.dialog_error
                    .set(format!("Could not update CLAP: {error}"));
                return;
            }
        }
        self.dialog.set(Dialog::None);
        self.routing_edit(target, "Edit CLAP parameters", |routing| {
            routing
                .inserts
                .get_mut(slot)
                .ok_or("Insert no longer exists")?
                .kind = InsertKind::Clap { plugin };
            Ok(())
        });
    }
}
