//! Nondestructive region processing. Source buffers always remain immutable.
use crate::{Clip, Result};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ClipEdit {
    /// Region gain, before the track's inserts and mixer.
    pub gain_db: f32,
    /// Linear fade lengths, in project-rate frames. Zero disables a fade.
    pub fade_in: usize,
    pub fade_out: usize,
    pub reversed: bool,
    // A trim/split retains the original envelope instead of inventing new
    // fades at each cut. New fade edits deliberately reanchor to current edges.
    #[serde(skip_serializing_if = "Option::is_none")]
    envelope_frames: Option<usize>,
    #[serde(skip_serializing_if = "is_zero")]
    envelope_offset: usize,
}

fn is_zero(value: &usize) -> bool {
    *value == 0
}

impl ClipEdit {
    /// A trim or split retained an envelope anchored to the original region.
    /// Editing edge fades should first explicitly reset that inherited envelope.
    pub fn has_inherited_fades(self) -> bool {
        self.envelope_frames.is_some() && (self.fade_in != 0 || self.fade_out != 0)
    }

    pub(crate) fn valid(self, frames: usize) -> bool {
        let envelope_frames = self.envelope_frames.unwrap_or(frames);
        self.gain_db.is_finite()
            && (-60.0..=24.0).contains(&self.gain_db)
            && self.fade_in <= envelope_frames
            && self.fade_out <= envelope_frames
            && self
                .envelope_offset
                .checked_add(frames)
                .is_some_and(|end| end <= envelope_frames)
    }

    pub fn gain_linear(self) -> f32 {
        if self.gain_db == 0.0 {
            1.0
        } else {
            10.0f32.powf(self.gain_db / 20.0)
        }
    }

    fn fade_gain(self, relative: f64, frames: usize) -> f32 {
        let position = self.envelope_offset as f64 + relative;
        let mut gain = 1.0;
        if self.fade_in > 0 {
            gain *=
                (position / self.fade_in.saturating_sub(1).max(1) as f64).clamp(0.0, 1.0) as f32;
        }
        if self.fade_out > 0 {
            let remaining =
                self.envelope_frames.unwrap_or(frames).saturating_sub(1) as f64 - position;
            gain *=
                (remaining / self.fade_out.saturating_sub(1).max(1) as f64).clamp(0.0, 1.0) as f32;
        }
        gain
    }

    fn retain_interval(&mut self, original_frames: usize, from: usize) {
        if self.fade_in != 0 || self.fade_out != 0 {
            self.envelope_frames.get_or_insert(original_frames);
            self.envelope_offset += from;
        }
    }
}

impl Clip {
    /// Map a half-open interval from region time to the immutable source.
    /// The returned bounds are ascending even when playback is reversed.
    pub fn source_range(&self, from: usize, to: usize) -> std::ops::Range<usize> {
        let from = from.min(self.frames);
        let to = to.max(from).min(self.frames);
        if self.edit.reversed {
            self.source_offset + self.frames - to..self.source_offset + self.frames - from
        } else {
            self.source_offset + from..self.source_offset + to
        }
    }

    /// Region gain and fade envelope at a project-frame position.
    pub fn amplitude_at(&self, relative: f64) -> f32 {
        if !relative.is_finite() || relative < 0.0 || relative >= self.frames as f64 {
            return 0.0;
        }
        self.edit.gain_linear() * self.edit.fade_gain(relative, self.frames)
    }

    /// Keep a region-relative half-open interval. This moves its timeline start
    /// by `from`, preserves the original fade envelope, and shares the source.
    pub fn trim_relative(&mut self, from: usize, to: usize) -> Result<()> {
        if from >= to || to > self.frames {
            return Err("Trim range must be inside the region".into());
        }
        let start = self
            .start
            .checked_add(from as u64)
            .ok_or("Region start is too large")?;
        let source_offset = self
            .source_offset
            .checked_add(if self.edit.reversed {
                self.frames - to
            } else {
                from
            })
            .ok_or("Region source offset is too large")?;
        self.edit.retain_interval(self.frames, from);
        self.start = start;
        self.source_offset = source_offset;
        self.frames = to - from;
        Ok(())
    }

    /// Resize either edge, allowing the region to expose more of its source.
    /// Negative `from` extends the left edge; `to > frames` extends the right.
    /// Restoring a previously trimmed range restores its original fade envelope.
    /// Extending beyond that envelope places its fades at the new region edges.
    pub fn resize_relative(&mut self, from: i64, to: i64) -> Result<()> {
        if from >= to || !self.edit.valid(self.frames) {
            return Err("Invalid region resize range".into());
        }
        let frames = to as i128 - from as i128;
        let start = self.start as i128 + from as i128;
        let source_offset = self.source_offset as i128
            + if self.edit.reversed {
                self.frames as i128 - to as i128
            } else {
                from as i128
            };
        if start < 0
            || start + frames > u64::MAX as i128
            || source_offset < 0
            || source_offset + frames > self.samples.len() as i128
        {
            return Err("Region resize exceeds the source or timeline bounds".into());
        }
        let frames = frames as usize;
        if self.edit.fade_in != 0 || self.edit.fade_out != 0 {
            let envelope_frames = self.edit.envelope_frames.unwrap_or(self.frames);
            let envelope_offset = self.edit.envelope_offset as i128 + from as i128;
            if envelope_offset >= 0 && envelope_offset + frames as i128 <= envelope_frames as i128 {
                self.edit.envelope_frames = Some(envelope_frames);
                self.edit.envelope_offset = envelope_offset as usize;
            } else {
                self.edit.envelope_frames = None;
                self.edit.envelope_offset = 0;
                self.edit.fade_in = self.edit.fade_in.min(frames);
                self.edit.fade_out = self.edit.fade_out.min(frames);
            }
        } else {
            self.edit.envelope_frames = None;
            self.edit.envelope_offset = 0;
        }
        self.start = start as u64;
        self.source_offset = source_offset as usize;
        self.frames = frames;
        Ok(())
    }

    /// Split in region-relative project frames. Self becomes the left piece;
    /// the returned piece begins at the cut and shares the immutable source.
    pub fn split_relative(&mut self, at: usize) -> Result<Self> {
        if at == 0 || at >= self.frames {
            return Err("Split position must be inside the region".into());
        }
        let mut right = self.clone();
        right.trim_relative(at, self.frames)?;
        self.trim_relative(0, at)?;
        Ok(right)
    }

    /// Set fades relative to the current region edges, including after a trim.
    pub fn set_fades(&mut self, fade_in: usize, fade_out: usize) -> Result<()> {
        if fade_in > self.frames || fade_out > self.frames {
            return Err("Fade length cannot exceed the region".into());
        }
        self.edit.fade_in = fade_in;
        self.edit.fade_out = fade_out;
        self.edit.envelope_frames = None;
        self.edit.envelope_offset = 0;
        Ok(())
    }

    /// Reverse the selected source interval; fades remain in timeline order.
    pub fn set_reversed(&mut self, reversed: bool) {
        self.edit.reversed = reversed;
    }

    pub fn set_gain_db(&mut self, gain_db: f32) -> Result<()> {
        if !gain_db.is_finite() || !(-60.0..=24.0).contains(&gain_db) {
            return Err("Region gain must be between -60 and +24 dB".into());
        }
        self.edit.gain_db = gain_db;
        Ok(())
    }

    /// Peak-normalize the audible source and fades without changing samples.
    /// Call on a control/worker thread; scans the entire selected region.
    pub fn normalize(&mut self, target_db: f32) -> Result<()> {
        if !target_db.is_finite() || !(-60.0..=0.0).contains(&target_db) {
            return Err("Normalization target must be between -60 and 0 dBFS".into());
        }
        if !self.edit.valid(self.frames)
            || self
                .source_offset
                .checked_add(self.frames)
                .is_none_or(|end| end > self.samples.len())
        {
            return Err("Invalid region".into());
        }
        let mut peak = 0.0f32;
        for frame in 0..self.frames {
            let sample = self.sample_with_gain(frame as f64, self.frames.saturating_sub(1), 1.0);
            if sample.iter().any(|value| !value.is_finite()) {
                return Err("Cannot normalize non-finite audio".into());
            }
            peak = peak.max(sample[0].abs()).max(sample[1].abs());
        }
        if !peak.is_finite() || peak <= f32::MIN_POSITIVE {
            return Err("Cannot normalize a silent region".into());
        }
        self.set_gain_db(target_db - 20.0 * peak.log10())
    }

    /// Process a region-relative position for waveform previews. Rendering uses
    /// the same operation with a prepared gain and continuation lookahead.
    pub fn sample_at(&self, relative_frame: f64) -> [f32; 2] {
        self.sample_with_gain(
            relative_frame,
            self.frames.saturating_sub(1),
            self.edit.gain_linear(),
        )
    }

    pub(crate) fn sample_with_gain(
        &self,
        relative: f64,
        interpolation_end: usize,
        gain: f32,
    ) -> [f32; 2] {
        if !relative.is_finite() || relative < 0.0 || relative >= self.frames as f64 {
            return [0.0; 2];
        }
        let a = relative as usize;
        let b = (a + 1).min(interpolation_end);
        let index = |frame| {
            if self.edit.reversed {
                self.source_offset + self.frames - 1 - frame
            } else {
                self.source_offset + frame
            }
        };
        let a = self.samples[index(a)];
        let b = self.samples[index(b)];
        let f = relative.fract() as f32;
        let mut output = [a[0] * (1.0 - f) + b[0] * f, a[1] * (1.0 - f) + b[1] * f];
        // Keep the unedited path bit-identical, including signed zero.
        let gain = gain * self.edit.fade_gain(relative, self.frames);
        if gain != 1.0 {
            output[0] *= gain;
            output[1] *= gain;
        }
        output
    }

    pub(crate) fn continues_into(&self, next: &Self) -> bool {
        if self.frames == 0
            || next.frames == 0
            || self.start.checked_add(self.frames as u64) != Some(next.start)
            || self.edit.gain_db != next.edit.gain_db
            || self.edit.reversed != next.edit.reversed
            || self.edit.fade_in != next.edit.fade_in
            || self.edit.fade_out != next.edit.fade_out
        {
            return false;
        }
        let adjacent_source = if self.edit.reversed {
            next.source_offset.checked_add(next.frames) == Some(self.source_offset)
        } else {
            self.source_offset.checked_add(self.frames) == Some(next.source_offset)
        };
        adjacent_source
            && ((self.edit.fade_in == 0 && self.edit.fade_out == 0)
                || (self.edit.envelope_frames.unwrap_or(self.frames)
                    == next.edit.envelope_frames.unwrap_or(next.frames)
                    && self.edit.envelope_offset.checked_add(self.frames)
                        == Some(next.edit.envelope_offset)))
            && (std::sync::Arc::ptr_eq(&self.samples, &next.samples)
                || self.samples.as_ref() == next.samples.as_ref())
    }
}
