//! Host-integrated ScarletUI editor for the Resonara Freeverb CLAP DSP.
//! The host owns the window, parameter commit, transport, state and Undo.
//! This crate creates no event loop and never runs on the audio thread.
use scarlet_ui::{hstack as row, prelude::*, vstack};
use std::{any::Any, rc::Rc};
mod knob;
mod ui;
use knob::RotaryKnob;
use ui::*;

pub const PLUGIN_ID: &str = "org.resonara.freeverb";
pub const PARAMETER_IDS: [u32; 5] = [0, 1, 2, 3, 4];
pub const DEFAULTS: [f64; 5] = [0.3, 1., 0.5, 0.5, 1.];
pub const WIDTH: f32 = 548.;

fn round_value(value: f64) -> f64 {
    (value * 100.).round() / 100. + 0.0
}
fn format_value(value: f64) -> String {
    format!("{:.2}", round_value(value))
}

#[derive(Clone, Copy, Debug)]
pub enum Preset {
    Default,
    Room,
    Hall,
    AuxSend,
}
impl Preset {
    pub fn values(self) -> [f64; 5] {
        match self {
            Self::Default => DEFAULTS,
            Self::Room => [0.22, 1., 0.3, 0.7, 0.65],
            Self::Hall => [0.35, 1., 0.82, 0.45, 1.],
            Self::AuxSend => [1., 0., 0.75, 0.45, 1.],
        }
    }
}

#[derive(Clone)]
pub struct FreeverbEditor {
    fields: [State<String>; 5],
    knobs: [RotaryKnob; 5],
    error: State<String>,
    apply: Rc<dyn Fn()>,
    cancel: Rc<dyn Fn()>,
}
impl FreeverbEditor {
    pub fn new(values: [f64; 5]) -> std::result::Result<Self, &'static str> {
        if values
            .iter()
            .any(|v| !v.is_finite() || !(0. ..=1.).contains(v))
        {
            return Err("Freeverb values must be finite and between 0 and 1");
        }
        let fields = std::array::from_fn(|i| {
            State::new(
                scarlet_ui::state::generate_state_id(),
                format_value(values[i]),
            )
        });
        let knobs = std::array::from_fn(|i| {
            RotaryKnob::parameter(fields[i].clone(), 0., 1., values[i], false).unwrap()
        });
        Ok(Self {
            fields,
            knobs,
            error: State::new(scarlet_ui::state::generate_state_id(), String::new()),
            apply: Rc::new(|| {}),
            cancel: Rc::new(|| {}),
        })
    }
    pub fn fields(&self) -> [State<String>; 5] {
        self.fields.clone()
    }
    pub fn values(&self) -> std::result::Result<[f64; 5], &'static str> {
        let mut values = [0.; 5];
        for (i, field) in self.fields.iter().enumerate() {
            values[i] = field
                .get()
                .trim()
                .parse::<f64>()
                .ok()
                .filter(|v| v.is_finite() && (0. ..=1.).contains(v))
                .ok_or("Every value must be between 0 and 1")?;
            values[i] = round_value(values[i]);
        }
        Ok(values)
    }
    /// Normalize numeric drafts to the editor's 0.01 resolution before commit.
    pub fn normalize_drafts(&self) -> std::result::Result<(), &'static str> {
        let values = self.values()?;
        for (field, value) in self.fields.iter().zip(values) {
            field.set(format_value(value));
        }
        Ok(())
    }
    pub fn preset(&self, preset: Preset) {
        for (field, value) in self.fields.iter().zip(preset.values()) {
            field.set(format_value(value));
        }
        self.error.set(String::new());
    }
    pub fn on_apply(mut self, callback: impl Fn() + 'static) -> Self {
        self.apply = Rc::new(callback);
        self
    }
    pub fn on_cancel(mut self, callback: impl Fn() + 'static) -> Self {
        self.cancel = Rc::new(callback);
        self
    }
    pub fn error(mut self, error: State<String>) -> Self {
        self.error = error;
        self
    }
    fn control(&self, i: usize, name: &str, hint: &str, diameter: f32) -> AnyView {
        let apply = self.apply.clone();
        AnyView::new(vstack! {
            label(name).font_size(12.),
            self.knobs[i].clone().frame(diameter,diameter),
            field(self.fields[i].clone()).font_size(11.).on_submit(move||apply()).frame(100.,28.).on_key(|event|matches!(event,scarlet_ui::event::KeyEvent::Pressed{keycode:scarlet_ui::event::KeyCode::Char(_),..})),
            caption(hint).font_size(10.)
        }.alignment(Alignment::Center).spacing(6.).frame_width(144.))
    }
    fn preset_button(&self, name: &str, preset: Preset) -> Button {
        let editor = self.clone();
        button(name)
            .font_size(11.)
            .on_click(move || editor.preset(preset))
    }
    fn body(&self) -> AnyView {
        let apply = self.apply.clone();
        let cancel = self.cancel.clone();
        AnyView::new(VStack::new(Children(vec![
            Box::new(vstack! { caption("RESONARA · STEREO REVERB").font_size(10.).color(ACCENT),label("Freeverb").font_size(28.),caption("Space, tone and stereo ambience").font_size(12.) }.alignment(Alignment::TopLeading).spacing(4.)),
            Box::new(row! { self.preset_button("Default",Preset::Default),self.preset_button("Room",Preset::Room),self.preset_button("Hall",Preset::Hall),self.preset_button("Aux send",Preset::AuxSend) }.spacing(8.)),
            Box::new(Rectangle::new().fill(LINE).frame(500.,1.)),
            Box::new(caption("SPACE").font_size(10.)),
            Box::new(row! { self.control(2,"Room size","Small → spacious",76.),self.control(3,"Damping","Bright → soft",76.),self.control(4,"Width","Mono → stereo",76.) }.spacing(18.)),
            Box::new(Rectangle::new().fill(LINE).frame(500.,1.)),
            Box::new(caption("OUTPUT MIX").font_size(10.)),
            Box::new(row! { self.control(0,"Wet","Reverb level",52.),self.control(1,"Dry","Original level",52.) }.spacing(18.)),
            Box::new(caption("Apply to hear changes · Aux send uses Wet 1 / Dry 0").font_size(10.)),
            Box::new(Text::from_state(self.error.clone()).font_size(11.).color(GOLD).frame_width(500.)),
            Box::new(row! { button("Cancel").on_click(move||cancel()),button("Apply").background_color(ACCENT).text_color(BG).on_click(move||apply()) }.spacing(8.)),
        ])).alignment(Alignment::TopLeading).spacing(8.).padding(24.).frame_width(WIDTH))
    }
}
impl View for FreeverbEditor {
    fn create_element(&self) -> Box<dyn scarlet_ui::Element> {
        Box::new(scarlet_ui::ComponentElement::new_with_builder(
            self.clone(),
            |s| Box::new(s.body()),
        ))
    }
    fn listenables(&self) -> Vec<&dyn Listenable> {
        self.fields
            .iter()
            .map(|s| s as &dyn Listenable)
            .chain(std::iter::once(&self.error as &dyn Listenable))
            .collect()
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn presets_preserve_parameter_order_and_validate_typed_drafts() {
        let editor = FreeverbEditor::new(DEFAULTS).unwrap();
        assert_eq!(editor.values().unwrap(), DEFAULTS);
        editor.preset(Preset::AuxSend);
        assert_eq!(editor.values().unwrap(), [1., 0., 0.75, 0.45, 1.]);
        editor.fields()[2].set("0.12345678901234567".into());
        assert_eq!(editor.values().unwrap()[2], 0.12);
        editor.normalize_drafts().unwrap();
        assert_eq!(editor.fields()[2].get(), "0.12");
        assert_eq!(format_value(0.2), "0.20");
        assert_eq!(format_value(0.25), "0.25");
        assert_eq!(format_value(1.), "1.00");
        assert_eq!(format_value(0.), "0.00");
        let rounded = FreeverbEditor::new([0.123456789; 5]).unwrap();
        assert!(rounded.fields().iter().all(|f| f.get() == "0.12"));
        editor.fields()[0].set("NaN".into());
        assert!(editor.values().is_err());
        editor.preset(Preset::Default);
        assert_eq!(editor.values().unwrap(), DEFAULTS);
        assert!(FreeverbEditor::new([f64::INFINITY; 5]).is_err());
    }
}
