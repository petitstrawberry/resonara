//! The single shared channel-control component used by Mixer and Inspector.
use super::*;
pub(super) const INSPECTOR_CONTROL_OVERHEAD: f32 = 118.;
impl Daw {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn channel_strip_controls(
        &self,
        target: RoutingTarget,
        c: &Channel,
        gain_value: f32,
        pan_value: f32,
        muted: bool,
        soloed: Option<bool>,
        fader_height: f32,
        inspector: bool,
    ) -> AnyView {
        let gain = self.clone();
        let pan = self.clone();
        let mute = self.clone();
        let solo = self.clone();
        let meter_status = self.status.clone();
        let meter_peak = c.peak.clone();
        // Values, metering and gesture state are shared; focus is per mounted
        // control, so keyboard input is never delivered to both copies.
        let gain_focus = if inspector {
            self.inspector_fader_focus.clone()
        } else {
            c.focused.clone()
        };
        let pan_focus = if inspector {
            self.inspector_pan_focus.clone()
        } else {
            c.pan_focused.clone()
        };
        let mut buttons: Vec<Box<dyn View>> = vec![Box::new(
            ui::button("M")
                .background_color(if muted { GOLD } else { RAISED })
                .text_color(if muted { BG } else { TEXT })
                .on_click(move || match target {
                    RoutingTarget::Track(i) => mute.mix(i, None, None, Some(false)),
                    RoutingTarget::Bus(id) => mute.bus_mix(id, None, None, true),
                })
                .frame(24., 24.),
        )];
        if let Some(soloed) = soloed {
            buttons.push(Box::new(
                ui::button("S")
                    .background_color(if soloed { GOLD } else { RAISED })
                    .text_color(if soloed { BG } else { TEXT })
                    .on_click(move || {
                        if let RoutingTarget::Track(i) = target {
                            solo.mix(i, None, None, Some(true));
                        }
                    })
                    .frame(24., 24.),
            ));
        }
        // Center the interactive controls as one group on the pan/fader axis.
        // The Aux role label uses a side gutter, never participates in centering M.
        let group_width = if soloed.is_some() { 54. } else { 24. };
        let gutter = (90. - group_width) / 2.;
        let right = if soloed.is_some() {
            AnyView::new(Spacer::new().frame(gutter, 24.))
        } else {
            AnyView::new(
                caption("AUX")
                    .font_size(10.)
                    .alignment(Alignment::Center)
                    .frame(gutter, 24.),
            )
        };
        let switches = row! {
            Spacer::new().frame(gutter,24.),
            HStack::new(Children(buttons)).spacing(6.).frame(group_width,24.),
            right
        }
        .spacing(0.)
        .frame(90., 24.);
        AnyView::new(vstack!{
            switches,
            vstack!{knob::PanKnob::new(c.pan.clone(),c.dragging_pan.clone(),pan_focus,move|value|match target{RoutingTarget::Track(i)=>pan.mix(i,None,Some(value),None),RoutingTarget::Bus(id)=>pan.bus_mix(id,None,Some(value),false)}).frame(32.,32.),caption(ui::pan(pan_value)).font_size(10.)}.spacing(0.).frame(90.,44.),
            fader::Fader::new(c.gain.clone(),c.peak.clone(),c.dragging_gain.clone(),gain_focus,move|value|match target{RoutingTarget::Track(i)=>gain.mix(i,Some(value),None,None),RoutingTarget::Bus(id)=>gain.bus_mix(id,Some(value),None,false)}).frame(90.,fader_height),
            label(format!("Gain {}",ui::db(gain_value))).font_size(10.).alignment(Alignment::Center).frame(90.,14.),
            animation::Readout::new(c.meter.clone(),9.,ACCENT,Size::new(90.,12.)).on_hover(move||meter_status.set(meter_peak.get().detail(false)))
        }.spacing(2.).frame(90.,fader_height+102.))
    }
}
