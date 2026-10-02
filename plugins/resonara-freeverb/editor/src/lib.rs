#![cfg_attr(not(feature = "std"), no_std)]
//! Plugin-owned ScarletUI controls and CLAP embedding adapters.
//! The host owns the containing window, transport, state and Undo.
//! This crate creates no event loop and never runs on the audio thread.
use scarlet_ui::{hstack as row, prelude::*, vstack};
extern crate alloc;
use alloc::{boxed::Box, format, rc::Rc, string::String, vec, vec::Vec};
use core::any::Any;
#[cfg(target_os = "macos")]
pub mod cocoa;
mod knob;
#[cfg(target_os = "scarlet")]
pub mod sws;
mod ui;
use knob::RotaryKnob;
use ui::*;

pub const PLUGIN_ID: &str = "org.resonara.freeverb";
pub const PARAMETER_IDS: [u32; 5] = [0, 1, 2, 3, 4];
pub const DEFAULTS: [f64; 5] = [0.3, 1., 0.5, 0.5, 1.];
pub const WIDTH: f32 = 548.;

fn round_value(value: f64) -> f64 {
    libm::round(value * 100.) / 100. + 0.0
}
fn format_value(value: f64) -> String {
    format!("{:.2}", round_value(value))
}

/// Distinguish rounded UI text from actual host values. Passive clicks must not
/// quantize untouched parameters, and successive edits retain earlier requests.
#[cfg(any(target_os = "macos", target_os = "scarlet", test))]
struct LiveParameters {
    observed: [f64; 5],
    desired: [f64; 5],
    shown: [f64; 5],
}
#[cfg(any(target_os = "macos", target_os = "scarlet", test))]
impl LiveParameters {
    fn new(values: [f64; 5]) -> Self {
        Self {
            observed: values,
            desired: values,
            shown: values.map(round_value),
        }
    }
    fn observe(&mut self, values: [f64; 5]) -> bool {
        if self.observed == values {
            return false;
        }
        *self = Self::new(values);
        true
    }
    fn changes(&mut self, values: [f64; 5]) -> Option<[f64; 5]> {
        if self.shown == values {
            return None;
        }
        for i in 0..5 {
            if self.shown[i] != values[i] {
                self.desired[i] = values[i];
            }
        }
        self.shown = values;
        Some(self.desired)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
    preset_index: State<usize>,
    live: bool,
    apply: Rc<dyn Fn()>,
    cancel: Rc<dyn Fn()>,
}
impl FreeverbEditor {
    pub fn new(values: [f64; 5]) -> core::result::Result<Self, &'static str> {
        if values
            .iter()
            .any(|v| !v.is_finite() || !(0. ..=1.).contains(v))
        {
            return Err("Freeverb values must be finite and between 0 and 1");
        }
        let fields = core::array::from_fn(|i| {
            State::new(
                scarlet_ui::state::generate_state_id(),
                format_value(values[i]),
            )
        });
        let knobs = core::array::from_fn(|i| {
            RotaryKnob::parameter(fields[i].clone(), 0., 1., values[i], false).unwrap()
        });
        Ok(Self {
            fields,
            knobs,
            error: State::new(scarlet_ui::state::generate_state_id(), String::new()),
            preset_index: State::new(scarlet_ui::state::generate_state_id(), 0),
            live: false,
            apply: Rc::new(|| {}),
            cancel: Rc::new(|| {}),
        })
    }
    pub fn fields(&self) -> [State<String>; 5] {
        self.fields.clone()
    }
    pub fn values(&self) -> core::result::Result<[f64; 5], &'static str> {
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
    pub fn normalize_drafts(&self) -> core::result::Result<(), &'static str> {
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
        let apply: Rc<dyn Fn()> = if self.live {
            let editor = self.clone();
            Rc::new(move || {
                let _ = editor.normalize_drafts();
            })
        } else {
            self.apply.clone()
        };
        AnyView::new(vstack! {
            label(name).font_size(12.),
            self.knobs[i].clone().frame(diameter,diameter),
            field(self.fields[i].clone()).font_size(11.).on_submit(move||apply()).frame(100.,28.).on_key(|event|matches!(event,scarlet_ui::event::KeyEvent::Pressed{keycode:scarlet_ui::event::KeyCode::Char(_),..})),
            caption(hint).font_size(10.)
        }.alignment(Alignment::Center).spacing(6.).frame_width(144.))
    }
    pub fn live(mut self) -> Self {
        self.live = true;
        self
    }
    pub fn set_values(&self, values: [f64; 5]) {
        for (field, value) in self.fields.iter().zip(values) {
            let value = format_value(value);
            if field.get() != value {
                field.set(value);
            }
        }
    }
    pub fn current_preset(&self) -> Option<Preset> {
        let values = self.values().ok()?;
        [Preset::Default, Preset::Room, Preset::Hall, Preset::AuxSend]
            .into_iter()
            .find(|p| p.values() == values)
    }
    fn preset_selector(&self) -> AnyView {
        let presets = [Preset::Default, Preset::Room, Preset::Hall, Preset::AuxSend];
        let index = self
            .current_preset()
            .and_then(|p| presets.iter().position(|v| *v == p))
            .unwrap_or(4);
        if self.preset_index.get() != index {
            self.preset_index.set(index);
        }
        let editor = self.clone();
        AnyView::new(row! {
            caption("PRESET").font_size(10.),
            Select::new(vec!["Default".into(), "Room".into(), "Hall".into(), "Aux send".into(), "Custom".into()], self.preset_index.clone())
                .width(240.).row_height(30.).on_change(move |i| { if let Some(p) = presets.get(i) { editor.preset(*p); } })
        }.spacing(12.))
    }
    fn body(&self) -> AnyView {
        let apply = self.apply.clone();
        let cancel = self.cancel.clone();
        AnyView::new(VStack::new(Children(vec![
            Box::new(vstack! { caption("RESONARA · STEREO REVERB").font_size(10.).color(ACCENT),label("Freeverb").font_size(28.),caption("Space, tone and stereo ambience").font_size(12.) }.alignment(Alignment::TopLeading).spacing(4.)),
            Box::new(self.preset_selector()),
            Box::new(Rectangle::new().fill(LINE).frame(500.,1.)),
            Box::new(caption("SPACE").font_size(10.)),
            Box::new(row! { self.control(2,"Room size","Small → spacious",76.),self.control(3,"Damping","Bright → soft",76.),self.control(4,"Width","Mono → stereo",76.) }.spacing(18.)),
            Box::new(Rectangle::new().fill(LINE).frame(500.,1.)),
            Box::new(caption("OUTPUT MIX").font_size(10.)),
            Box::new(row! { self.control(0,"Wet","Reverb level",52.),self.control(1,"Dry","Original level",52.) }.spacing(18.)),
            Box::new(caption(if self.live { "Changes apply immediately · Aux send uses Wet 1 / Dry 0" } else { "Apply to hear changes · Aux send uses Wet 1 / Dry 0" }).font_size(10.)),
            Box::new(Text::from_state(self.error.clone()).font_size(11.).color(GOLD).frame_width(500.)),
            Box::new(if self.live { AnyView::new(caption("")) } else { AnyView::new(row! { button("Cancel").on_click(move||cancel()),button("Apply").background_color(ACCENT).text_color(BG).on_click(move||apply()) }.spacing(8.)) }),
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
            .chain(core::iter::once(&self.error as &dyn Listenable))
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
    fn passive_input_preserves_unrounded_values_and_successive_edits() {
        let actual = [0.303, 0.997, 0.503, 0.504, 0.995];
        let mut live = LiveParameters::new(actual);
        let mut ui = actual.map(round_value);
        assert_eq!(live.changes(ui), None);
        ui[0] = 0.20;
        let mut expected = actual;
        expected[0] = 0.20;
        assert_eq!(live.changes(ui), Some(expected));
        ui[1] = 0.90;
        expected[1] = 0.90;
        assert_eq!(live.changes(ui), Some(expected));
        assert!(live.observe(expected));
        assert_eq!(live.changes(ui), None);
        assert!(!live.observe(expected));
    }

    #[test]
    fn preset_selection_tracks_external_values_and_custom_edits() {
        let editor = FreeverbEditor::new(DEFAULTS).unwrap().live();
        let fields = editor.fields();
        assert_eq!(editor.current_preset(), Some(Preset::Default));
        editor.set_values(Preset::Hall.values());
        assert_eq!(editor.current_preset(), Some(Preset::Hall));
        assert_eq!(
            fields[2].get(),
            "0.82",
            "synchronization keeps existing field states"
        );
        fields[2].set("0.80".into());
        assert_eq!(editor.current_preset(), None);
        editor.preset(Preset::AuxSend);
        assert_eq!(editor.current_preset(), Some(Preset::AuxSend));
        assert_eq!(fields[0].get(), "1.00");
        assert_eq!(fields[1].get(), "0.00");
    }

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
