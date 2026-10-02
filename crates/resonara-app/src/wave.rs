//! Retained SGFX waveforms. Source peaks are cached once, never rescanned by meters.
use crate::ui;
use resonara_core::Track;
use scarlet_ui::prelude::*;
use std::{collections::HashMap, sync::Arc};

const BIN_FRAMES: usize = 128;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Envelope {
    pub min: [f32; 2],
    pub max: [f32; 2],
}
impl Envelope {
    fn empty() -> Self {
        Self {
            min: [f32::INFINITY; 2],
            max: [f32::NEG_INFINITY; 2],
        }
    }
    fn sample(&mut self, sample: [f32; 2]) {
        for channel in 0..2 {
            self.min[channel] = self.min[channel].min(sample[channel]);
            self.max[channel] = self.max[channel].max(sample[channel]);
        }
    }
    fn merge(&mut self, other: Self) {
        for channel in 0..2 {
            self.min[channel] = self.min[channel].min(other.min[channel]);
            self.max[channel] = self.max[channel].max(other.max[channel]);
        }
    }
}
struct SourcePeaks {
    samples: Arc<Vec<[f32; 2]>>,
    bins: Vec<Envelope>,
}
impl SourcePeaks {
    fn envelope(&self, from: usize, to: usize) -> Envelope {
        let from = from.min(self.samples.len());
        let to = to.min(self.samples.len()).max(from);
        if from == to {
            return Envelope {
                min: [0.; 2],
                max: [0.; 2],
            };
        }
        let mut envelope = Envelope::empty();
        let first_full = from.div_ceil(BIN_FRAMES);
        let last_full = to / BIN_FRAMES;
        if first_full >= last_full {
            for sample in &self.samples[from..to] {
                envelope.sample(*sample);
            }
        } else {
            for sample in &self.samples[from..first_full * BIN_FRAMES] {
                envelope.sample(*sample);
            }
            for bin in &self.bins[first_full..last_full] {
                envelope.merge(*bin);
            }
            for sample in &self.samples[last_full * BIN_FRAMES..to] {
                envelope.sample(*sample);
            }
        }
        envelope
    }
}
#[derive(Default)]
pub struct Peaks {
    sources: HashMap<usize, SourcePeaks>,
}
impl Peaks {
    fn source(&mut self, samples: &Arc<Vec<[f32; 2]>>) -> &SourcePeaks {
        let key = Arc::as_ptr(samples) as usize;
        self.sources.entry(key).or_insert_with(|| {
            let bins = samples
                .chunks(BIN_FRAMES)
                .map(|chunk| {
                    let mut envelope = Envelope::empty();
                    for sample in chunk {
                        envelope.sample(*sample);
                    }
                    envelope
                })
                .collect();
            SourcePeaks {
                samples: samples.clone(),
                bins,
            }
        })
    }
    #[cfg(test)]
    pub(crate) fn envelope(
        &mut self,
        samples: &Arc<Vec<[f32; 2]>>,
        from: usize,
        to: usize,
    ) -> Envelope {
        self.source(samples).envelope(from, to)
    }
    pub fn retain(&mut self, tracks: &[Track]) {
        self.sources.retain(|key, _| {
            tracks.iter().any(|t| {
                t.clips
                    .iter()
                    .any(|c| Arc::as_ptr(&c.samples) as usize == *key)
            })
        });
    }

    /// Peak-cache lookup in audible clip order. Edits share the source cache;
    /// only the tiny visible sample window is read directly at sample zoom.
    pub(crate) fn clip_envelope(
        &mut self,
        clip: &resonara_core::Clip,
        from: usize,
        to: usize,
    ) -> Envelope {
        let from = from.min(clip.frames);
        let to = to.min(clip.frames).max(from);
        if to <= from {
            return Envelope {
                min: [0.; 2],
                max: [0.; 2],
            };
        }
        if to - from <= 4 {
            let mut result = Envelope::empty();
            for frame in from..to {
                result.sample(clip.sample_at(frame as f64));
            }
            return result;
        }
        let range = clip.source_range(from, to);
        let mut result = self.source(&clip.samples).envelope(range.start, range.end);
        // A column spans at most two display pixels. Evaluate the envelope at
        // its center so gain/fade previews match the audible edited region.
        let gain = clip.amplitude_at((from as f64 + to as f64 - 1.) * 0.5);
        for channel in 0..2 {
            result.min[channel] *= gain;
            result.max[channel] *= gain;
        }
        result
    }
}
#[derive(Clone, Copy, Debug)]
pub(crate) struct Lane {
    pub channel: usize,
    pub top: f32,
    pub bottom: f32,
    pub center: f32,
    pub amplitude: f32,
}
pub(crate) fn lanes(source_channels: u16, height: f32) -> Vec<Lane> {
    let top = 27f32.min(height * 0.45);
    let bottom = (height - 10.).max(top + 1.);
    let count = if source_channels == 1 { 1 } else { 2 };
    let gap = if count == 2 { 3. } else { 0. };
    let lane_height = ((bottom - top - gap) / count as f32).max(1.);
    (0..count)
        .map(|channel| {
            let y = top + channel as f32 * (lane_height + gap);
            Lane {
                channel,
                top: y,
                bottom: y + lane_height,
                center: y + lane_height / 2.,
                amplitude: (lane_height / 2. - 1.).max(0.5),
            }
        })
        .collect()
}
#[derive(Clone, Copy, Debug)]
pub(crate) struct Projection {
    pub left: f32,
    pub right: f32,
    pub left_edge_visible: bool,
    pub right_edge_visible: bool,
    source_x: f64,
    source_right: f64,
}
impl Projection {
    #[cfg(test)]
    pub(crate) fn source_range(
        self,
        clip: &resonara_core::Clip,
        left: f32,
        right: f32,
    ) -> std::ops::Range<usize> {
        let range = self.relative_range(clip, left, right);
        clip.source_range(range.start, range.end)
    }
    pub(crate) fn relative_range(
        self,
        clip: &resonara_core::Clip,
        left: f32,
        right: f32,
    ) -> std::ops::Range<usize> {
        let width = self.source_right - self.source_x;
        let from = (((left as f64 - self.source_x) / width).clamp(0., 1.) * clip.frames as f64)
            .floor() as usize;
        let to = (((right as f64 - self.source_x) / width).clamp(0., 1.) * clip.frames as f64)
            .ceil() as usize;
        from.min(clip.frames)..to.min(clip.frames)
    }
}
pub(crate) fn project_clip(
    clip: &resonara_core::Clip,
    width: f32,
    offset: f64,
    span: f64,
    rate: u32,
) -> Option<Projection> {
    if width <= 0. || span <= 0. || rate == 0 || clip.frames == 0 {
        return None;
    }
    let start = clip.start as f64 / rate as f64;
    let end = (clip.start + clip.frames as u64) as f64 / rate as f64;
    if end <= offset || start >= offset + span {
        return None;
    }
    let x = (start - offset) / span * width as f64;
    let right = (end - offset) / span * width as f64;
    let projection = Projection {
        left: x.max(0.) as f32,
        right: right.min(width as f64) as f32,
        left_edge_visible: x >= 0.,
        right_edge_visible: right <= width as f64,
        source_x: x,
        source_right: right,
    };
    (projection.right > projection.left).then_some(projection)
}
pub fn rectangle(
    v: &mut Vec<SgfxCanvasVertex>,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    c: Color,
    width: f32,
    height: f32,
) {
    let right = (x + w).min(width);
    let bottom = (y + h).min(height);
    let x = x.max(0.);
    let y = y.max(0.);
    let w = right - x;
    let h = bottom - y;
    if w <= 0. || h <= 0. || width <= 0. || height <= 0. {
        return;
    }
    let a = 2. * x / width - 1.;
    let b = 1. - 2. * y / height;
    let d = 2. * (x + w) / width - 1.;
    let e = 1. - 2. * (y + h) / height;
    let points = [[a, b], [d, b], [d, e], [a, e]];
    for i in [0, 1, 2, 0, 2, 3] {
        v.push(SgfxCanvasVertex::new(
            [points[i][0], points[i][1], 0., 1.],
            [c.r, c.g, c.b, c.a],
        ));
    }
}
pub const ID: [f32; 16] = [
    1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.,
];
pub fn tick_step(span: f64) -> f64 {
    let ideal = span / 8.;
    let mag = 10f64.powf(ideal.max(0.001).log10().floor());
    [1., 2., 5., 10.]
        .into_iter()
        .map(|v| v * mag)
        .find(|v| *v >= ideal)
        .unwrap_or(mag * 10.)
}
pub fn base(
    track: &Track,
    index: usize,
    selected: Option<usize>,
    width: f32,
    height: f32,
    offset: f64,
    span: f64,
    grid_step: f64,
    rate: u32,
    peaks: &mut Peaks,
    handle: SgfxMeshHandle,
    revision: u64,
) -> Arc<SgfxMesh> {
    SgfxMesh::with_handle(
        handle,
        revision,
        vertices(
            track, index, selected, width, height, offset, span, grid_step, rate, peaks,
        ),
    )
}
pub(crate) fn vertices(
    track: &Track,
    index: usize,
    selected: Option<usize>,
    width: f32,
    height: f32,
    offset: f64,
    span: f64,
    grid_step: f64,
    rate: u32,
    peaks: &mut Peaks,
) -> Vec<SgfxCanvasVertex> {
    let mut v = Vec::new();
    let h = height;
    let mut rect = |x, y, w, hh, c| rectangle(&mut v, x, y, w, hh, c, width, h);
    let step = grid_step;
    let first = (offset / step).floor() as i64;
    for i in first..=first + 12 {
        let x = ((i as f64 * step - offset) / span) as f32 * width;
        if x >= 0. && x <= width {
            rect(x, 0., 1., h, ui::LINE);
        }
    }
    rect(0., h - 1., width, 1., ui::LINE);
    let col = ui::color(index);
    let mute = track.mute;
    for (ci, c) in track.clips.iter().enumerate() {
        let Some(projection) = project_clip(c, width, offset, span, rate) else {
            continue;
        };
        let a = projection.left;
        let b = projection.right;
        let active = selected == Some(ci);
        let tint = if mute { ui::MUTED } else { col };
        let bg = Color::rgb(tint.r * 0.29, tint.g * 0.29, tint.b * 0.29);
        let clip_bottom = h - 7.;
        rect(a, 7., b - a, clip_bottom - 7., bg);
        rect(
            a,
            7.,
            b - a,
            18.,
            if active {
                tint
            } else {
                Color::rgb(tint.r * 0.60, tint.g * 0.60, tint.b * 0.60)
            },
        );
        if active {
            let edge = (b - a).min(2.);
            if projection.left_edge_visible {
                rect(a, 7., edge, clip_bottom - 7., tint);
            }
            if projection.right_edge_visible {
                rect(b - edge, 7., edge, clip_bottom - 7., tint);
            }
            rect(a, clip_bottom - 2., b - a, 2., tint);
        }
        let layout = lanes(c.source_channels, h);
        if layout.len() == 2 {
            let separator = (layout[0].bottom + layout[1].top) / 2.;
            rect(
                a,
                separator,
                b - a,
                1.,
                Color::rgb(tint.r * 0.40, tint.g * 0.40, tint.b * 0.40),
            );
        }
        for lane in &layout {
            rect(
                a,
                lane.center,
                b - a,
                0.7,
                Color::rgb(tint.r * 0.42, tint.g * 0.42, tint.b * 0.42),
            );
        }
        let count = ((b - a) / 2.).ceil().max(1.) as usize;
        for bin in 0..count {
            let px = a + bin as f32 * 2.;
            let next = (px + 2.).min(b);
            let range = projection.relative_range(c, px, next);
            let envelope = peaks.clip_envelope(c, range.start, range.end);
            for lane in &layout {
                let top = lane.center - envelope.max[lane.channel].clamp(-1., 1.) * lane.amplitude;
                let bottom =
                    lane.center - envelope.min[lane.channel].clamp(-1., 1.) * lane.amplitude;
                // A true one-sample/zero envelope is a thin line at its actual amplitude.
                rect(px, top, (next - px).min(1.3), (bottom - top).max(0.7), tint);
            }
        }
        if active && b - a >= 12. {
            let middle = (layout[0].top + layout.last().unwrap().bottom) / 2.;
            if projection.left_edge_visible {
                rect(a + 3., middle - 8., 2., 16., ui::TEXT);
            }
            if projection.right_edge_visible {
                rect(b - 5., middle - 8., 2., 16., ui::TEXT);
            }
        }
    }
    v
}
pub fn frame(
    mesh: Arc<SgfxMesh>,
    width: f32,
    height: f32,
    offset: f64,
    span: f64,
    playhead: f64,
    selected: bool,
    revision: u64,
    playhead_handle: SgfxMeshHandle,
) -> Arc<SgfxCanvasFrame> {
    let mut v = Vec::new();
    let x = ((playhead - offset) / span) as f32 * width;
    if x >= 0. && x <= width {
        rectangle(&mut v, x, 0., 1.5, height, ui::TEXT, width, height);
    }
    let mut frame = SgfxCanvasFrame::new(
        revision,
        if selected {
            Color::rgb(0.105, 0.135, 0.156)
        } else {
            ui::BG
        },
    )
    .reference_aspect(width / height)
    .draw(SgfxCanvasDraw::new(mesh, ID));
    // SGFX rejects empty draw meshes. An offscreen playhead has no draw.
    if !v.is_empty() {
        frame = frame.draw(SgfxCanvasDraw::new(
            SgfxMesh::with_handle(playhead_handle, revision, v),
            ID,
        ));
    }
    Arc::new(frame)
}
