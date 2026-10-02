//! Compact send-slot destination/context menus. Gain dragging stays in the rack.
use super::*;
use resonara_core::Destination;

impl Daw {
    pub(super) fn open_send_picker(&self, target: RoutingTarget, slot: Option<usize>) {
        if self.busy() {
            return;
        }
        self.clear_control_focus();
        self.routing_menu.set(None);
        self.menu_choice.set(0);
        self.dialog_error.set(String::new());
        self.dialog.set(Dialog::SendPicker(target, slot));
    }
    pub(super) fn open_send_actions(&self, target: RoutingTarget, slot: usize) {
        if self.busy() {
            return;
        }
        self.finish_mix();
        self.clear_control_focus();
        self.menu_choice.set(0);
        self.dialog.set(Dialog::SendActions(target, slot));
    }
    fn send_choices(&self, target: RoutingTarget, slot: Option<usize>) -> Vec<(BusId, String)> {
        let m = self.model.borrow();
        let Some(routing) = target.get(&m.project) else {
            return vec![];
        };
        m.project
            .buses
            .iter()
            .filter(|bus| {
                target != RoutingTarget::Bus(bus.id)
                    && !routing
                        .sends
                        .iter()
                        .enumerate()
                        .any(|(index, send)| Some(index) != slot && send.target == bus.id)
            })
            .map(|bus| {
                (
                    bus.id,
                    routing::destination_name(&m.project, Destination::Bus(bus.id)),
                )
            })
            .collect()
    }
    pub(super) fn send_picker_dialog(&self, target: RoutingTarget, slot: Option<usize>) -> AnyView {
        let choices = self.send_choices(target, slot);
        let count = choices.len();
        let mut rows: Vec<Box<dyn View>> = vec![Box::new(label("Send destination").font_size(15.))];
        for (index, (_, name)) in choices.into_iter().enumerate() {
            rows.push(Box::new(self.insert_menu_item(
                &ui::elide(&name, 35),
                index,
                move |s| s.choose_send_destination(target, slot, index),
            )));
        }
        rows.push(Box::new(self.insert_menu_item(
            "New Bus → Aux",
            count,
            move |s| s.choose_send_destination(target, slot, count),
        )));
        rows.push(Box::new(self.insert_menu_item("Cancel", count + 1, |s| {
            s.dialog.set(Dialog::None)
        })));
        self.insert_popup(
            AnyView::new(
                VStack::new(Children(rows))
                    .alignment(Alignment::TopLeading)
                    .spacing(1.)
                    .padding(12.),
            ),
            274.,
        )
    }
    pub(super) fn choose_send_destination(
        &self,
        target: RoutingTarget,
        slot: Option<usize>,
        choice: usize,
    ) {
        let choices = self.send_choices(target, slot);
        self.dialog.set(Dialog::None);
        if let Some((destination, _)) = choices.get(choice) {
            let destination = *destination;
            if let Some(index) = slot {
                self.routing_edit(target, "Change send destination", |routing| {
                    routing
                        .sends
                        .get_mut(index)
                        .ok_or("Send no longer exists")?
                        .target = destination;
                    Ok(())
                });
            } else {
                self.add_send(target, destination);
            }
        } else if choice == choices.len() {
            self.create_aux_route_at(Some(RoutingMenu::Send(target)), slot);
        }
    }
    pub(super) fn send_actions_dialog(&self, target: RoutingTarget, slot: usize) -> AnyView {
        let m = self.model.borrow();
        let send = target.get(&m.project).and_then(|r| r.sends.get(slot));
        let Some(send) = send else {
            return self.insert_popup(AnyView::new(label("Send unavailable").padding(16.)), 270.);
        };
        let name = routing::destination_name(&m.project, Destination::Bus(send.target));
        self.insert_popup(AnyView::new(vstack!{
            label(ui::elide(&name,32)).font_size(14.),caption(format!("{} · {}",ui::db(send.gain),if send.pre_fader{"Pre Fader"}else{"Post Pan"})).font_size(10.),
            self.insert_menu_item("Set level…",0,move|s|s.send_action(target,slot,0)),
            self.insert_menu_item("Show receiver",1,move|s|s.send_action(target,slot,1)),
            self.insert_menu_item("Change destination…",2,move|s|s.send_action(target,slot,2)),
            self.insert_menu_item(if send.enabled{"Bypass send"}else{"Enable send"},3,move|s|s.send_action(target,slot,3)),
            self.insert_menu_item(if send.pre_fader{"Switch to Post Pan"}else{"Switch to Pre Fader"},4,move|s|s.send_action(target,slot,4)),
            self.insert_menu_item("Remove send",5,move|s|s.send_action(target,slot,5)),
            self.insert_menu_item("Cancel",6,|s|s.dialog.set(Dialog::None)),
        }.alignment(Alignment::TopLeading).spacing(1.).padding(12.)),274.)
    }
    pub(super) fn send_action(&self, target: RoutingTarget, slot: usize, action: usize) {
        self.dialog.set(Dialog::None);
        match action {
            0 => {
                let gain = target
                    .get(&self.model.borrow().project)
                    .and_then(|r| r.sends.get(slot))
                    .map(|s| s.gain);
                if let Some(gain) = gain {
                    self.routing_value.set(send_db_input(gain));
                    self.dialog_error.set(String::new());
                    self.dialog.set(Dialog::SendLevel(target, slot));
                }
            }
            1 => {
                let id = target
                    .get(&self.model.borrow().project)
                    .and_then(|r| r.sends.get(slot))
                    .map(|s| s.target);
                if let Some(id) = id {
                    self.choose_bus(id);
                }
            }
            2 => self.open_send_picker(target, Some(slot)),
            3 => self.edit_send(target, slot, routing::SendEdit::Enable),
            4 => self.edit_send(target, slot, routing::SendEdit::PrePost),
            5 => self.edit_send(target, slot, routing::SendEdit::Remove),
            _ => {}
        }
    }
    pub(super) fn handle_send_popup_key(&self, key: KeyCode) -> bool {
        let count = match self.dialog.get() {
            Dialog::SendPicker(target, slot) => self.send_choices(target, slot).len() + 2,
            Dialog::SendActions(..) => 7,
            _ => return false,
        };
        match key {
            KeyCode::Up => self
                .menu_choice
                .set((self.menu_choice.get() + count - 1) % count),
            KeyCode::Down => self.menu_choice.set((self.menu_choice.get() + 1) % count),
            KeyCode::Enter => match self.dialog.get() {
                Dialog::SendPicker(target, slot) => {
                    self.choose_send_destination(target, slot, self.menu_choice.get())
                }
                Dialog::SendActions(target, slot) => {
                    self.send_action(target, slot, self.menu_choice.get())
                }
                _ => {}
            },
            KeyCode::Escape => self.dialog.set(Dialog::None),
            _ => {}
        }
        true
    }
    pub(super) fn send_level_dialog(&self, target: RoutingTarget, slot: usize) -> AnyView {
        let submit = self.clone();
        let cancel = self.clone();
        self.insert_popup(AnyView::new(vstack!{
            label("Send level · dB").font_size(17.),caption("−120 = silence · maximum +6.02 dB").font_size(11.),
            ui::field(self.routing_value.clone()).on_submit(move||submit.submit_send_level(target,slot)).on_cancel(move||cancel.dialog.set(Dialog::None)).autofocus(true).frame(180.,28.).input_guard(),
            Text::from_state(self.dialog_error.clone()).font_size(11.).color(GOLD),
            row!{self.button("Cancel","Discard typed level",|s|s.dialog.set(Dialog::None)),self.button("Apply","Set send level without interrupting playback",move|s|s.submit_send_level(target,slot))}.spacing(6.)
        }.alignment(Alignment::TopLeading).spacing(10.).padding(18.)),340.)
    }
    pub(super) fn submit_send_level(&self, target: RoutingTarget, slot: usize) {
        let Some(gain) = target
            .get(&self.model.borrow().project)
            .and_then(|r| r.sends.get(slot))
            .map(|s| s.gain)
        else {
            return;
        };
        let input = self.routing_value.get();
        if input.trim() == send_db_input(gain) {
            self.dialog.set(Dialog::None);
            return;
        }
        let Some(db) = input
            .trim()
            .parse::<f32>()
            .ok()
            .filter(|v| v.is_finite() && (-120. ..=20. * 2f32.log10()).contains(v))
        else {
            self.dialog_error
                .set("Enter a finite level from −120 to +6.02 dB".into());
            return;
        };
        self.dialog.set(Dialog::None);
        self.send_gain(
            target,
            slot,
            if db <= -120. {
                0.
            } else {
                10f32.powf(db / 20.)
            },
        );
        self.finish_mix();
    }
}
fn send_db_input(gain: f32) -> String {
    if gain <= 0. {
        "-120".into()
    } else {
        format!("{:.2}", 20. * gain.log10())
    }
}
