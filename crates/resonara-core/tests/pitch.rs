use resonara_core::pitch::transpose_region;
use rustfft::{FftPlannerScalar, num_complex::Complex};
use std::f64::consts::TAU;
use std::sync::atomic::AtomicBool;

fn tone(rate: u32, length: usize, frequency: f64) -> Vec<[f32; 2]> {
    (0..length)
        .map(|i| {
            let sample = (0.35 * (TAU * frequency * i as f64 / rate as f64).sin()) as f32;
            [sample, sample]
        })
        .collect()
}

fn transpose(samples: &[[f32; 2]], rate: u32, semitones: f32) -> Vec<[f32; 2]> {
    transpose_region(samples, rate, semitones, &AtomicBool::new(false)).unwrap()
}

// Inspect the rendered samples, independently of the shifter's phase tracking.
// Parabolic interpolation of the log power around a Hann-windowed FFT peak
// measures fractional-bin frequencies as well as octave shifts.
fn dominant_frequency(samples: &[[f32; 2]], rate: u32) -> f64 {
    let length = (samples.len() / 2).next_power_of_two();
    let start = (samples.len() - length) / 2;
    let mut spectrum: Vec<_> = samples[start..start + length]
        .iter()
        .enumerate()
        .map(|(i, frame)| {
            let window = 0.5 - 0.5 * (TAU * i as f64 / length as f64).cos();
            Complex::new(frame[0] * window as f32, 0.0)
        })
        .collect();
    FftPlannerScalar::new()
        .plan_fft_forward(length)
        .process(&mut spectrum);
    let bin = (1..length / 2)
        .max_by(|a, b| spectrum[*a].norm_sqr().total_cmp(&spectrum[*b].norm_sqr()))
        .unwrap();
    let a = (spectrum[bin - 1].norm_sqr() as f64).max(1e-30).ln();
    let b = (spectrum[bin].norm_sqr() as f64).max(1e-30).ln();
    let c = (spectrum[bin + 1].norm_sqr() as f64).max(1e-30).ln();
    let correction = 0.5 * (a - c) / (a - 2.0 * b + c);
    (bin as f64 + correction) * rate as f64 / length as f64
}

fn rms(samples: &[[f32; 2]]) -> f64 {
    (samples
        .iter()
        .map(|frame| (frame[0] as f64).powi(2))
        .sum::<f64>()
        / samples.len() as f64)
        .sqrt()
}

#[test]
fn octave_and_fractional_bin_transposition_changes_frequency_not_duration() {
    let rate = 48000;
    let input = tone(rate, rate as usize + 113, 440.0);
    for semitones in [-12.0, 7.0, 12.0] {
        let output = transpose(&input, rate, semitones);
        assert_eq!(output.len(), input.len());
        let expected = 440.0 * 2.0_f64.powf(semitones as f64 / 12.0);
        let measured = dominant_frequency(&output, rate);
        assert!(
            (measured - expected).abs() < 0.6,
            "{semitones} semitones: measured {measured}, expected {expected}"
        );
        let level = rms(&output[rate as usize / 4..rate as usize * 3 / 4]);
        assert!((0.20..0.29).contains(&level), "unexpected RMS {level}");
    }
}

#[test]
fn bass_transposes_for_small_changes_too() {
    let rate = 48000;
    let input = tone(rate, rate as usize * 2 + 37, 60.0);
    let output = transpose(&input, rate, 1.0);
    let expected = 60.0 * 2.0_f64.powf(1.0 / 12.0);
    let measured = dominant_frequency(&output, rate);
    assert!((measured - expected).abs() < 0.3, "measured {measured}");
}

#[test]
fn sample_rate_and_cent_offsets_are_respected() {
    for rate in [8000, 44100, 96000, 192000] {
        let input = tone(rate, rate as usize + 53, 321.7);
        let output = transpose(&input, rate, 0.5);
        assert_eq!(output.len(), input.len());
        let expected = 321.7 * 2.0_f64.powf(0.5 / 12.0);
        assert!(
            (dominant_frequency(&output, rate) - expected).abs() < 0.8,
            "rate {rate}"
        );
    }
}

#[test]
fn silence_short_clips_and_empty_regions_remain_finite_and_same_length() {
    for length in [0, 1, 17, 511, 4097] {
        for shift in [-12.0, 3.5, 12.0] {
            let silent = transpose(&vec![[0.0; 2]; length], 48000, shift);
            assert_eq!(silent, vec![[0.0; 2]; length]);
            let output = transpose(&tone(48000, length, 440.0), 48000, shift);
            assert_eq!(output.len(), length);
            assert!(output.iter().flatten().all(|sample| sample.is_finite()));
        }
    }
}

#[test]
fn stereo_relationships_survive_including_inverse_polarity() {
    let input = tone(48000, 24017, 257.3);
    for right_gain in [1.0, -1.0, 0.0, 0.4] {
        let stereo: Vec<_> = input
            .iter()
            .map(|frame| [frame[0], frame[0] * right_gain])
            .collect();
        let output = transpose(&stereo, 48000, -5.0);
        assert!(
            output
                .iter()
                .all(|frame| (frame[1] - frame[0] * right_gain).abs() < 2e-5),
            "right gain {right_gain}"
        );
        assert!(rms(&output) > 0.15);
    }
}

#[test]
fn zero_semitones_is_bit_exact_and_source_is_untouched() {
    let input = vec![[-0.0, 0.0], [0.37, -0.81], [-2.5, 1.5]];
    let output = transpose(&input, 44100, 0.0);
    assert_eq!(
        input
            .iter()
            .flatten()
            .map(|v| v.to_bits())
            .collect::<Vec<_>>(),
        output
            .iter()
            .flatten()
            .map(|v| v.to_bits())
            .collect::<Vec<_>>()
    );
    let before = input.clone();
    let _ = transpose(&input, 44100, 7.0);
    assert_eq!(input, before);
}

#[test]
fn beginning_and_end_audio_are_not_discarded_as_latency() {
    // A short burst at each edge makes an incorrect Nfft/2 offset or missing
    // drain/padding visible without pretending a phase vocoder keeps impulses
    // completely sharp at large transpositions.
    let length = 48000;
    let mut input = tone(48000, length, 731.0);
    for frame in &mut input[4096..length - 4096] {
        *frame = [0.0; 2];
    }
    for shift in [-12.0, 12.0] {
        let output = transpose(&input, 48000, shift);
        assert!(rms(&output[..4096]) > 0.10, "start lost at {shift}");
        assert!(rms(&output[length - 4096..]) > 0.10, "end lost at {shift}");
        assert!(rms(&output[12000..36000]) < 1e-5);
    }
}

#[test]
fn pitch_up_filters_frequencies_that_would_alias() {
    let input = tone(48000, 48000, 18000.0);
    let output = transpose(&input, 48000, 12.0);
    let level = rms(&output[12000..36000]);
    assert!(level < 0.005, "aliased RMS {level}");
}

#[test]
fn cancelled_or_invalid_requests_fail_without_partial_output() {
    let cancelled = AtomicBool::new(true);
    assert!(transpose_region(&[[0.0; 2]; 16], 48000, 3.0, &cancelled).is_err());
    let cancel = AtomicBool::new(false);
    for rate in [0, 7999, 192001] {
        assert!(transpose_region(&[], rate, 0.0, &cancel).is_err());
    }
    for shift in [f32::NAN, f32::INFINITY, -12.1, 12.1] {
        assert!(transpose_region(&[], 48000, shift, &cancel).is_err());
    }
    for invalid in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        assert!(transpose_region(&[[invalid, 0.0]], 48000, 0.0, &cancel).is_err());
    }
}

#[test]
fn finite_input_that_overflows_the_fft_is_reported_instead_of_becoming_silence() {
    let input = vec![[f32::MAX; 2]; 4096];
    let result = transpose_region(&input, 48000, 7.0, &AtomicBool::new(false));
    assert!(result.is_err());
}

#[test]
fn an_in_flight_worker_can_be_cancelled() {
    use std::sync::{Arc, atomic::Ordering};
    let cancelled = Arc::new(AtomicBool::new(false));
    let worker_cancel = cancelled.clone();
    let input = tone(48000, 48000 * 20, 440.0);
    let worker = std::thread::spawn(move || {
        transpose_region(&input, 48000, 7.0, &worker_cancel)
            .map(|_| ())
            .map_err(|error| error.to_string())
    });
    std::thread::sleep(std::time::Duration::from_millis(5));
    cancelled.store(true, Ordering::Relaxed);
    let error = worker.join().unwrap().unwrap_err();
    assert!(error.contains("cancelled"), "{error}");
}

#[test]
fn unequal_stereo_tones_and_a_chord_keep_their_pitches() {
    let rate = 48000;
    let shift = -5.0;
    let ratio = 2.0_f64.powf(shift as f64 / 12.0);
    let input: Vec<_> = (0..rate)
        .map(|i| {
            let time = i as f64 / rate as f64;
            [
                (0.2 * (TAU * 440.0 * time).sin() + 0.15 * (TAU * 659.255 * time).sin()) as f32,
                (0.3 * (TAU * 110.0 * time).sin()) as f32,
            ]
        })
        .collect();
    let output = transpose(&input, rate, shift);
    // Direct frequency projections inspect both chord partials rather than
    // accepting a processor that retains only the dominant note.
    for (channel, frequency, input_amplitude) in
        [(0, 440.0, 0.2), (0, 659.255, 0.15), (1, 110.0, 0.3)]
    {
        let frequency = frequency * ratio;
        let segment = &output[12000..36000];
        let mut real = 0.0;
        let mut imag = 0.0;
        for (i, frame) in segment.iter().enumerate() {
            let angle = TAU * frequency * i as f64 / rate as f64;
            real += frame[channel] as f64 * angle.cos();
            imag += frame[channel] as f64 * angle.sin();
        }
        let measured_amplitude = 2.0 * real.hypot(imag) / segment.len() as f64;
        assert!(
            measured_amplitude > input_amplitude * 0.7,
            "channel {channel}, {frequency} Hz: amplitude {measured_amplitude}"
        );
    }
}

#[test]
fn changing_stronger_stereo_channel_preserves_the_phase_relationship() {
    let length = 48000;
    let input: Vec<_> = (0..length)
        .map(|i| {
            let phase = TAU * 433.7 * i as f64 / 48000.0;
            let mix = i as f64 / length as f64;
            [
                ((0.4 - 0.2 * mix) * phase.sin()) as f32,
                ((0.2 + 0.2 * mix) * (phase + 0.7).sin()) as f32,
            ]
        })
        .collect();
    let output = transpose(&input, 48000, 4.0);
    let expected = 433.7 * 2.0_f64.powf(4.0 / 12.0);
    for range in [8000..16000, 20000..28000, 32000..40000] {
        let mut phases = [0.0; 2];
        for channel in 0..2 {
            let mut real = 0.0;
            let mut imag = 0.0;
            for (i, frame) in output[range.clone()].iter().enumerate() {
                let angle = TAU * expected * i as f64 / 48000.0;
                real += frame[channel] as f64 * angle.cos();
                imag += frame[channel] as f64 * angle.sin();
            }
            phases[channel] = real.atan2(imag);
        }
        let difference =
            (phases[1] - phases[0] + std::f64::consts::PI).rem_euclid(TAU) - std::f64::consts::PI;
        assert!((difference - 0.7).abs() < 0.03, "stereo phase {difference}");
    }
}
