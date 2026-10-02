//! Full-height selected-channel inspector, independent of the right mixer split.
use super::*;
use resonara_core::Destination;

impl Daw {
    pub(super) fn channel_inspector_panel(&self) -> AnyView {
        let m = self.model.borrow();
        let target = m
            .selected_bus
            .map_or(RoutingTarget::Track(m.selected), RoutingTarget::Bus);
        let selected = match target {
            RoutingTarget::Track(i) => m.project.tracks.get(i).map(|t| {
                (
                    t.name.clone(),
                    t.gain,
                    t.pan,
                    t.mute,
                    Some(t.solo),
                    &m.channels[i],
                )
            }),
            RoutingTarget::Bus(id) => m.project.buses.iter().position(|b| b.id == id).map(|i| {
                let b = &m.project.buses[i];
                (
                    b.name.clone(),
                    b.gain,
                    b.pan,
                    b.mute,
                    None,
                    &m.bus_channels[i],
                )
            }),
        };
        let height = (self.size.get().height - 174.).max(320.);
        let resize = self.clone();
        let panel = if let Some((name, gain, pan, mute, solo, channel)) = selected {
            let rename = self.clone();
            let inspector_title = row!{caption("INSPECTOR"),Spacer::new(),self.icon(Icon::X,"Hide inspector · I",false,|s|s.inspector.set(false))}
                .frame_height(24.).padding_insets(EdgeInsets::new(14.,6.,14.,6.)).frame_height(36.);
            let name_field = ui::field(self.track_name.clone())
                .on_submit(move || {
                    let value = rename.track_name.get();
                    let value = value.trim();
                    if value == name {
                        return;
                    }
                    rename.edit("Rename channel", |m| {
                        if value.is_empty() {
                            return Err("Channel name cannot be empty".into());
                        }
                        match target {
                            RoutingTarget::Track(i) => {
                                m.project
                                    .tracks
                                    .get_mut(i)
                                    .ok_or("Track no longer exists")?
                                    .name = value.into()
                            }
                            RoutingTarget::Bus(id) => {
                                m.project.bus_mut(id).ok_or("Aux no longer exists")?.name =
                                    value.into()
                            }
                        }
                        Ok(())
                    });
                })
                .blur_on_submit(true)
                .frame(
                    if matches!(target, RoutingTarget::Bus(_)) {
                        160.
                    } else {
                        190.
                    },
                    24.,
                )
                .input_guard();
            let name_row = match target {
                RoutingTarget::Track(_) => name_field,
                RoutingTarget::Bus(id) => {
                    AnyView::new(row! {name_field, self.aux_operations_button(id)}.spacing(6.))
                }
            };
            let heading=AnyView::new(vstack!{
                name_row,
                caption(match target {RoutingTarget::Track(_)=>"AUDIO TRACK".to_owned(),RoutingTarget::Bus(id)=>format!("AUX · INPUT Bus {}",id.0)}).font_size(10.).frame_height(12.)
            }.alignment(Alignment::TopLeading).spacing(3.).padding_insets(EdgeInsets::new(14.,0.,14.,3.)).frame_height(42.));
            // Region/source metadata is separate from the channel's signal path.
            // Nothing is inserted between Output → Inserts → Sends and its fader.
            let mut metadata: Vec<Box<dyn View>> = Vec::new();
            let mut metadata_height = 0.;
            match target {
                RoutingTarget::Track(i) => {
                    let track = &m.project.tracks[i];
                    if let Some((index, clip)) = m
                        .clip
                        .and_then(|c| track.clips.get(c).map(|clip| (c, clip)))
                    {
                        metadata.push(Box::new(self.routing_details_button(index + 1)));
                        metadata_height += 32.;
                        if self.inspector_details.get() {
                            metadata.push(Box::new(
                                caption(format!(
                                    "Start {:.3}s · length {:.3}s",
                                    clip.start as f64 / m.project.sample_rate as f64,
                                    clip.frames as f64 / m.project.sample_rate as f64
                                ))
                                .font_size(10.),
                            ));
                            metadata.push(Box::new(caption("RANGE · SECONDS")));
                            metadata.push(Box::new(row!{ui::field(self.cursor.clone()).frame(90.,24.).input_guard(),ui::field(self.range_end.clone()).frame(90.,24.).input_guard()}.spacing(8.)));
                            metadata.push(Box::new(
                                self.routing_button(
                                    "Keep range on track",
                                    "Keep only audio inside the specified range",
                                    |s| s.trim_range(),
                                )
                                .frame_width(190.),
                            ));
                            metadata_height += 120.;
                        }
                        metadata.push(Box::new(Rectangle::new().fill(LINE).frame(190., 1.)));
                        metadata_height += 9.;
                    }
                }
                RoutingTarget::Bus(id) => {
                    metadata.push(Box::new(caption("INPUTS")));
                    metadata_height += 24.;
                    let mut count = 0;
                    for (name, routing) in m
                        .project
                        .tracks
                        .iter()
                        .map(|t| (&t.name, &t.routing))
                        .chain(m.project.buses.iter().map(|b| (&b.name, &b.routing)))
                    {
                        let output = routing.output == Destination::Bus(id);
                        let send = routing.sends.iter().any(|s| s.target == id);
                        if output || send {
                            metadata.push(Box::new(
                                caption(format!(
                                    "{} · {}",
                                    ui::elide(name, 18),
                                    if output { "output" } else { "send" }
                                ))
                                .font_size(10.),
                            ));
                            count += 1;
                        }
                    }
                    if count == 0 {
                        metadata.push(Box::new(caption("No input routes yet").font_size(10.)));
                        count = 1;
                    }
                    metadata_height += count as f32 * 24.;
                }
            }
            let fader_height = self.mixer_fader_height();
            let scroll = ScrollView::new(
                vstack! {
                    inspector_title,
                    VStack::new(Children(metadata)).alignment(Alignment::TopLeading)
                        .spacing(8.).padding_insets(EdgeInsets::new(14.,0.,14.,0.)),
                    heading,
                    self.routing_rack(&m.project, target).padding_insets(EdgeInsets::new(14.,0.,14.,8.))
                }
                .spacing(0.)
                .alignment(Alignment::TopLeading),
            )
            .vertical()
            .content_size(220., self.routing_rack_height(&m.project, target) + metadata_height + 106.)
            .frame_height(
                (height - fader_height - channel_strip::INSPECTOR_CONTROL_OVERHEAD).max(0.),
            );
            let fader = self.inspector_channel_fader(target, channel, gain, pan, mute, solo);
            AnyView::new(
                vstack! {scroll,fader}
                    .spacing(0.)
                    .frame_height(height)
                    .background(PANEL),
            )
        } else {
            AnyView::new(
                vstack! {caption("INSPECTOR"),caption("No track selected"),Spacer::new()}
                    .alignment(Alignment::TopLeading)
                    .spacing(10.)
                    .padding(14.)
                    .frame_height(height)
                    .background(PANEL),
            )
        };
        AnyView::new(panel.on_geometry_change(
            |g| g.size().width,
            move |width| {
                let fraction = width / (resize.size.get().width - 4.);
                if (fraction - resize.inspector_fraction.get()).abs() > 0.001 {
                    resize.inspector_fraction.set(fraction);
                }
            },
        ))
    }
    fn aux_operations_button(&self, id: BusId) -> AnyView {
        let anchor = Rc::new(Cell::new(Point::ZERO));
        let open_anchor = anchor.clone();
        let open = self.clone();
        let hover = self.clone();
        AnyView::new(AuxMenuAnchor {
            content: AnyView::new(
                ui::compact_button("⋯")
                    .font_size(16.)
                    .on_click(move || open.open_aux_menu(id, open_anchor.get()))
                    .on_hover(move || hover.status.set("Aux channel options".into()))
                    .frame(24., 24.),
            ),
            anchor,
        })
    }
    fn routing_details_button(&self, region: usize) -> AnyView {
        AnyView::new(
            self.routing_button(
                format!(
                    "REGION {region} {}",
                    if self.inspector_details.get() {
                        "▾"
                    } else {
                        "▸"
                    }
                ),
                "Show selected region positions and precise trim range",
                |s| s.inspector_details.set(!s.inspector_details.get()),
            )
            .frame_width(190.),
        )
    }
    fn inspector_channel_fader(
        &self,
        target: RoutingTarget,
        channel: &Channel,
        gain: f32,
        pan: f32,
        mute: bool,
        solo: Option<bool>,
    ) -> AnyView {
        let fader_height = self.mixer_fader_height();
        let controls =
            self.channel_strip_controls(target, channel, gain, pan, mute, solo, fader_height, true);
        AnyView::new(
            row! {Spacer::new(),controls.padding(4.).frame(100.,fader_height+110.),Spacer::new()}
                .spacing(0.)
                .padding_insets(EdgeInsets::new(0., 4., 0., 4.))
                .frame_height(fader_height + channel_strip::INSPECTOR_CONTROL_OVERHEAD)
                .background(PANEL),
        )
    }
}

// GeometryProxy currently reports only local size. Keep a transparent marker
// around the stock button so the root input boundary can resolve its actual
// window position, including the inspector's scroll offset, before activation.
#[derive(Clone)]
struct AuxMenuAnchor {
    content: AnyView,
    anchor: Rc<Cell<Point>>,
}
impl View for AuxMenuAnchor {
    fn create_element(&self) -> Box<dyn scarlet_ui::Element> {
        Box::new(scarlet_ui::RenderElement::with_view_children_and_updater(
            self.clone(),
            |view| AuxMenuAnchorRender {
                anchor: view.anchor.clone(),
                size: Size::ZERO,
            },
            |render, view| {
                render.anchor = view.anchor.clone();
                scarlet_ui::element::UpdateResult::NoChange
            },
            |view| vec![Box::new(view.content.clone())],
        ))
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}
struct AuxMenuAnchorRender {
    anchor: Rc<Cell<Point>>,
    size: Size,
}
impl scarlet_ui::ElementRenderObject for AuxMenuAnchorRender {
    fn layout(&mut self, _: scarlet_ui::LayoutConstraints) -> Size {
        self.size
    }
    fn layout_with_children(
        &mut self,
        constraints: scarlet_ui::LayoutConstraints,
        children: &mut [Box<dyn scarlet_ui::Element>],
    ) -> Size {
        self.size = children
            .first_mut()
            .map_or(Size::ZERO, |child| child.layout(constraints));
        self.size
    }
    fn size(&self) -> Size {
        self.size
    }
    fn render(&mut self) {}
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}
pub(super) fn update_aux_menu_anchor(element: &dyn scarlet_ui::Element, parent: Point) {
    let origin = Point::new(
        parent.x + element.position().x,
        parent.y + element.position().y,
    );
    if let Some(marker) = element
        .render_object()
        .and_then(|render| render.as_any().downcast_ref::<AuxMenuAnchorRender>())
    {
        marker
            .anchor
            .set(Point::new(origin.x, origin.y + marker.size.height + 3.));
    }
    for child in element.children() {
        update_aux_menu_anchor(child.as_ref(), origin);
    }
}

#[cfg(test)]
#[path = "aux_menu_tests.rs"]
mod tests;
