//! Matched old/new core-render benchmark. Copy unchanged into each checkout.
//! Linux thread CPU time distinguishes DSP work from shared-host descheduling.
use resonara_core::{Controls, Engine, Project, Result};
use std::{path::PathBuf, sync::Arc, time::Instant};
#[cfg(target_os = "linux")]
fn cpu_ns() -> u64 {
    #[repr(C)]
    struct Timespec {
        seconds: i64,
        nanoseconds: i64,
    }
    unsafe extern "C" {
        fn clock_gettime(clock: i32, value: *mut Timespec) -> i32;
    }
    let mut time = Timespec {
        seconds: 0,
        nanoseconds: 0,
    };
    let result = unsafe { clock_gettime(3, &mut time) }; // CLOCK_THREAD_CPUTIME_ID
    assert_eq!(result, 0);
    time.seconds as u64 * 1_000_000_000 + time.nanoseconds as u64
}
#[cfg(not(target_os = "linux"))]
fn cpu_ns() -> u64 {
    0
}
fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let input = PathBuf::from(args.next().ok_or("Usage: compare_render input.wav label")?);
    let label = args.next().unwrap_or_else(|| "unspecified".into());
    let mut project = Project::default();
    project.import_wav(&input)?;
    let source = project.tracks[0].clone();
    project.tracks = (0..8)
        .map(|index| {
            let mut track = source.clone();
            track.name = format!("Real audio {:02}", index + 1);
            track.gain = 1.0 / 8.0;
            track.pan = -0.7 + 1.4 * index as f32 / 7.0;
            track.clips[0].start = index as u64 * u64::from(project.sample_rate) / 4;
            track
        })
        .collect();
    project.validate()?;
    let build = || {
        Engine::new(
            &project,
            Arc::new(Controls::new(&project)),
            project.sample_rate,
            0,
        )
    };
    let mut block = [0.0f32; 1024];
    // Exercise code, allocator-independent state and the start of source data
    // before timing. A fresh prepared engine then renders the identical timeline.
    let mut warm = build();
    for _ in 0..128 {
        warm.render(&mut block, 2);
    }
    drop(warm);
    let mut engine = build();
    let frames = project.duration();
    let mut wall = Vec::with_capacity(frames.div_ceil(512) as usize);
    let mut cpu = Vec::with_capacity(wall.capacity());
    let mut hash = 0xcbf29ce484222325u64;
    let mut oracle_samples = 0u64;
    for start in (0..frames).step_by(512) {
        let count = (frames - start).min(512) as usize;
        let output = &mut block[..count * 2];
        let cpu_begin = cpu_ns();
        let wall_begin = Instant::now();
        engine.render(output, 2);
        wall.push(wall_begin.elapsed().as_secs_f64() * 1e6);
        cpu.push((cpu_ns() - cpu_begin) as f64 / 1e3);
        for sample in output.iter() {
            assert!(sample.is_finite());
            hash = (hash ^ u64::from(sample.to_bits())).wrapping_mul(0x100000001b3);
        }
        if start < 16384 || start / 512 % 257 == 0 || start + count as u64 == frames {
            for (offset, actual) in output.chunks_exact(2).enumerate() {
                let position = start + offset as u64;
                let mut expected = [0.0f32; 2];
                for track in &project.tracks {
                    let clip = &track.clips[0];
                    if position >= clip.start && position - clip.start < clip.frames as u64 {
                        let sample =
                            clip.samples[clip.source_offset + (position - clip.start) as usize];
                        expected[0] += sample[0] * track.gain * (1.0 - track.pan.max(0.0));
                        expected[1] += sample[1] * track.gain * (1.0 + track.pan.min(0.0));
                    }
                }
                for channel in 0..2 {
                    let expected = (expected[channel] * project.master).clamp(-1.0, 1.0);
                    assert!((actual[channel] - expected).abs() <= 1e-6);
                    oracle_samples += 1;
                }
            }
        }
    }
    engine.render(&mut block, 2);
    assert!(block.iter().all(|sample| *sample == 0.0));
    let deadline = 512.0 / f64::from(project.sample_rate) * 1e6;
    let stats = |mut values: Vec<f64>| {
        let total: f64 = values.iter().sum();
        let misses = values.iter().filter(|&&value| value > deadline).count();
        values.sort_by(f64::total_cmp);
        let p = |fraction: f64| values[((values.len() - 1) as f64 * fraction).ceil() as usize];
        serde_json::json!({"total_us":total,"p50_us":p(0.5),"p95_us":p(0.95),"p99_us":p(0.99),"max_us":values.last(),"over_deadline":misses})
    };
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "label":label,"profile":if cfg!(debug_assertions) {"debug"} else {"release"},
            "tracks":8,"frames":frames,"sample_rate":project.sample_rate,"block_frames":512,
            "callbacks":wall.len(),"deadline_us":deadline,"warmup_callbacks":128,
            "audio_hash_fnv1a_bits":format!("{hash:016x}"),"oracle_samples":oracle_samples,
            "wall":stats(wall),"thread_cpu":stats(cpu),
            "limits":"Same source and render harness; CPU-clock calls are outside render and add small common measurement overhead. Shared CPU can deschedule either run. Thread CPU timing is Linux-only.",
        }))?
    );
    Ok(())
}
