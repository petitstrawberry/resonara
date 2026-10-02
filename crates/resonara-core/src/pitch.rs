//! Offline, fixed-duration region transposition.
//!
//! A phase-locked phase vocoder stretches the audio, then a band-limited
//! resampler restores its original length. Stereo bins receive the same phase
//! rotation, retaining their relative phase rather than making two independent
//! mono effects. This is transposition, not formant preservation or note tuning.
//! The caller must run it on a worker, never on the audio callback or UI thread.

use crate::Result;
use rustfft::{FftPlannerScalar, num_complex::Complex};
use std::f64::consts::{PI, TAU};
use std::sync::atomic::{AtomicBool, Ordering};

pub const MAX_TRANSPOSE_SEMITONES: f32 = 12.0;
// Includes both the stretched audio/normalization and the returned audio. A
// finite work budget is especially important on Scarlet; longer recordings can
// be split into regions. The FFT workspace is at most another few megabytes.
const MAX_AUDIO_WORK_BYTES: usize = 512 * 1024 * 1024;
const SINC_HALF_WIDTH: isize = 48;
const SINC_PHASES: usize = 1024;
const SINC_TAPS: usize = SINC_HALF_WIDTH as usize * 2;

/// Transpose by at most one octave without changing the number of frames.
///
/// Input is stereo; duplicated mono remains duplicated mono. Zero semitones is
/// an exact bypass. The operation is cancellable and leaves its input untouched.
/// Large shifts can smear attacks and change vocal timbre; no formant correction
/// or detected-note editing is implied by this operation.
pub fn transpose_region(
    samples: &[[f32; 2]],
    sample_rate: u32,
    semitones: f32,
    cancel: &AtomicBool,
) -> Result<Vec<[f32; 2]>> {
    if !(8000..=192000).contains(&sample_rate) {
        return Err("Pitch shift requires a sample rate between 8000 and 192000 Hz".into());
    }
    if !semitones.is_finite() || semitones.abs() > MAX_TRANSPOSE_SEMITONES {
        return Err("Pitch shift must be between -12 and +12 semitones".into());
    }
    check_cancel(cancel)?;
    if samples
        .len()
        .checked_mul(std::mem::size_of::<[f32; 2]>())
        .is_none_or(|bytes| bytes > MAX_AUDIO_WORK_BYTES)
    {
        return Err(
            "Pitch shift region exceeds the 512 MiB work budget; split it into shorter regions"
                .into(),
        );
    }
    for chunk in samples.chunks(16_384) {
        check_cancel(cancel)?;
        if chunk.iter().flatten().any(|sample| !sample.is_finite()) {
            return Err("Pitch shift requires finite audio samples".into());
        }
    }
    if samples.is_empty() {
        return Ok(Vec::new());
    }
    if semitones == 0.0 {
        let mut output = allocate(samples.len(), [0.0; 2], cancel)?;
        for (dst, src) in output.chunks_mut(16_384).zip(samples.chunks(16_384)) {
            check_cancel(cancel)?;
            dst.copy_from_slice(src);
        }
        return Ok(output);
    }

    let ratio = 2.0_f64.powf(semitones as f64 / 12.0);
    render_pitch(samples, sample_rate, ratio, cancel)
}

fn render_pitch(
    samples: &[[f32; 2]],
    sample_rate: u32,
    ratio: f64,
    cancel: &AtomicBool,
) -> Result<Vec<[f32; 2]>> {
    // Roughly 85 ms provides useful bass resolution at every supported rate.
    // The scalar planner works on both Scarlet CPU architectures without
    // platform CPU-feature probing or external/native DSP libraries.
    let fft_size = ((sample_rate as usize * 85) / 1000)
        .next_power_of_two()
        .clamp(1024, 16_384);
    let hop = fft_size / 8;
    let half = fft_size / 2;
    let bins = half + 1;
    let padding = fft_size * 2;
    let stretched_frames = (samples.len() as f64 * ratio).ceil() as usize;
    let workspace_frames = stretched_frames
        .checked_add(padding * 2)
        .ok_or("Pitch shift region is too large")?;
    let audio_bytes = workspace_frames
        .checked_mul(std::mem::size_of::<[f32; 2]>() + std::mem::size_of::<f32>())
        .and_then(|n| {
            samples
                .len()
                .checked_mul(std::mem::size_of::<[f32; 2]>())
                .and_then(|bytes| n.checked_add(bytes))
        })
        .ok_or("Pitch shift region is too large")?;
    if audio_bytes > MAX_AUDIO_WORK_BYTES {
        return Err(
            "Pitch shift region exceeds the 512 MiB work budget; split it into shorter regions"
                .into(),
        );
    }
    let mut stretched = allocate(workspace_frames, [0.0_f32; 2], cancel)?;
    let mut weights = allocate(workspace_frames, 0.0_f32, cancel)?;
    let window: Vec<f32> = (0..fft_size)
        .map(|i| (0.5 - 0.5 * (TAU * i as f64 / fft_size as f64).cos()) as f32)
        .collect();
    let mut planner = FftPlannerScalar::<f32>::new();
    let forward = planner.plan_fft_forward(fft_size);
    let inverse = planner.plan_fft_inverse(fft_size);
    let mut scratch = vec![
        Complex::new(0.0, 0.0);
        forward
            .get_inplace_scratch_len()
            .max(inverse.get_inplace_scratch_len())
    ];
    let mut spectra = [
        vec![Complex::new(0.0, 0.0); fft_size],
        vec![Complex::new(0.0, 0.0); fft_size],
    ];
    let mut phase = vec![[0.0_f64; 2]; bins];
    let mut previous_phase = vec![[0.0_f64; 2]; bins];
    let mut synthesis_phase = vec![0.0_f64; bins];
    let mut reference = vec![0_usize; bins];
    let mut power = vec![0.0_f64; bins];
    let mut previous_power = vec![0.0_f64; bins];
    let mut peaks = Vec::with_capacity(bins);
    let mut nearest_peak = vec![0_usize; bins];
    let mut rotations = vec![0.0_f64; bins];
    let mut previous_center = None;
    let first_center = -(half as i64);
    let last_center = samples.len() as i64 + half as i64;

    // Map window CENTERS. Mapping their starts would shift the result by
    // (ratio - 1) * fft_size / 2, which is audible on region boundaries.
    for input_center in (first_center..=last_center).step_by(hop) {
        check_cancel(cancel)?;
        let output_center = (input_center as f64 * ratio).round() as i64;
        let output_hop = previous_center.map(|last| output_center - last);
        let input_start = input_center - half as i64;
        for channel in 0..2 {
            for (i, value) in spectra[channel].iter_mut().enumerate() {
                let index = input_start + i as i64;
                let sample = if index >= 0 {
                    samples.get(index as usize).map_or(0.0, |s| s[channel])
                } else {
                    0.0
                };
                *value = Complex::new(sample * window[i], 0.0);
            }
            forward.process_with_scratch(&mut spectra[channel], &mut scratch);
            if spectra[channel]
                .iter()
                .any(|value| !value.re.is_finite() || !value.im.is_finite())
            {
                return Err("Audio level exceeds the pitch processor's numeric range".into());
            }
        }
        let mut max_power = 0.0_f64;
        for bin in 0..bins {
            let left = spectra[0][bin];
            let right = spectra[1][bin];
            phase[bin] = [
                (left.im as f64).atan2(left.re as f64),
                (right.im as f64).atan2(right.re as f64),
            ];
            power[bin] = spectral_energy(left) + spectral_energy(right);
            max_power = max_power.max(power[bin]);
        }
        for bin in 0..bins {
            let ref_channel =
                usize::from(spectral_energy(spectra[1][bin]) > spectral_energy(spectra[0][bin]));
            if let Some(output_hop) = output_hop
                && previous_power[bin] > max_power * 1e-12
                && power[bin] > max_power * 1e-12
            {
                // When the stronger channel changes, rebase the accumulated
                // phase using the old frame before measuring its advance.
                synthesis_phase[bin] += wrap_phase(
                    previous_phase[bin][ref_channel] - previous_phase[bin][reference[bin]],
                );
                let nominal = TAU * bin as f64 / fft_size as f64;
                let deviation = wrap_phase(
                    phase[bin][ref_channel]
                        - previous_phase[bin][ref_channel]
                        - nominal * hop as f64,
                );
                let frequency = nominal + deviation / hop as f64;
                synthesis_phase[bin] =
                    wrap_phase(synthesis_phase[bin] + frequency * output_hop as f64);
            } else {
                synthesis_phase[bin] = phase[bin][ref_channel];
            }
            reference[bin] = ref_channel;
        }

        // Identity phase locking retains phase relationships within each
        // harmonic's spectral lobe. Use combined channel energy for peak
        // detection: an anti-phase stereo signal must not cancel the guide.
        peaks.clear();
        for bin in 0..bins {
            if power[bin] > max_power * 1e-12
                && (bin == 0 || power[bin] >= power[bin - 1])
                && (bin + 1 == bins || power[bin] > power[bin + 1])
            {
                peaks.push(bin);
            }
        }
        if peaks.is_empty() {
            for spectrum in &mut spectra {
                spectrum.fill(Complex::new(0.0, 0.0));
            }
        } else {
            let mut peak_index = 0;
            for (bin, nearest) in nearest_peak.iter_mut().enumerate() {
                while peak_index + 1 < peaks.len()
                    && bin * 2 > peaks[peak_index] + peaks[peak_index + 1]
                {
                    peak_index += 1;
                }
                *nearest = peaks[peak_index];
            }
            for bin in 0..bins {
                let peak = nearest_peak[bin];
                rotations[bin] = synthesis_phase[peak] - phase[peak][reference[peak]];
            }
            for bin in 0..bins {
                let rotation = rotations[bin];
                // Store the locked phase too, so a drifting harmonic can
                // move to the neighbouring bin without adopting an unrelated
                // phase accumulator there on the following frame.
                synthesis_phase[bin] = wrap_phase(phase[bin][reference[bin]] + rotation);
                let rotation = Complex::new(rotation.cos() as f32, rotation.sin() as f32);
                for spectrum in &mut spectra {
                    spectrum[bin] *= rotation;
                }
            }
            for spectrum in &mut spectra {
                spectrum[0].im = 0.0;
                spectrum[half].im = 0.0;
                for bin in 1..half {
                    spectrum[fft_size - bin] = spectrum[bin].conj();
                }
                inverse.process_with_scratch(spectrum, &mut scratch);
            }
        }
        let output_start = padding as i64 + output_center - half as i64;
        for i in 0..fft_size {
            let index = output_start + i as i64;
            if index < 0 || index as usize >= stretched.len() {
                continue;
            }
            let index = index as usize;
            let weight = window[i];
            weights[index] += weight * weight;
            for channel in 0..2 {
                stretched[index][channel] += spectra[channel][i].re * weight / fft_size as f32;
            }
        }
        previous_phase.copy_from_slice(&phase);
        previous_power.copy_from_slice(&power);
        previous_center = Some(output_center);
    }
    for (chunk, weight_chunk) in stretched.chunks_mut(16_384).zip(weights.chunks(16_384)) {
        check_cancel(cancel)?;
        for (frame, weight) in chunk.iter_mut().zip(weight_chunk) {
            if *weight > 1e-8 {
                frame[0] /= weight;
                frame[1] /= weight;
            } else {
                *frame = [0.0; 2];
            }
        }
    }
    drop(weights);

    let mut result = allocate(samples.len(), [0.0; 2], cancel)?;
    // Pitching up is downsampling. Narrow the low-pass filter accordingly so
    // content above the new Nyquist limit is removed, rather than folded back.
    let cutoff = ratio.recip().min(1.0) * 0.97;
    let kernels = sinc_kernels(cutoff);
    for (index, frame) in result.iter_mut().enumerate() {
        if index % 2048 == 0 {
            check_cancel(cancel)?;
        }
        let position = padding as f64 + index as f64 * ratio;
        let center = position.floor() as isize;
        let kernel_position = position.fract() * SINC_PHASES as f64;
        let kernel_index = kernel_position.floor() as usize;
        let kernel_fraction = kernel_position.fract();
        let first_tap = center - SINC_HALF_WIDTH + 1;
        let mut sum = [0.0_f64; 2];
        for offset in 0..SINC_TAPS {
            let tap = first_tap + offset as isize;
            let lower = kernels[kernel_index][offset];
            let upper = kernels[kernel_index + 1][offset];
            let weight = lower + (upper - lower) * kernel_fraction;
            if tap >= 0
                && let Some(source) = stretched.get(tap as usize)
            {
                sum[0] += source[0] as f64 * weight;
                sum[1] += source[1] as f64 * weight;
            }
        }
        for channel in 0..2 {
            frame[channel] = sum[channel] as f32;
            if !frame[channel].is_finite() {
                return Err("Pitch shift produced non-finite audio".into());
            }
        }
    }
    check_cancel(cancel)?;
    Ok(result)
}

fn sinc_kernels(cutoff: f64) -> Vec<[f64; SINC_TAPS]> {
    // Interpolated polyphase coefficients avoid millions of trigonometric
    // calls on long regions while preserving fractional/cents transposition.
    (0..=SINC_PHASES)
        .map(|phase| {
            let mut kernel = [0.0; SINC_TAPS];
            let fraction = phase as f64 / SINC_PHASES as f64;
            for (offset, weight) in kernel.iter_mut().enumerate() {
                let distance = offset as f64 - SINC_HALF_WIDTH as f64 + 1.0 - fraction;
                let window_position = distance / SINC_HALF_WIDTH as f64;
                let window = 0.42
                    + 0.5 * (PI * window_position).cos()
                    + 0.08 * (TAU * window_position).cos();
                let angle = PI * cutoff * distance;
                let sinc = if angle.abs() < 1e-12 {
                    cutoff
                } else {
                    cutoff * angle.sin() / angle
                };
                *weight = window * sinc;
            }
            let weight_sum: f64 = kernel.iter().sum();
            for weight in &mut kernel {
                *weight /= weight_sum;
            }
            kernel
        })
        .collect()
}

fn wrap_phase(phase: f64) -> f64 {
    (phase + PI).rem_euclid(TAU) - PI
}

fn spectral_energy(value: Complex<f32>) -> f64 {
    value.re as f64 * value.re as f64 + value.im as f64 * value.im as f64
}

fn check_cancel(cancel: &AtomicBool) -> Result<()> {
    if cancel.load(Ordering::Relaxed) {
        Err("Pitch shift cancelled".into())
    } else {
        Ok(())
    }
}

fn allocate<T: Clone>(length: usize, value: T, cancel: &AtomicBool) -> Result<Vec<T>> {
    let mut buffer = Vec::new();
    buffer
        .try_reserve_exact(length)
        .map_err(|_| "Not enough memory for pitch shift")?;
    while buffer.len() < length {
        check_cancel(cancel)?;
        buffer.resize((buffer.len() + 16_384).min(length), value.clone());
    }
    Ok(buffer)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unity_vocoder_reconstructs_phase_and_amplitude_without_public_bypass() {
        let source: Vec<_> = (0..24_017)
            .map(|i| {
                [
                    (TAU * 731.3 * i as f64 / 48000.0).sin() as f32 * 0.3,
                    (TAU * 731.3 * i as f64 / 48000.0 + 0.72).sin() as f32 * 0.4,
                ]
            })
            .collect();
        let output = render_pitch(&source, 48000, 1.0, &AtomicBool::new(false)).unwrap();
        let max_error = source[256..source.len() - 256]
            .iter()
            .zip(&output[256..output.len() - 256])
            .flat_map(|(a, b)| [(a[0] - b[0]).abs(), (a[1] - b[1]).abs()])
            .fold(0.0_f32, f32::max);
        assert!(max_error < 3e-5, "unity reconstruction error {max_error}");
    }
}
