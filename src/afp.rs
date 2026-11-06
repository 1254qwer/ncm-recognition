use rustfft::{num_complex::Complex, FftPlanner};

const SAMPLE_RATE: u32 = 8_000;
const FFT_LEN: usize = 2_048;
const HOP: usize = 160;
const PAD_LEFT: usize = FFT_LEN / 2;
const LOW_BIN: usize = 25;
const HIGH_BIN: usize = 1_024;
const USED_BINS: usize = HIGH_BIN - LOW_BIN;
const LOG_FLOOR: f32 = f32::from_bits(0x3596_37bd);
const ENERGY_THRESHOLD: f32 = 2.0;
const SIGN_FACTOR: f32 = 1.0;
const MEAN_RADIUS_F: usize = 10;
const MEAN_RADIUS_T: usize = 5;

#[derive(Clone, Debug, PartialEq)]
pub struct Peak {
    pub frequency_bin: i32,
    pub time_frame: i32,
    pub score: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AfpResult {
    pub normal: Vec<Peak>,
    pub dense: Vec<Peak>,
    pub duration_seconds: f32,
}

pub fn extract(samples: &[f32], sample_rate: u32) -> Result<AfpResult, String> {
    if sample_rate != SAMPLE_RATE {
        return Err("AFP requires already-resampled 8000 Hz mono float PCM".into());
    }
    if samples.iter().any(|x| !x.is_finite()) {
        return Err("AFP PCM contains a non-finite sample".into());
    }
    let duration_seconds = samples.len() as f32 / SAMPLE_RATE as f32;
    let frames = frame_count(samples.len());
    if frames <= 9 {
        return Ok(AfpResult {
            normal: Vec::new(),
            dense: Vec::new(),
            duration_seconds,
        });
    }
    if frames > i32::MAX as usize {
        return Err("AFP time-frame index exceeds the wire representation".into());
    }
    let spectrum = centered_log_spectrum(samples, frames)?;
    let candidates = find_candidates(&spectrum, frames);
    let significant: Vec<(usize, usize)> = candidates
        .into_iter()
        .filter(|&(f, t)| survives_silence_filter(&spectrum, frames, f, t))
        .collect();
    Ok(AfpResult {
        normal: remove_adjacent(&spectrum, frames, &significant, 30, 8),
        dense: remove_adjacent(&spectrum, frames, &significant, 20, 4),
        duration_seconds,
    })
}

fn frame_count(samples: usize) -> usize {
    if samples == 0 {
        0
    } else {
        samples / HOP + 1
    }
}

fn hamming_window() -> Vec<f32> {
    // Round the window coefficient to f32 before evaluating in f64.
    let alpha = f32::from_bits(0x3f0a_3d71) as f64;
    (0..FFT_LEN)
        .map(|i| {
            (alpha
                - (1.0 - alpha) * (std::f64::consts::TAU * i as f64 / (FFT_LEN - 1) as f64).cos())
                as f32
        })
        .collect()
}

fn centered_log_spectrum(samples: &[f32], frames: usize) -> Result<Vec<f32>, String> {
    let len = USED_BINS
        .checked_mul(frames)
        .ok_or_else(|| "AFP spectrum size overflow".to_string())?;
    let mut spectrum = vec![0.0; len];
    let window = hamming_window();
    let mut planner = FftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(FFT_LEN);
    let mut block = vec![Complex::new(0.0, 0.0); FFT_LEN];
    let mut scratch = vec![Complex::new(0.0, 0.0); fft.get_inplace_scratch_len()];
    for t in 0..frames {
        let center = t * HOP;
        for (j, value) in block.iter_mut().enumerate() {
            let padded_index = center + j;
            let sample = if padded_index >= PAD_LEFT {
                samples.get(padded_index - PAD_LEFT).copied().unwrap_or(0.0)
            } else {
                0.0
            };
            *value = Complex::new(sample * window[j], 0.0);
        }
        fft.process_with_scratch(&mut block, &mut scratch);
        for f in LOW_BIN..HIGH_BIN {
            let z = block[f];
            let power = z.re * z.re + z.im * z.im;
            spectrum[(f - LOW_BIN) * frames + t] = power.sqrt().max(LOG_FLOOR).ln();
        }
    }
    let mut sum = 0.0_f64;
    for &x in &spectrum {
        sum += x as f64;
    }
    let mean = (sum / spectrum.len() as f64) as f32;
    for x in &mut spectrum {
        *x -= mean;
    }
    Ok(spectrum)
}

fn bounds(index: usize, radius: usize, size: usize) -> (usize, usize) {
    (index.saturating_sub(radius), (index + radius + 1).min(size))
}

fn is_local_max(
    spectrum: &[f32],
    frames: usize,
    f: usize,
    t: usize,
    radius_f: usize,
    radius_t: usize,
) -> bool {
    let target = spectrum[f * frames + t];
    let (f0, f1) = bounds(f, radius_f, USED_BINS);
    let (t0, t1) = bounds(t, radius_t, frames);
    for row in f0..f1 {
        for col in t0..t1 {
            if spectrum[row * frames + col] > target {
                return false;
            }
        }
    }
    true
}

fn find_candidates(spectrum: &[f32], frames: usize) -> Vec<(usize, usize)> {
    let mut candidates = Vec::new();
    for f in 0..USED_BINS {
        let mut t = 0;
        while t < frames {
            if is_local_max(spectrum, frames, f, t, 1, 1) {
                candidates.push((f, t));
                // Skip the next time cell after accepting a plateau maximum.
                t += 2;
            } else {
                t += 1;
            }
        }
    }
    candidates
}

fn survives_silence_filter(spectrum: &[f32], frames: usize, f: usize, t: usize) -> bool {
    let peak = spectrum[f * frames + t];
    if peak <= 0.0 {
        return false;
    }
    let (f0, f1) = bounds(f, MEAN_RADIUS_F, USED_BINS);
    let (t0, t1) = bounds(t, MEAN_RADIUS_T, frames);
    let mut sum = 0.0_f32;
    for row in f0..f1 {
        for col in t0..t1 {
            sum += spectrum[row * frames + col];
        }
    }
    let mean = sum / ((f1 - f0) * (t1 - t0)) as f32;
    mean > ENERGY_THRESHOLD || peak - mean > SIGN_FACTOR
}

fn remove_adjacent(
    spectrum: &[f32],
    frames: usize,
    candidates: &[(usize, usize)],
    radius_f: usize,
    radius_t: usize,
) -> Vec<Peak> {
    let mut peaks: Vec<Peak> = candidates
        .iter()
        .copied()
        .filter(|&(f, t)| is_local_max(spectrum, frames, f, t, radius_f, radius_t))
        .map(|(f, t)| Peak {
            frequency_bin: (f + LOW_BIN) as i32,
            time_frame: t as i32,
            score: spectrum[f * frames + t],
        })
        .collect();
    peaks.sort_unstable_by_key(|p| (p.time_frame, p.frequency_bin));
    peaks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn centered_frame_count() {
        assert_eq!(frame_count(0), 0);
        assert_eq!(frame_count(159), 1);
        assert_eq!(frame_count(160), 2);
        assert_eq!(frame_count(16_000), 101);
    }

    #[test]
    fn unsupported_rate_is_not_silently_resampled() {
        assert!(extract(&[0.0; 160], 16_000).is_err());
    }

    #[test]
    fn symmetric_hamming_uses_float_alpha() {
        let w = hamming_window();
        let edge = (2.0 * (f32::from_bits(0x3f0a_3d71) as f64) - 1.0) as f32;
        assert_eq!(w[0], edge);
        assert_eq!(w[FFT_LEN - 1], edge);
        for i in 0..FFT_LEN {
            assert!((w[i] - w[FFT_LEN - 1 - i]).abs() < 1e-7);
        }
    }

    #[test]
    fn silence_and_short_slices_have_no_peaks() {
        for samples in [vec![], vec![0.0; 1_439], vec![0.0; 8_000]] {
            let result = extract(&samples, 8_000).unwrap();
            assert!(result.normal.is_empty());
            assert!(result.dense.is_empty());
            assert_eq!(result.duration_seconds, samples.len() as f32 / 8_000.0);
        }
    }

    #[test]
    fn plateau_candidate_scan_skips_the_next_time_cell() {
        let frames = 5;
        let matrix = vec![0.0; USED_BINS * frames];
        let candidates = find_candidates(&matrix, frames);
        assert_eq!(&candidates[..3], &[(0, 0), (0, 2), (0, 4)]);
        assert_eq!(candidates.len(), USED_BINS * 3);
    }

    #[test]
    fn energy_gate_uses_strict_thresholds_and_original_matrix_nms() {
        let frames = 3;
        let mut matrix = vec![0.0; USED_BINS * frames];
        matrix[40 * frames + 1] = 1.0;
        assert!(!survives_silence_filter(&matrix, frames, 40, 1));
        matrix[40 * frames + 1] = 3.0;
        assert!(survives_silence_filter(&matrix, frames, 40, 1));
        matrix[45 * frames + 1] = 4.0;
        assert!(remove_adjacent(&matrix, frames, &[(40, 1)], 30, 8).is_empty());
    }
}
