//! Non-destructive timeline range removal and offline sample processing.
//! Original sources remain shared by Undo and unaffected regions.
use crate::{Clip, Result};
use std::{
    ops::Range,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

#[derive(Clone)]
pub enum AudioEdit {
    Silence(Range<usize>),
    Reverse(Range<usize>),
}
impl AudioEdit {
    pub fn range(&self) -> Range<usize> {
        match self {
            Self::Silence(r) | Self::Reverse(r) => r.clone(),
        }
    }
}
fn valid(clip: &Clip) -> bool {
    clip.frames > 0
        && matches!(clip.source_channels, 1 | 2)
        && clip.edit.valid(clip.frames)
        && clip
            .source_offset
            .checked_add(clip.frames)
            .is_some_and(|end| end <= clip.samples.len())
        && clip.start.checked_add(clip.frames as u64).is_some()
}
fn valid_range(clip: &Clip, range: &Range<usize>) -> Result<()> {
    if !valid(clip) {
        return Err("Invalid audio region".into());
    }
    if range.start > range.end || range.end > clip.frames {
        return Err("Audio edit range must be inside the region".into());
    }
    Ok(())
}
fn raw(clip: &Clip, at: usize) -> [f32; 2] {
    let at = if clip.edit.reversed {
        clip.frames - 1 - at
    } else {
        at
    };
    clip.samples[clip.source_offset + at]
}

/// Remove a range without closing time. Remaining segments retain their exact
/// timeline positions, shared source, direction, gain and inherited envelope.
/// A full-region range removes it; an empty range leaves it untouched.
pub fn remove_region_range(original: &Clip, range: Range<usize>) -> Result<Vec<Clip>> {
    valid_range(original, &range)?;
    if range.is_empty() {
        return Ok(vec![original.clone()]);
    }
    let mut remaining = Vec::with_capacity(2);
    if range.start > 0 {
        let mut left = original.clone();
        left.trim_relative(0, range.start)?;
        remaining.push(left);
    }
    if range.end < original.frames {
        let mut right = original.clone();
        right.trim_relative(range.end, original.frames)?;
        remaining.push(right);
    }
    Ok(remaining)
}

/// Materialize this region on a worker. Silence/reverse preserve duration and
/// timeline position. Region gain and edge fades stay separate from source.
pub fn edit_region_audio(original: &Clip, edit: &AudioEdit, cancel: &AtomicBool) -> Result<Clip> {
    if cancel.load(Ordering::Acquire) {
        return Err("Region processing cancelled".into());
    }
    let range = edit.range();
    valid_range(original, &range)?;
    let frames = original.frames;
    if frames > 512 * 1024 * 1024 / std::mem::size_of::<[f32; 2]>() {
        return Err("Audio edit exceeds the 512 MiB buffer limit".into());
    }
    let mut samples = Vec::new();
    samples
        .try_reserve_exact(frames)
        .map_err(|_| "Could not allocate edited audio")?;
    for at in 0..frames {
        if at % 1024 == 0 && cancel.load(Ordering::Acquire) {
            return Err("Region processing cancelled".into());
        }
        let sample = if range.contains(&at) {
            match edit {
                AudioEdit::Silence(_) => [0.; 2],
                AudioEdit::Reverse(_) => raw(original, range.end - 1 - (at - range.start)),
            }
        } else {
            raw(original, at)
        };
        if sample.iter().any(|v| !v.is_finite()) {
            return Err("Cannot edit non-finite audio".into());
        }
        samples.push(sample);
    }
    if cancel.load(Ordering::Acquire) {
        return Err("Region processing cancelled".into());
    }
    if original.edit.reversed {
        samples.reverse();
    }
    let mut result = original.clone();
    result.source_offset = 0;
    result.samples = Arc::new(samples);
    Ok(result)
}
