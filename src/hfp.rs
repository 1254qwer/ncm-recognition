use rustfft::{num_complex::Complex, FftPlanner};
use std::sync::Arc;

const RATE: u32 = 16_000;
const BLOCK: usize = 2_048;
const HALF: usize = BLOCK / 2;
const HOP: usize = 160;

#[derive(Debug, Clone)]
struct Note {
    start_ms: i32,
    duration_ms: i32,
    pitch: f32,
}

pub fn extract(samples: &[f32], sample_rate: u32) -> Result<Vec<i16>, String> {
    if sample_rate != RATE {
        return Err("HFP requires 16000 Hz mono PCM".into());
    }
    if samples.iter().any(|x| !x.is_finite()) {
        return Err("HFP PCM contains non-finite samples".into());
    }
    if samples.is_empty() {
        return Ok(Vec::new());
    }
    let mut yin = Yin::new();
    let mut frames = Vec::new();
    // Tail windows are right-padded; the first window is not centered.
    for start in (0..samples.len()).step_by(HOP) {
        let mut block = [0.0; BLOCK];
        let count = BLOCK.min(samples.len() - start);
        block[..count].copy_from_slice(&samples[start..start + count]);
        frames.push(yin.candidates(&block));
    }
    let frequencies = crate::hfp_hmm::track(&frames);
    let notes = frequencies
        .iter()
        .enumerate()
        .filter_map(|(frame, &frequency)| {
            let frequency = frequency as f32;
            if !(27.5..=4186.0).contains(&frequency) {
                return None;
            }
            // Keep this rounded logarithm divisor and f32 calculation.
            let pitch = 69.0 + 12.0 * (frequency / 440.0).ln() / 0.69315;
            let start_sample = (frame + 1) * HOP + BLOCK / 4;
            let secs = start_sample / RATE as usize;
            let nanos = ((start_sample % RATE as usize) as f64 * 1e9 / RATE as f64).round() as u32;
            let millis = secs as f32 * 1000.0 + nanos as f32 / 1_000_000.0;
            let start_ms = millis.floor() as i32;
            let end_ms = (millis + HOP as f32 * 1000.0 / RATE as f32).floor() as i32;
            Some(Note {
                start_ms,
                duration_ms: end_ms - start_ms,
                pitch,
            })
        })
        .collect::<Vec<_>>();
    notes_to_grid(&notes)
}

fn notes_to_grid(notes: &[Note]) -> Result<Vec<i16>, String> {
    let Some(last) = notes.last() else {
        return Ok(vec![]);
    };
    let end = last
        .start_ms
        .checked_add(last.duration_ms)
        .ok_or("note time overflow")?;
    if end < 0 {
        return Err("negative HFP note end".into());
    }
    let mut grid = vec![0i16; (end / 10) as usize];
    for note in notes {
        if !note.pitch.is_finite() || note.pitch < 0.00000011921 {
            return Err("invalid positive note pitch".into());
        }
        if note.start_ms < 0 || note.duration_ms < 0 {
            return Err("negative note time".into());
        }
        if note.duration_ms < 10 {
            continue;
        }
        let start = (note.start_ms / 10) as usize;
        let length = (note.duration_ms / 10).max(1) as usize;
        let target = grid
            .get_mut(start..start + length)
            .ok_or("note exceeds final grid extent")?;
        target.fill((note.pitch * 100.0) as i32 as i16);
    }
    Ok(grid)
}

struct Yin {
    forward: Arc<dyn rustfft::Fft<f32>>,
    inverse: Arc<dyn rustfft::Fft<f32>>,
    prior: [f32; 100],
}
impl Yin {
    fn new() -> Self {
        let mut planner = FftPlanner::new();
        Self {
            forward: planner.plan_fft_forward(BLOCK),
            inverse: planner.plan_fft_inverse(BLOCK),
            prior: threshold_prior(),
        }
    }
    fn candidates(&mut self, input: &[f32; BLOCK]) -> Vec<(f64, f64)> {
        let mut cmnd = self.difference(input);
        cmnd[0] = 1.0;
        let mut sum = 0.0;
        for (tau, value) in cmnd.iter_mut().enumerate().skip(1) {
            let diff = *value;
            sum += diff;
            *value = if sum == 0.0 {
                1.0
            } else {
                diff * (tau as f64 / sum)
            };
        }
        let probabilities = probabilistic_thresholds(&cmnd, &self.prior);
        let mut energy = 0.0f64;
        for &sample in input {
            energy += (sample * sample) as f64;
        }
        let rms = (energy / BLOCK as f64).sqrt();
        let low_amp = 0.01f32 as f64;
        let scale = if rms < low_amp {
            (rms + low_amp * 0.01) / (low_amp * 1.01)
        } else {
            1.0
        };
        probabilities
            .iter()
            .enumerate()
            .filter(|&(_, &p)| p > 0.0)
            .map(|(tau, &probability)| {
                (RATE as f64 / parabolic_tau(&cmnd, tau), probability * scale)
            })
            .collect()
    }
    // FFT uses f32; spectral products and energy sums use f64.
    fn difference(&self, input: &[f32; BLOCK]) -> Vec<f64> {
        let mut a: Vec<_> = input.iter().map(|&x| Complex::new(x, 0.0)).collect();
        let mut b = vec![Complex::new(0.0, 0.0); BLOCK];
        for i in 0..HALF {
            b[i].re = input[HALF - 1 - i];
        }
        self.forward.process(&mut a);
        self.forward.process(&mut b);
        for (x, y) in a.iter_mut().zip(&b) {
            let re = x.re as f64 * y.re as f64 - x.im as f64 * y.im as f64;
            let im = x.re as f64 * y.im as f64 + x.im as f64 * y.re as f64;
            *x = Complex::new(re as f32, im as f32);
        }
        self.inverse.process(&mut a);
        let mut power = vec![0.0f64; HALF];
        for &x in &input[..HALF] {
            power[0] += (x as f64) * (x as f64);
        }
        for tau in 1..HALF {
            let old = input[tau - 1] as f64;
            let new = input[tau + HALF] as f64;
            power[tau] = power[tau - 1] - old * old + new * new;
        }
        (0..HALF)
            .map(|tau| {
                let cross = a[HALF - 1 + tau].re as f64 / BLOCK as f64;
                power[0] + power[tau] - (cross + cross)
            })
            .collect()
    }
}

fn threshold_prior() -> [f32; 100] {
    let beta = 2.0f64 / 0.15 - 2.0;
    let raw: [f64; 100] = std::array::from_fn(|i| {
        let x = (i + 1) as f64 / 100.0;
        x * (1.0 - x).powf(beta - 1.0)
    });
    let sum: f64 = raw.iter().sum();
    std::array::from_fn(|i| ((raw[i] / sum * 1e6).round() / 1e6) as f32)
}

fn probabilistic_thresholds(cmnd: &[f64], prior: &[f32; 100]) -> Vec<f64> {
    let mut probability = vec![0.0; cmnd.len()];
    let threshold: [f32; 100] = std::array::from_fn(|i| (i as f64 * 0.01 + 0.01) as f32);
    let mut best = 0usize;
    let mut best_value = 42.0f32;
    let mut total = 0.0f32;
    let mut tau = 2;
    while tau + 1 < cmnd.len() {
        if cmnd[tau] >= threshold[99] as f64 || cmnd[tau + 1] >= cmnd[tau] {
            tau += 1;
            continue;
        }
        let mut previous = cmnd[tau];
        let mut minimum = cmnd.len() - 1;
        loop {
            tau += 1;
            if tau >= cmnd.len() {
                break;
            }
            let value = cmnd[tau];
            if value >= previous {
                minimum = tau - 1;
                break;
            }
            previous = value;
        }
        if minimum > 2 && cmnd[minimum] < best_value as f64 {
            best = minimum;
            best_value = cmnd[minimum] as f32;
        }
        if cmnd[minimum] < threshold[99] as f64 {
            for k in (0..100).rev() {
                probability[minimum] += prior[k] as f64;
                if k == 0 || cmnd[minimum] >= threshold[k - 1] as f64 {
                    break;
                }
            }
        }
        total += probability[minimum] as f32;
    }
    let best_probability = probability[best];
    if best_probability <= 1.0 {
        let mut remainder = 0.01;
        if total > 0.0 {
            // Later slots use the updated best probability; keep this loop in place.
            probability[2] = probability[2] / total as f64 * best_probability;
            let mut unvoiced = 1.0 - probability[2];
            for i in 3..probability.len() {
                probability[i] = probability[i] / total as f64 * probability[best];
                unvoiced -= probability[i];
            }
            remainder = unvoiced * 0.01;
        }
        if best >= 1 {
            probability[best] += remainder;
        }
    }
    probability
}

fn parabolic_tau(cmnd: &[f64], tau: usize) -> f64 {
    if tau == 0 || tau + 1 >= cmnd.len() {
        return tau as f64;
    }
    let y = cmnd[tau] as f32;
    let right = cmnd[tau + 1] as f32;
    let left = cmnd[tau - 1] as f32;
    let a = ((y + y) - right) - left;
    let delta = ((right - left) / (a + a)) as f64;
    tau as f64 + if delta.abs() > 1.0 { 0.0 } else { delta }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sine440_preserves_subharmonic() {
        let samples: Vec<f32> = (0..64_000)
            .map(|i| {
                let value = 0.15 * (std::f64::consts::TAU * 440.0 * i as f64 / 16_000.0).sin();
                (value * 32767.0).round() as i16 as f32 / 32768.0
            })
            .collect();
        let result = extract(&samples, RATE).unwrap();
        let mut expected = vec![0; 4];
        expected.extend(vec![3531; 398]);
        assert_eq!(result, expected);
    }

    #[test]
    fn prior_coefficients() {
        let p = threshold_prior();
        assert_eq!(
            &p[..4],
            &[0.012614f32, 0.022715f32, 0.030646f32, 0.036712f32]
        );
        assert_eq!(p[8], 0.047528f32);
        assert_eq!(p[99], 0.0);
    }
    #[test]
    fn silence_has_no_yin_candidates() {
        assert!(Yin::new().candidates(&[0.; BLOCK]).is_empty());
    }
    #[test]
    fn harmonic_has_fundamental_candidate() {
        let block = std::array::from_fn(|i| {
            (std::f64::consts::TAU * 220. * i as f64 / RATE as f64).sin() as f32 * 0.3
        });
        let c = Yin::new().candidates(&block);
        assert!(c.iter().any(|&(f, p)| (f - 220.).abs() < 1. && p > 0.));
    }
    #[test]
    fn grid_uses_last_note_extent_and_truncates_pitch() {
        let notes = vec![
            Note {
                start_ms: 32,
                duration_ms: 10,
                pitch: 69.009,
            },
            Note {
                start_ms: 62,
                duration_ms: 10,
                pitch: 70.999,
            },
        ];
        let grid = notes_to_grid(&notes).unwrap();
        assert_eq!(grid, vec![0, 0, 0, 6900, 0, 0, 7099]);
    }
}
