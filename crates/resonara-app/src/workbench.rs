//! Track management and timeline navigation, independent of native menu APIs.
use super::*;
#[derive(Clone, Copy, PartialEq)]
pub(super) struct TrackMenu {
    pub target: Option<usize>,
    pub anchor: Point,
}
#[derive(Clone, Copy)]
pub(super) enum TrackAction {
    Add,
    Duplicate,
    Delete,
}
const MENU_WIDTH: f32 = 220.;
#[derive(Clone, Copy)]
pub(super) struct RulerDrag {
    before: f64,
    resume: bool,
    start: f64,
    span: f64,
    width: f32,
}
impl Daw {
    pub(super) fn header_button(
        &self,
        text: &str,
        help: &'static str,
        action: impl Fn(Self) + 'static,
    ) -> AnyView {
        let s = self.clone();
        let h = self.clone();
        AnyView::new(
            ui::compact_button(text)
                .on_click(move || action(s.clone()))
                .on_hover(move || h.status.set(help.into()))
                .frame_height(CONTROL_HEIGHT),
        )
    }
    pub(super) fn header_icon(
        &self,
        icon: Icon,
        help: &'static str,
        active: bool,
        action: impl Fn(Self) + 'static,
    ) -> AnyView {
        let s = self.clone();
        let h = self.clone();
        AnyView::new(
            Button::icon_only(icon)
                .icon_size(IconSize::Small)
                .padding(4.)
                .background_color(if active { ACCENT } else { RAISED })
                .icon_color(if active { BG } else { TEXT })
                .border_color(LINE)
                .on_click(move || action(s.clone()))
                .on_hover(move || h.status.set(help.into()))
                .frame(CONTROL_HEIGHT, CONTROL_HEIGHT),
        )
    }
    pub(super) fn follow_position(&self, position: f64) {
        if !self.follow_playhead.get()
            || self.follow_suspended.get()
            || self.ruler_drag.borrow().is_some()
        {
            return;
        }
        let start = self.view_start.get();
        let span = self.view_span.get();
        if position < start || position >= start + span * 0.9 {
            let next = (position - span * 0.1).max(0.);
            if next != start {
                self.view_start.set(next);
                self.refresh(true);
            }
        }
    }
    pub(super) fn toggle_follow(&self) {
        self.follow_suspended.set(false);
        self.follow_playhead.set(!self.follow_playhead.get());
        self.follow_position(self.playhead.get());
    }
    pub(super) fn suspend_follow(&self) {
        if self.follow_playhead.get() && self.model.borrow().audio.is_some() {
            self.follow_suspended.set(true);
        }
    }
    pub(super) fn scroll_timeline(&self, delta: f64) {
        self.suspend_follow();
        self.view_start.set((self.view_start.get() + delta).max(0.));
        self.refresh(true);
    }
    pub(super) fn add_track(&self, after: Option<usize>) {
        self.edit("Add audio track", |m| {
            let at = after.map_or(m.project.tracks.len(), |i| {
                (i + 1).min(m.project.tracks.len())
            });
            m.project.tracks.insert(
                at,
                resonara_core::Track {
                    name: format!("Audio track {}", m.project.tracks.len() + 1),
                    clips: vec![],
                    gain: 1.,
                    pan: 0.,
                    mute: false,
                    solo: false,
                },
            );
            m.selected = at;
            m.clip = None;
            Ok(())
        });
    }
    pub(super) fn open_track_menu(&self, target: Option<usize>, anchor: Point) {
        if self.busy() || self.dialog.get() != Dialog::None {
            return;
        }
        if let Some(i) = target {
            if i >= self.model.borrow().project.tracks.len() {
                return;
            }
            self.choose(i, None);
        }
        let height = if target.is_some() { 108. } else { 44. };
        let size = self.size.get();
        self.menu_choice.set(0);
        self.track_menu.set(Some(TrackMenu {
            target,
            anchor: Point::new(
                anchor.x.clamp(0., (size.width - MENU_WIDTH).max(0.)),
                anchor.y.clamp(0., (size.height - height).max(0.)),
            ),
        }));
    }
    pub(super) fn track_menu_action(&self, action: TrackAction) {
        let Some(menu) = self.track_menu.get() else {
            return;
        };
        self.track_menu.set(None);
        if let Some(i) = menu.target {
            if i >= self.model.borrow().project.tracks.len() {
                return;
            }
            self.choose(i, None);
        }
        match action {
            TrackAction::Add => self.add_track(menu.target),
            TrackAction::Duplicate if menu.target.is_some() => self.duplicate(),
            TrackAction::Delete if menu.target.is_some() => self.delete(true),
            _ => {}
        }
    }
    pub(super) fn track_context_event(
        &self,
        root: &dyn scarlet_ui::Element,
        e: &Event,
        control: bool,
    ) -> bool {
        if self.ruler_drag.borrow().is_some() {
            match e {
                Event::Keyboard(KeyEvent::Pressed {
                    keycode: KeyCode::Escape,
                    ..
                }) => {
                    self.cancel_ruler_drag();
                    return true;
                }
                Event::Keyboard(_) | Event::Mouse(MouseEvent::Wheel { .. }) => return true,
                _ => {}
            }
        }
        if let Some(menu) = self.track_menu.get() {
            match e {
                Event::Keyboard(KeyEvent::Pressed { keycode, .. }) => {
                    let count = if menu.target.is_some() { 3 } else { 1 };
                    match keycode {
                        KeyCode::Escape => self.track_menu.set(None),
                        KeyCode::Down => self.menu_choice.set((self.menu_choice.get() + 1) % count),
                        KeyCode::Up => self
                            .menu_choice
                            .set((self.menu_choice.get() + count - 1) % count),
                        KeyCode::Enter => self.track_menu_action(
                            [
                                TrackAction::Add,
                                TrackAction::Duplicate,
                                TrackAction::Delete,
                            ][self.menu_choice.get()],
                        ),
                        _ => {}
                    }
                    return true;
                }
                Event::Keyboard(_) => return true,
                Event::Mouse(MouseEvent::ButtonCancelled { .. }) => {
                    self.track_menu.set(None);
                    return true;
                }
                Event::Mouse(MouseEvent::Wheel { .. }) => return true,
                Event::Mouse(MouseEvent::ButtonPressed { x, y, .. }) => {
                    let height = if menu.target.is_some() { 108. } else { 44. };
                    if !Rect::from_xywh(menu.anchor.x, menu.anchor.y, MENU_WIDTH, height)
                        .contains(Point::new(*x as f32, *y as f32))
                    {
                        self.track_menu.set(None);
                        return true;
                    }
                }
                _ => {}
            }
            return false;
        }
        if let Event::Mouse(MouseEvent::ButtonPressed { button, x, y, .. }) = e {
            if *button == MouseButton::Right || (*button == MouseButton::Left && control) {
                let point = Point::new(*x as f32, *y as f32);
                if let Some(target) = ui::track_at(root, Point::ZERO, point) {
                    self.open_track_menu(target, point);
                    return true;
                }
            }
        }
        false
    }
    pub(super) fn track_menu_view(&self, menu: TrackMenu) -> AnyView {
        let mut rows: Vec<Box<dyn View>> = vec![];
        for (index, (text, action)) in [
            ("Add audio track", TrackAction::Add),
            ("Duplicate track", TrackAction::Duplicate),
            ("Delete track", TrackAction::Delete),
        ]
        .into_iter()
        .enumerate()
        {
            if index > 0 && menu.target.is_none() {
                break;
            }
            let s = self.clone();
            rows.push(Box::new(
                ui::button(text)
                    .background_color(if self.menu_choice.get() == index {
                        RAISED
                    } else {
                        PANEL
                    })
                    .on_click(move || s.track_menu_action(action))
                    .frame(MENU_WIDTH - 12., 32.),
            ));
        }
        AnyView::new(
            Surface::overlay(VStack::new(Children(rows)).spacing(0.).padding(6.))
                .fill(PANEL)
                .border_color(LINE)
                .frame_width(MENU_WIDTH)
                .padding_insets(EdgeInsets::new(menu.anchor.x, menu.anchor.y, 0., 0.)),
        )
    }
    fn ruler_position(&self, x: i32, drag: RulerDrag) -> f64 {
        let m = self.model.borrow();
        let end = if m.project.duration() == 0 && self.metronome.get() {
            86400.
        } else {
            m.project.duration() as f64 / m.project.sample_rate as f64
        };
        let position = (drag.start
            + (x as f32 / drag.width.max(1.)).clamp(0., 1.) as f64 * drag.span)
            .clamp(0., end);
        if self.snap.get() {
            self.snap_grid_for(&m.project)
                .position(position)
                .clamp(0., end)
        } else {
            position
        }
    }
    fn ruler_preview(&self, position: f64) {
        self.cursor.set(crate::timeline::seconds_input(position));
        self.animate_playhead(position);
    }
    pub(super) fn ruler_event(&self, e: &Event, start: f64, span: f64, width: f32) -> bool {
        match e {
            Event::Mouse(MouseEvent::ButtonPressed {
                button: MouseButton::Left,
                x,
                ..
            }) => {
                if self.busy() {
                    return false;
                }
                let resume = self.model.borrow().audio.is_some();
                let drag = RulerDrag {
                    before: self.playhead.get(),
                    resume,
                    start,
                    span,
                    width,
                };
                *self.ruler_drag.borrow_mut() = Some(drag);
                self.ruler_preview(self.ruler_position(*x, drag));
                true
            }
            Event::Mouse(MouseEvent::Moved { x, .. }) => {
                let Some(drag) = *self.ruler_drag.borrow() else {
                    return false;
                };
                self.ruler_preview(self.ruler_position(*x, drag));
                true
            }
            Event::Mouse(MouseEvent::ButtonReleased {
                button: MouseButton::Left,
                x,
                ..
            }) => {
                let Some(drag) = self.ruler_drag.borrow_mut().take() else {
                    return false;
                };
                let was_playing = self.model.borrow().audio.is_some();
                self.seek(self.ruler_position(*x, drag));
                if drag.resume && !was_playing {
                    self.play();
                }
                true
            }
            Event::Mouse(MouseEvent::ButtonCancelled { .. }) => self.cancel_ruler_drag(),
            _ => false,
        }
    }
    pub(super) fn cancel_ruler_drag(&self) -> bool {
        let Some(drag) = self.ruler_drag.borrow_mut().take() else {
            return false;
        };
        if drag.resume {
            // No seek was committed: keep the existing stream at its current position.
            let position = {
                let m = self.model.borrow();
                m.audio.as_ref().map_or(self.playhead.get(), |audio| {
                    audio.controls.position.load(Ordering::Relaxed) as f64
                        / m.project.sample_rate as f64
                })
            };
            self.ruler_preview(position);
        } else {
            self.seek(drag.before);
        }
        true
    }
}
