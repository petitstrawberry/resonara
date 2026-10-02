//! Shared lower workspace. Each editor owns its controls and viewport; tabs
//! select the task, while the editor content follows the selected material.
use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PanelTab {
    Mixer = 0,
    Editor = 1,
}

impl Daw {
    pub(super) fn panel_mode(&self) -> PanelTab {
        match self.panel_tab.get() {
            1 => PanelTab::Editor,
            _ => PanelTab::Mixer,
        }
    }

    pub(super) fn toggle_panel(&self, tab: PanelTab) {
        if self.panel_visible.get() && self.panel_mode() == tab {
            self.panel_visible.set(false);
        } else {
            self.show_panel(tab);
        }
    }

    pub(super) fn show_panel(&self, tab: PanelTab) {
        self.finish_mix();
        self.panel_tab.set(tab as usize);
        self.panel_visible.set(true);
        if tab == PanelTab::Editor {
            self.refresh_region_editor();
        }
    }

    pub(super) fn open_editor_panel(&self) {
        self.show_panel(PanelTab::Editor);
    }

    pub(super) fn lower_panel_header(&self) -> AnyView {
        // Use ScarletUI's tab hit testing, hover/press behavior and bound
        // selection state. The zero-height content lets channel actions share
        // this strip; the material workspace is composed just below it.
        let tabs = TabView::with_selected_index(
            vec![
                TabItem::new("Mixer", || Spacer::new()),
                TabItem::new("Editor", || Spacer::new()),
            ],
            self.panel_tab.clone(),
        )
        .tab_bar_placement(TabBarPlacement::Top)
        .tab_bar_height(MIXER_HEADER_HEIGHT)
        .tab_padding(16.)
        .font_size(11.)
        .style(TabStyle {
            background_color: PANEL,
            selected_color: PANEL,
            hover_color: RAISED,
            border_color: LINE,
            text_color: MUTED,
            selected_text_color: TEXT,
            indicator_color: ACCENT,
            indicator_height: 2.,
        })
        .frame(138., MIXER_HEADER_HEIGHT);
        let detail = match self.panel_mode() {
            PanelTab::Mixer => {
                let m = self.model.borrow();
                AnyView::new(row! {
                    caption(format!("{} audio · {} aux", m.project.tracks.len(), m.project.buses.len())),
                    self.header_button("+ Aux", "Create an aux channel and its input bus", |s|s.add_bus())
                }.spacing(10.))
            }
            PanelTab::Editor => AnyView::new(caption("AUDIO")),
        };
        AnyView::new(
            row! {
                tabs,
                Rectangle::new().fill(LINE).frame(1., 16.),
                detail,
                Spacer::new(),
                self.header_icon(Icon::X, "Close lower pane", false, |s|s.panel_visible.set(false))
            }
            .spacing(6.)
            .padding_insets(EdgeInsets::new(8., 0., 8., 0.))
            .frame_height(MIXER_HEADER_HEIGHT)
            .background(PANEL),
        )
    }

    pub(super) fn lower_panel(&self) -> AnyView {
        if self.panel_mode() == PanelTab::Editor {
            self.refresh_region_editor();
        }
        match self.panel_mode() {
            PanelTab::Mixer => self.mixer(),
            // Add editor dispatch here as the project acquires new material
            // types (MIDI notes, drum patterns, etc.). The pane is shared.
            PanelTab::Editor => AnyView::new(
                vstack! {
                    self.lower_panel_header(),
                    self.region_editor_view().frame(f32::INFINITY, f32::INFINITY)
                }
                .spacing(0.)
                .frame(f32::INFINITY, f32::INFINITY)
                .background(BG),
            ),
        }
    }
}
