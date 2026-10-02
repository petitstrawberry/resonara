//! Allocation-free PCM staging for the SAS ring. Kept host-testable.
use resonara_core::Engine;
use std::sync::atomic::Ordering;

pub(crate) const CHANNELS: usize = 2;
pub(crate) const PERIOD_FRAMES: usize = 256;
pub(crate) const FRAME_BYTES: usize = CHANNELS * 2;

pub(crate) trait Ring {
    fn closed(&self) -> bool;
    fn writable_frames(&self) -> usize;
    /// Return the number of complete stereo frames accepted.
    fn write(&mut self, bytes: &[u8]) -> usize;
    fn begin_drain(&mut self);
    fn is_empty(&self) -> bool;
    fn read_frames(&self) -> u64;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Step {
    Progress,
    Waiting,
    Finished,
    Cancelled,
}

pub(crate) struct Pump {
    samples: [i16; PERIOD_FRAMES * CHANNELS],
    bytes: [u8; PERIOD_FRAMES * FRAME_BYTES],
    pending: usize,
    sent: usize,
    rendering_finished: bool,
    draining: bool,
    drained_frames: u64,
}

impl Pump {
    pub(crate) fn new() -> Self {
        Self {
            samples: [0; PERIOD_FRAMES * CHANNELS],
            bytes: [0; PERIOD_FRAMES * FRAME_BYTES],
            pending: 0,
            sent: 0,
            rendering_finished: false,
            draining: false,
            drained_frames: 0,
        }
    }

    /// One non-blocking producer/drain step. A short write retains its exact
    /// remainder. Natural EOF completes only after SAS consumes every staged
    /// frame; explicit cancellation never drains or waits for ring capacity.
    pub(crate) fn step(
        &mut self,
        engine: &mut Engine,
        ring: &mut impl Ring,
        cancelled: bool,
    ) -> Result<Step, ()> {
        if cancelled {
            return Ok(Step::Cancelled);
        }
        if ring.closed() {
            return Err(());
        }
        if self.draining {
            if ring.is_empty() {
                return Ok(Step::Finished);
            }
            let read_frames = ring.read_frames();
            if read_frames != self.drained_frames {
                self.drained_frames = read_frames;
                return Ok(Step::Progress);
            }
            return Ok(Step::Waiting);
        }
        let writable = ring.writable_frames();
        if writable == 0 {
            return Ok(Step::Waiting);
        }
        if self.sent == self.pending {
            engine.render(&mut self.samples, CHANNELS);
            self.rendering_finished = !engine.controls.playing.load(Ordering::Relaxed);
            for (sample, bytes) in self.samples.iter().zip(self.bytes.chunks_exact_mut(2)) {
                bytes.copy_from_slice(&sample.to_le_bytes());
            }
            self.pending = PERIOD_FRAMES;
            self.sent = 0;
        }
        let frames = (self.pending - self.sent).min(writable);
        let start = self.sent * FRAME_BYTES;
        let accepted = ring.write(&self.bytes[start..start + frames * FRAME_BYTES]);
        if accepted > frames {
            return Err(());
        }
        self.sent += accepted;
        if self.rendering_finished && self.sent == self.pending {
            ring.begin_drain();
            self.draining = true;
            self.drained_frames = ring.read_frames();
        }
        Ok(if accepted == 0 {
            Step::Waiting
        } else {
            Step::Progress
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use resonara_core::{Controls, Project};
    use std::sync::{Arc, atomic::Ordering};

    struct TestRing {
        closed: bool,
        writable: usize,
        accept: usize,
        bytes: Vec<u8>,
        read_frames: u64,
        drain_calls: usize,
    }
    impl Ring for TestRing {
        fn closed(&self) -> bool {
            self.closed
        }
        fn writable_frames(&self) -> usize {
            self.writable
        }
        fn write(&mut self, bytes: &[u8]) -> usize {
            let frames = self.accept.min(bytes.len() / FRAME_BYTES);
            self.bytes.extend_from_slice(&bytes[..frames * FRAME_BYTES]);
            frames
        }
        fn begin_drain(&mut self) {
            self.drain_calls += 1;
        }
        fn is_empty(&self) -> bool {
            self.read_frames as usize >= self.bytes.len() / FRAME_BYTES
        }
        fn read_frames(&self) -> u64 {
            self.read_frames
        }
    }
    fn setup() -> (Pump, Engine, Arc<Controls>, TestRing) {
        let project = Project::demo();
        let controls = Arc::new(Controls::new(&project));
        let engine = Engine::new(&project, controls.clone(), 48_000, 0);
        (
            Pump::new(),
            engine,
            controls,
            TestRing {
                closed: false,
                writable: PERIOD_FRAMES,
                accept: PERIOD_FRAMES,
                bytes: vec![],
                read_frames: 0,
                drain_calls: 0,
            },
        )
    }
    #[test]
    fn partial_writes_preserve_every_pcm_frame_and_byte_order() {
        let (mut pump, mut engine, _, mut ring) = setup();
        ring.accept = 17;
        while ring.bytes.len() < 2 * PERIOD_FRAMES * FRAME_BYTES {
            assert_eq!(pump.step(&mut engine, &mut ring, false), Ok(Step::Progress));
        }
        let (_, mut reference, _, _) = setup();
        let mut samples = [0_i16; 2 * PERIOD_FRAMES * CHANNELS];
        reference.render(&mut samples, CHANNELS);
        let expected: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
        assert_eq!(ring.bytes, expected);
    }
    #[test]
    fn live_swap_preserves_partially_staged_pcm_before_rendering_the_new_graph() {
        use resonara_core::{Clip, Insert, InsertKind, live::Playback};
        let mut project = Project::demo();
        project.tracks.truncate(1);
        project.tracks[0].gain = 1.;
        project.tracks[0].pan = 0.;
        project.master = 1.;
        project.tracks[0].clips = vec![Clip {
            edit: Default::default(),
            source_channels: 2,
            start: 0,
            source_offset: 0,
            frames: 1024,
            samples: Arc::new(vec![[0.125, -0.25]; 1024]),
        }];
        let (mut playback, mut renderer) =
            Playback::new(&project, project.sample_rate, 0, false).unwrap();
        let (_, _, _, mut ring) = setup();
        ring.accept = 17;
        let mut pump = Pump::new();
        pump.step(renderer.engine_mut(), &mut ring, false).unwrap();
        assert_eq!(playback.controls.position.load(Ordering::Relaxed), 256);
        project.tracks[0].routing.inserts.push(Insert {
            kind: InsertKind::Gain { gain: 0.5 },
            bypass: false,
        });
        playback.update(&project).unwrap();
        renderer.apply_pending();
        while ring.bytes.len() < 2 * PERIOD_FRAMES * FRAME_BYTES {
            pump.step(renderer.engine_mut(), &mut ring, false).unwrap();
        }
        let mut expected = [4096i16, -8192].repeat(PERIOD_FRAMES);
        expected.extend([2048i16, -4096].repeat(PERIOD_FRAMES));
        assert_eq!(
            ring.bytes,
            expected
                .iter()
                .flat_map(|s| s.to_le_bytes())
                .collect::<Vec<_>>()
        );
        assert_eq!(playback.controls.position.load(Ordering::Relaxed), 512);
        drop(renderer);
    }
    #[test]
    fn full_or_closed_ring_does_not_advance_the_engine() {
        let (mut pump, mut engine, controls, mut ring) = setup();
        ring.writable = 0;
        assert_eq!(pump.step(&mut engine, &mut ring, false), Ok(Step::Waiting));
        assert_eq!(controls.position.load(Ordering::Relaxed), 0);
        ring.closed = true;
        ring.writable = PERIOD_FRAMES;
        assert_eq!(pump.step(&mut engine, &mut ring, false), Err(()));
        assert_eq!(controls.position.load(Ordering::Relaxed), 0);
    }
    #[test]
    fn zero_write_keeps_pending_samples_for_retry() {
        let (mut pump, mut engine, controls, mut ring) = setup();
        ring.accept = 0;
        assert_eq!(pump.step(&mut engine, &mut ring, false), Ok(Step::Waiting));
        assert_eq!(
            controls.position.load(Ordering::Relaxed),
            PERIOD_FRAMES as u64
        );
        ring.accept = PERIOD_FRAMES;
        assert_eq!(pump.step(&mut engine, &mut ring, false), Ok(Step::Progress));
        assert_eq!(
            controls.position.load(Ordering::Relaxed),
            PERIOD_FRAMES as u64
        );
        assert_eq!(ring.bytes.len(), PERIOD_FRAMES * FRAME_BYTES);
    }

    fn short_setup() -> (Pump, Engine, Arc<Controls>, TestRing) {
        let (pump, _, _, ring) = setup();
        let mut project = Project::demo();
        project.tracks.truncate(1);
        project.tracks[0].clips.truncate(1);
        project.tracks[0].clips[0].start = 0;
        project.tracks[0].clips[0].frames = 13;
        let controls = Arc::new(Controls::new(&project));
        let engine = Engine::new(&project, controls.clone(), 48_000, 0);
        (pump, engine, controls, ring)
    }

    #[test]
    fn eof_flushes_pending_pcm_before_draining_and_waits_for_consumption() {
        let (mut pump, mut engine, controls, mut ring) = short_setup();
        ring.accept = 17;
        assert_eq!(pump.step(&mut engine, &mut ring, false), Ok(Step::Progress));
        assert!(!controls.playing.load(Ordering::Relaxed));
        assert_eq!(controls.position.load(Ordering::Relaxed), 13);
        assert_eq!(ring.drain_calls, 0, "staged PCM remains");
        ring.writable = 0;
        assert_eq!(pump.step(&mut engine, &mut ring, false), Ok(Step::Waiting));
        assert_eq!(ring.drain_calls, 0);
        ring.writable = PERIOD_FRAMES;
        while ring.bytes.len() < PERIOD_FRAMES * FRAME_BYTES {
            assert_eq!(pump.step(&mut engine, &mut ring, false), Ok(Step::Progress));
        }
        assert_eq!(ring.drain_calls, 1);
        assert_eq!(pump.step(&mut engine, &mut ring, false), Ok(Step::Waiting));
        ring.read_frames = 100;
        assert_eq!(pump.step(&mut engine, &mut ring, false), Ok(Step::Progress));
        assert_eq!(pump.step(&mut engine, &mut ring, false), Ok(Step::Waiting));
        ring.read_frames = PERIOD_FRAMES as u64;
        assert_eq!(pump.step(&mut engine, &mut ring, false), Ok(Step::Finished));
        assert_eq!(pump.step(&mut engine, &mut ring, false), Ok(Step::Finished));
        assert_eq!(ring.bytes.len(), PERIOD_FRAMES * FRAME_BYTES);
        assert_eq!(ring.drain_calls, 1);
        let (_, mut reference, _, _) = short_setup();
        let mut samples = [0_i16; PERIOD_FRAMES * CHANNELS];
        reference.render(&mut samples, CHANNELS);
        let expected: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
        assert_eq!(ring.bytes, expected);
    }

    #[test]
    fn cancellation_never_waits_for_pending_pcm_or_a_draining_ring() {
        for accept in [17, PERIOD_FRAMES] {
            let (mut pump, mut engine, _, mut ring) = short_setup();
            ring.accept = accept;
            assert_eq!(pump.step(&mut engine, &mut ring, false), Ok(Step::Progress));
            let bytes_before = ring.bytes.len();
            let drains_before = ring.drain_calls;
            ring.writable = 0;
            ring.closed = true;
            assert_eq!(pump.step(&mut engine, &mut ring, true), Ok(Step::Cancelled));
            assert_eq!(ring.bytes.len(), bytes_before);
            assert_eq!(ring.drain_calls, drains_before);
        }
    }

    #[test]
    fn closed_ring_during_eof_flush_or_drain_reports_failure() {
        for accept in [17, PERIOD_FRAMES] {
            let (mut pump, mut engine, _, mut ring) = short_setup();
            ring.accept = accept;
            assert_eq!(pump.step(&mut engine, &mut ring, false), Ok(Step::Progress));
            ring.closed = true;
            assert_eq!(pump.step(&mut engine, &mut ring, false), Err(()));
        }
    }
}
