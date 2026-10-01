//! Reproducible, headless workload for a licensed real-audio fixture.
//! This measures core render cost, not device latency or native UI frame rate.
use resonara_core::{Controls, Engine, Project, Result};
use std::{path::PathBuf, sync::Arc, time::Instant};

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let input = PathBuf::from(
        args.next()
            .ok_or("Usage: stress input.wav output.wav [tracks]")?,
    );
    let output = PathBuf::from(args.next().ok_or("Missing output WAV path")?);
    let tracks: usize = args.next().map_or(Ok(8), |v| v.parse())?;
    if !(1..=64).contains(&tracks) {
        return Err("Track count must be between 1 and 64".into());
    }
    let mut project = Project::default();
    let imported = Instant::now();
    project.import_wav(&input)?;
    let import_seconds = imported.elapsed().as_secs_f64();
    let source = project.tracks[0].clone();
    project.tracks = (0..tracks)
        .map(|index| {
            let mut track = source.clone();
            track.name = format!("Real audio {:02}", index + 1);
            track.gain = 1.0 / tracks as f32;
            track.pan = if tracks == 1 {
                0.0
            } else {
                -0.7 + 1.4 * index as f32 / (tracks - 1) as f32
            };
            track.clips[0].start = index as u64 * u64::from(project.sample_rate) / 4;
            track
        })
        .collect();
    project.validate()?;
    assert!(
        project
            .tracks
            .iter()
            .all(|track| Arc::ptr_eq(&source.clips[0].samples, &track.clips[0].samples))
    );
    let controls = Arc::new(Controls::new(&project));
    let mut engine = Engine::new(&project, controls, project.sample_rate, 0);
    let frames = project.duration();
    let mut block = [0.0f32; 1024];
    let mut timings = Vec::with_capacity(frames.div_ceil(512) as usize);
    let mut nonzero = 0u64;
    let mut peak = 0.0f32;
    let mut oracle_samples = 0u64;
    let render_start = Instant::now();
    for start in (0..frames).step_by(512) {
        let count = (frames - start).min(512) as usize;
        let buffer = &mut block[..count * 2];
        let begin = Instant::now();
        engine.render(buffer, 2);
        timings.push(begin.elapsed().as_secs_f64() * 1_000_000.0);
        for sample in buffer.iter() {
            assert!(sample.is_finite(), "Non-finite render output");
            nonzero += u64::from(*sample != 0.0);
            peak = peak.max(sample.abs());
        }
        // An independent, direct sample-index oracle at the project rate.
        // Include boundaries and deterministic blocks throughout the whole file.
        if start < 16_384 || start / 512 % 257 == 0 || start + count as u64 == frames {
            for (offset, frame) in buffer.chunks_exact(2).enumerate() {
                let position = start + offset as u64;
                let mut expected = [0.0f32; 2];
                for track in &project.tracks {
                    let clip = &track.clips[0];
                    if position >= clip.start && position - clip.start < clip.frames as u64 {
                        let source =
                            clip.samples[clip.source_offset + (position - clip.start) as usize];
                        expected[0] += source[0] * track.gain * (1.0 - track.pan.max(0.0));
                        expected[1] += source[1] * track.gain * (1.0 + track.pan.min(0.0));
                    }
                }
                for channel in 0..2 {
                    let expected = (expected[channel] * project.master).clamp(-1.0, 1.0);
                    assert!(
                        (frame[channel] - expected).abs() <= 1e-6,
                        "Mix oracle mismatch"
                    );
                    oracle_samples += 1;
                }
            }
        }
    }
    let render_wall_seconds = render_start.elapsed().as_secs_f64();
    assert!(nonzero > 0, "The real-audio fixture must not be silent");
    engine.render(&mut block, 2);
    assert!(
        block.iter().all(|sample| *sample == 0.0),
        "EOF must be silent"
    );
    timings.sort_by(f64::total_cmp);
    let percentile = |p: f64| timings[((timings.len() - 1) as f64 * p).ceil() as usize];
    let export_start = Instant::now();
    project.export_wav(&output)?;
    let export_seconds = export_start.elapsed().as_secs_f64();
    let reader = hound::WavReader::open(&output)?;
    assert_eq!(reader.spec().channels, 2);
    assert_eq!(reader.spec().sample_rate, project.sample_rate);
    assert_eq!(reader.spec().sample_format, hound::SampleFormat::Float);
    assert_eq!(u64::from(reader.duration()), frames);
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "input": input,
            "output": output,
            "profile": if cfg!(debug_assertions) { "debug" } else { "release" },
            "tracks": tracks,
            "shared_source_buffers": 1,
            "source_frames": source.clips[0].frames,
            "sample_rate": project.sample_rate,
            "duration_seconds": frames as f64 / f64::from(project.sample_rate),
            "import_seconds": import_seconds,
            "render_wall_seconds_including_checks": render_wall_seconds,
            "block_frames": 512,
            "block_deadline_us": 512.0 / f64::from(project.sample_rate) * 1_000_000.0,
            "render_block_p50_us": percentile(0.5),
            "render_block_p95_us": percentile(0.95),
            "render_block_p99_us": percentile(0.99),
            "render_block_max_us": timings.last(),
            "nonzero_samples": nonzero,
            "peak": peak,
            "independently_checked_samples": oracle_samples,
            "export_seconds": export_seconds,
            "export_bytes": std::fs::metadata(output)?.len(),
            "logical_parallelism": std::thread::available_parallelism().map(|n| n.get()).ok(),
            "limits": "Offline core benchmark on a shared cloud CPU. No device latency, UI frame-rate, or hard real-time guarantee."
        }))?
    );
    Ok(())
}
