//! Context actions for the same range commands used by the toolbar/shortcuts.
use super::*;
use scarlet_ui::Element;

const WIDTH: f32 = 224.;
const ROW_HEIGHT: f32 = 28.;
const ACTIONS: [(&str, Action); 10] = [
    ("Cut (copy + delete) · ⌘/Ctrl+X", Action::Cut),
    ("Copy · ⌘/Ctrl+C", Action::Copy),
    ("Paste · ⌘/Ctrl+V", Action::Paste),
    ("Delete (keep time)", Action::Delete),
    ("Silence selected audio", Action::Silence),
    ("Reverse selected audio", Action::Reverse),
    ("Trim to selection · ⌘/Ctrl+T", Action::Trim),
    ("Split · S", Action::Split),
    ("Select all · ⌘/Ctrl+A", Action::All),
    ("Zoom to selection", Action::Zoom),
];
#[derive(Clone, Copy)]
enum Action {
    Cut,
    Copy,
    Paste,
    Delete,
    Silence,
    Reverse,
    Trim,
    Split,
    All,
    Zoom,
}

fn waveform_rect(element: &dyn Element, parent: Point) -> Option<Rect> {
    let origin = Point::new(
        parent.x + element.position().x,
        parent.y + element.position().y,
    );
    if let Some(render) = element.render_object() {
        if render.as_any().is::<region_editor::SelectionRender>() {
            return Some(Rect::from_xywh(
                origin.x,
                origin.y,
                render.size().width,
                render.size().height,
            ));
        }
    }
    element
        .children()
        .iter()
        .find_map(|child| waveform_rect(child.as_ref(), origin))
}
impl Daw {
    fn editor_action_enabled(&self, action: Action) -> bool {
        let Some((_, _, clip, _, _)) = self.editor_selected() else {
            return false;
        };
        let range = self.region_editor.selection.get().range();
        let available = !self.region_processing_active() && !self.busy();
        match action {
            Action::Copy | Action::Zoom => !range.is_empty(),
            Action::All => true,
            Action::Paste => available && self.region_editor.clipboard.borrow().is_some(),
            Action::Split => {
                available
                    && range.start < clip.frames
                    && self.region_editor.selection.get().head > 0
            }
            _ => available && !range.is_empty(),
        }
    }
    fn run_editor_menu(&self, action: Action) {
        self.region_editor.menu.set(None);
        if !self.editor_action_enabled(action) {
            self.changed();
            return;
        }
        match action {
            Action::Cut => self.editor_cut(),
            Action::Copy => self.editor_copy(),
            Action::Paste => self.editor_paste(),
            Action::Delete => self.editor_delete_range(),
            Action::Silence => self.editor_silence(),
            Action::Reverse => self.editor_reverse_range(),
            Action::Trim => self.editor_trim(),
            Action::Split => self.editor_split(),
            Action::All => {
                if let Some((_, _, clip, _, _)) = self.editor_selected() {
                    self.editor_set_selection(region_editor::Selection {
                        anchor: 0,
                        head: clip.frames,
                    });
                }
            }
            Action::Zoom => self.editor_zoom_selection(),
        }
        self.changed();
    }
    pub(super) fn editor_context_event(
        &self,
        root: &dyn Element,
        event: &Event,
        control: bool,
    ) -> bool {
        let editor = &self.region_editor;
        if let Some(anchor) = editor.menu.get() {
            match event {
                Event::Keyboard(KeyEvent::Pressed { keycode, .. }) => {
                    match keycode {
                        KeyCode::Escape => {
                            editor.menu.set(None);
                            self.changed();
                        }
                        KeyCode::Down | KeyCode::Up => {
                            let direction = if *keycode == KeyCode::Down {
                                1
                            } else {
                                ACTIONS.len() - 1
                            };
                            for _ in 0..ACTIONS.len() {
                                editor
                                    .menu_choice
                                    .set((editor.menu_choice.get() + direction) % ACTIONS.len());
                                if self.editor_action_enabled(ACTIONS[editor.menu_choice.get()].1) {
                                    break;
                                }
                            }
                            self.changed();
                        }
                        KeyCode::Enter => self.run_editor_menu(ACTIONS[editor.menu_choice.get()].1),
                        _ => {}
                    }
                    return true;
                }
                Event::Keyboard(_) | Event::Mouse(MouseEvent::Wheel { .. }) => return true,
                Event::Mouse(MouseEvent::ButtonCancelled { .. }) => {
                    editor.menu.set(None);
                    self.changed();
                    return true;
                }
                Event::Mouse(MouseEvent::ButtonPressed { x, y, .. }) => {
                    if !Rect::from_xywh(
                        anchor.x,
                        anchor.y,
                        WIDTH,
                        ROW_HEIGHT * ACTIONS.len() as f32 + 12.,
                    )
                    .contains(Point::new(*x as f32, *y as f32))
                    {
                        editor.menu.set(None);
                        self.changed();
                        return true;
                    }
                }
                _ => {}
            }
            return false;
        }
        if !self.panel_visible.get()
            || self.panel_mode() != lower_panel::PanelTab::Editor
            || self.dialog.get() != Dialog::None
        {
            return false;
        }
        if let Event::Mouse(MouseEvent::ButtonPressed { button, x, y, .. }) = event {
            if *button == MouseButton::Right || (*button == MouseButton::Left && control) {
                let Some(rect) = waveform_rect(root, Point::ZERO) else {
                    return false;
                };
                let point = Point::new(*x as f32, *y as f32);
                if !rect.contains(point) {
                    return false;
                }
                if self.region_editor.selection.get().range().is_empty() {
                    if let Some((_, _, clip, _, _)) = self.editor_selected() {
                        let at = editor.frame_at(point.x - rect.origin.x, clip.frames);
                        self.editor_set_selection(region_editor::Selection {
                            anchor: at,
                            head: at,
                        });
                    }
                }
                self.track_menu.set(None);
                editor.menu.set(Some(Point::new(
                    point.x.min((self.size.get().width - WIDTH).max(0.)),
                    point.y.min(
                        (self.size.get().height - ROW_HEIGHT * ACTIONS.len() as f32 - 12.).max(0.),
                    ),
                )));
                editor.menu_choice.set(
                    ACTIONS
                        .iter()
                        .position(|(_, action)| self.editor_action_enabled(*action))
                        .unwrap_or(0),
                );
                self.changed();
                return true;
            }
        }
        false
    }
    pub(super) fn editor_menu_view(&self, anchor: Point) -> AnyView {
        let rows = ACTIONS
            .iter()
            .enumerate()
            .map(|(index, &(text, action))| {
                let clicked = self.clone();
                Box::new(
                    ui::compact_button(text)
                        .text_color(if self.editor_action_enabled(action) {
                            TEXT
                        } else {
                            MUTED
                        })
                        .background_color(if self.region_editor.menu_choice.get() == index {
                            RAISED
                        } else {
                            PANEL
                        })
                        .border_color(Color::TRANSPARENT)
                        .on_click(move || clicked.run_editor_menu(action))
                        .frame(WIDTH - 12., ROW_HEIGHT),
                ) as Box<dyn View>
            })
            .collect();
        AnyView::new(
            Surface::overlay(VStack::new(Children(rows)).spacing(0.).padding(6.))
                .fill(PANEL)
                .border_color(LINE)
                .frame_width(WIDTH)
                .padding_insets(EdgeInsets::new(anchor.x, anchor.y, 0., 0.)),
        )
    }
}
