//! Offline region jobs retain immutable input, and commit exactly one undoable
//! edit on the UI thread. Neither the audio callback nor the UI scans audio.
use super::*;
use std::sync::atomic::AtomicBool;

#[derive(Clone, Copy)]
enum RegionOperation {
    Transpose(f32),
    Normalize(f32),
}

impl RegionOperation {
    fn label(self) -> &'static str {
        match self {
            Self::Transpose(_) => "Transpose region",
            Self::Normalize(_) => "Normalize region",
        }
    }

    fn progress(self) -> String {
        match self {
            Self::Transpose(amount) => format!("Transposing region {amount:+.2} semitones…"),
            Self::Normalize(target) => format!("Normalizing region to {target:.1} dBFS…"),
        }
    }

    fn process(self, original: &Clip, rate: u32, cancel: &AtomicBool) -> WorkerResult {
        match self {
            Self::Transpose(amount) => {
                let end = original
                    .source_offset
                    .checked_add(original.frames)
                    .ok_or("Invalid region")?;
                let source = original
                    .samples
                    .get(original.source_offset..end)
                    .ok_or("Invalid region")?;
                let samples = resonara_core::pitch::transpose_region(source, rate, amount, cancel)
                    .map_err(|error| error.to_string())?;
                ProcessedAudio::new(samples, original.frames, cancel).map(RegionOutput::Audio)
            }
            Self::Normalize(target) => {
                normalize_gain(original, target, cancel).map(RegionOutput::Gain)
            }
        }
    }
}

// Only a worker constructs this after validating the complete buffer. Committing
// it on the UI thread then requires only a length check and an Arc assignment.
struct ProcessedAudio(Arc<Vec<[f32; 2]>>);
impl ProcessedAudio {
    fn new(
        samples: Vec<[f32; 2]>,
        frames: usize,
        cancel: &AtomicBool,
    ) -> std::result::Result<Self, String> {
        if samples.len() != frames {
            return Err("Region processor returned invalid audio".into());
        }
        for chunk in samples.chunks(1024) {
            if cancel.load(Ordering::Acquire) {
                return Err("Region processing cancelled".into());
            }
            if chunk.iter().flatten().any(|x| !x.is_finite()) {
                return Err("Region processor returned invalid audio".into());
            }
        }
        Ok(Self(Arc::new(samples)))
    }
}

enum RegionOutput {
    Audio(ProcessedAudio),
    Gain(f32),
}
type WorkerResult = std::result::Result<RegionOutput, String>;

fn normalize_gain(
    original: &Clip,
    target: f32,
    cancel: &AtomicBool,
) -> std::result::Result<f32, String> {
    if !target.is_finite() || !(-60.0..=0.0).contains(&target) {
        return Err("Normalization target must be between -60 and 0 dBFS".into());
    }
    if original.frames == 0
        || original
            .source_offset
            .checked_add(original.frames)
            .is_none_or(|end| end > original.samples.len())
    {
        return Err("Invalid region".into());
    }
    let mut clip = original.clone();
    // Match Clip::normalize: fades and reverse remain audible, but the new gain
    // replaces the existing gain instead of adding a second normalization pass.
    clip.set_gain_db(0.).map_err(|error| error.to_string())?;
    let mut peak = 0f32;
    for frame in 0..clip.frames {
        if frame % 1024 == 0 && cancel.load(Ordering::Acquire) {
            return Err("Region processing cancelled".into());
        }
        let sample = clip.sample_at(frame as f64);
        if sample.iter().any(|value| !value.is_finite()) {
            return Err("Cannot normalize non-finite audio".into());
        }
        peak = peak.max(sample[0].abs()).max(sample[1].abs());
    }
    if cancel.load(Ordering::Acquire) {
        return Err("Region processing cancelled".into());
    }
    if peak <= f32::MIN_POSITIVE {
        return Err("Cannot normalize a silent region".into());
    }
    let gain = target - 20.0 * peak.log10();
    clip.set_gain_db(gain).map_err(|error| error.to_string())?;
    Ok(gain)
}

pub(super) struct RegionJob {
    track: usize,
    clip: usize,
    version: u64,
    rate: u32,
    original: Clip,
    operation: RegionOperation,
    cancel: Arc<AtomicBool>,
    result: mpsc::Receiver<WorkerResult>,
}

impl Drop for RegionJob {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
    }
}

impl RegionJob {
    fn matches(&self, m: &Model) -> bool {
        m.version == self.version
            && m.project.sample_rate == self.rate
            && m.project
                .tracks
                .get(self.track)
                .and_then(|track| track.clips.get(self.clip))
                .is_some_and(|clip| {
                    Arc::ptr_eq(&clip.samples, &self.original.samples)
                        && clip.start == self.original.start
                        && clip.source_offset == self.original.source_offset
                        && clip.frames == self.original.frames
                        && clip.source_channels == self.original.source_channels
                        && clip.edit == self.original.edit
                })
    }
}

impl Daw {
    pub(super) fn region_processing_active(&self) -> bool {
        self.region_job.borrow().is_some()
    }

    pub(super) fn editor_transpose(&self) {
        let amount = match self.region_editor.transpose.get().trim().parse::<f32>() {
            Ok(value)
                if value.is_finite()
                    && value.abs() <= resonara_core::pitch::MAX_TRANSPOSE_SEMITONES =>
            {
                value
            }
            _ => {
                self.status
                    .set("Transpose must be between -12.00 and +12.00 semitones".into());
                return;
            }
        };
        if amount != 0. {
            self.start_region_processing(RegionOperation::Transpose(amount));
        }
    }

    pub(super) fn editor_normalize(&self) {
        let target = match self.region_editor.normalize.get().trim().parse::<f32>() {
            Ok(value) if value.is_finite() && (-60.0..=0.0).contains(&value) => value,
            _ => {
                self.status
                    .set("Normalization target must be between -60 and 0 dBFS".into());
                return;
            }
        };
        self.start_region_processing(RegionOperation::Normalize(target));
    }

    fn start_region_processing(&self, operation: RegionOperation) {
        if self.busy() || self.region_processing_active() || self.dialog.get() != Dialog::None {
            return;
        }
        self.finish_mix();
        let m = self.model.borrow();
        if m.selected_bus.is_some() || m.drag.is_some() {
            return;
        }
        let Some(clip) = m.clip else {
            return;
        };
        let Some(original) = m
            .project
            .tracks
            .get(m.selected)
            .and_then(|track| track.clips.get(clip))
            .cloned()
        else {
            return;
        };
        let rate = m.project.sample_rate;
        let input = original.clone();
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = cancel.clone();
        let (tx, result) = mpsc::channel();
        let worker = std::thread::Builder::new()
            .name("region-processing".into())
            .spawn(move || {
                let output = operation.process(&input, rate, &worker_cancel);
                let _ = tx.send(output);
            });
        if let Err(error) = worker {
            self.status
                .set(format!("Could not start region processing: {error}"));
            return;
        }
        *self.region_job.borrow_mut() = Some(RegionJob {
            track: m.selected,
            clip,
            version: m.version,
            rate,
            original,
            operation,
            cancel,
            result,
        });
        drop(m);
        self.status.set(operation.progress());
        self.changed();
    }

    pub(super) fn cancel_region_processing(&self) {
        if self.region_job.borrow_mut().take().is_some() {
            self.status.set("Region processing cancelled".into());
            self.changed();
        }
    }

    pub(super) fn poll_region_processing(&self) {
        // Never commit in the middle of a gesture, file operation, or modal
        // edit. The user can continue playing and navigating during processing.
        if self.busy() || self.dialog.get() != Dialog::None {
            return;
        }
        {
            let m = self.model.borrow();
            if m.drag.is_some() || m.mixer_before.is_some() {
                return;
            }
        }
        let output = {
            let jobs = self.region_job.borrow();
            let Some(job) = jobs.as_ref() else {
                return;
            };
            match job.result.try_recv() {
                Ok(output) => output,
                Err(mpsc::TryRecvError::Empty) => return,
                Err(mpsc::TryRecvError::Disconnected) => {
                    Err("Region worker ended without a result".into())
                }
            }
        };
        let job = self.region_job.borrow_mut().take().unwrap();
        if !job.matches(&self.model.borrow()) {
            self.status
                .set("Region result discarded because the project changed".into());
            self.changed();
            return;
        }
        match output {
            Ok(output) => {
                let valid = match (&output, job.operation) {
                    (RegionOutput::Audio(audio), RegionOperation::Transpose(_)) => {
                        audio.0.len() == job.original.frames
                    }
                    (RegionOutput::Gain(gain), RegionOperation::Normalize(_)) => {
                        gain.is_finite() && (-60.0..=24.0).contains(gain)
                    }
                    _ => false,
                };
                if !valid {
                    self.status
                        .set("Region processor returned invalid audio or gain".into());
                    self.changed();
                    return;
                }
                self.edit(job.operation.label(), |m| {
                    // Polling CLAP state inside edit() can itself change the
                    // project. Recheck after that poll and before mutating it.
                    if !job.matches(m) {
                        return Err("Project changed during region processing".into());
                    }
                    let clip = &mut m.project.tracks[job.track].clips[job.clip];
                    match output {
                        RegionOutput::Audio(audio) => {
                            clip.samples = audio.0;
                            clip.source_offset = 0;
                        }
                        RegionOutput::Gain(gain) => clip.set_gain_db(gain)?,
                    }
                    Ok(())
                });
                if matches!(job.operation, RegionOperation::Transpose(_)) {
                    self.region_editor.transpose.set("0.00".into());
                }
            }
            Err(error) => {
                self.status.set(format!(
                    "Could not {}: {error}",
                    job.operation.label().to_lowercase()
                ));
                self.changed();
            }
        }
    }
}

#[cfg(test)]
#[path = "region_processing_tests.rs"]
mod tests;
