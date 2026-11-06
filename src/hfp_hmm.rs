use std::collections::VecDeque;

const PITCH_BINS: usize = 345;
const STATES: usize = PITCH_BINS * 2;
const MIN_HZ: f64 = 61.735;
const BINS_PER_OCTAVE: f64 = 60.0;
const TRANSITION_RADIUS: usize = 5;
const STAY_VOICING: f64 = 0.99;
const OBSERVATION_VOICED_FACTOR: f64 = 0.5;
const FIXED_LAG: usize = 100;

type Candidate = (f64, f64);

#[derive(Clone, Copy, Debug)]
struct Edge {
    from: usize,
    to: usize,
    probability: f64,
}

// For more than 100 frames, omit the first frame and repeat the flush boundary.
pub fn track(frames: &[Vec<(f64, f64)>]) -> Vec<f64> {
    if frames.is_empty() {
        return Vec::new();
    }
    let centers = pitch_centers();
    let mut hmm = SparseHmm::new();
    let mut output = Vec::with_capacity(frames.len());
    for (index, frame) in frames.iter().enumerate() {
        let observation = observations(frame, &centers);
        if index == 0 {
            hmm.initialize(&observation);
        } else {
            hmm.update(&observation);
        }
        if index >= FIXED_LAG {
            let source_frame = index + 1 - FIXED_LAG;
            let path = hmm.path();
            output.push(selected_frequency(path[0], &frames[source_frame], &centers));
        }
    }
    let first = frames.len().saturating_sub(FIXED_LAG);
    for (offset, state) in hmm.path().into_iter().enumerate() {
        output.push(selected_frequency(state, &frames[first + offset], &centers));
    }
    output
}

fn pitch_centers() -> Vec<f64> {
    (0..PITCH_BINS)
        .map(|i| (i as f64 / BINS_PER_OCTAVE).exp2() * MIN_HZ)
        .collect()
}

fn observations(candidates: &[Candidate], centers: &[f64]) -> Vec<f64> {
    let mut obs = vec![0.0; STATES];
    let mut accepted_mass = 0.0;
    for &(frequency, probability) in candidates {
        if frequency <= MIN_HZ {
            continue;
        }
        let mut previous_distance = 1000.0;
        for bin in 0..PITCH_BINS {
            let distance = (frequency - centers[bin]).abs();
            if bin != 0 && previous_distance < distance {
                // Later candidates replace this bin, but both contribute to the mass.
                obs[bin - 1] = probability;
                accepted_mass += probability;
                break;
            }
            previous_distance = distance;
        }
    }
    let voiced_mass = accepted_mass * OBSERVATION_VOICED_FACTOR;
    let scale = voiced_mass / accepted_mass;
    let unvoiced = (1.0 - voiced_mass) / PITCH_BINS as f64;
    for bin in 0..PITCH_BINS {
        if accepted_mass > 0.0 {
            obs[bin] *= scale;
        }
        obs[bin + PITCH_BINS] = unvoiced;
    }
    obs
}

fn transitions() -> Vec<Edge> {
    let mut edges = Vec::with_capacity(15_060);
    for source in 0..PITCH_BINS {
        let start = source.saturating_sub(TRANSITION_RADIUS);
        let end = (source + TRANSITION_RADIUS + 1).min(PITCH_BINS);
        let mut weight_sum = 0.0;
        for target in start..end {
            weight_sum += (TRANSITION_RADIUS + 1 - source.abs_diff(target)) as f64;
        }
        for target in start..end {
            let weight = (TRANSITION_RADIUS + 1 - source.abs_diff(target)) as f64;
            let same = weight / weight_sum * STAY_VOICING;
            let change = weight / weight_sum * (1.0 - STAY_VOICING);
            // Equal-probability paths keep the first edge; order matters.
            edges.push(Edge {
                from: source,
                to: target,
                probability: same,
            });
            edges.push(Edge {
                from: source,
                to: target + PITCH_BINS,
                probability: change,
            });
            edges.push(Edge {
                from: source + PITCH_BINS,
                to: target + PITCH_BINS,
                probability: same,
            });
            edges.push(Edge {
                from: source + PITCH_BINS,
                to: target,
                probability: change,
            });
        }
    }
    edges
}

struct SparseHmm {
    edges: Vec<Edge>,
    delta: Vec<f64>,
    predecessor: VecDeque<Vec<usize>>,
}

impl SparseHmm {
    fn new() -> Self {
        Self {
            edges: transitions(),
            delta: vec![0.0; STATES],
            predecessor: VecDeque::new(),
        }
    }

    fn initialize(&mut self, observation: &[f64]) {
        self.predecessor.clear();
        let mut sum = 0.0;
        for (state, value) in self.delta.iter_mut().enumerate() {
            *value = (PITCH_BINS as f64 * 0.5) * observation[state];
            sum += *value;
        }
        for value in &mut self.delta {
            *value /= sum;
        }
        self.predecessor.push_back(vec![0; STATES]);
    }

    fn update(&mut self, observation: &[f64]) {
        let mut next = vec![0.0; STATES];
        let mut from = vec![0; STATES];
        for edge in &self.edges {
            let score = edge.probability * self.delta[edge.from];
            if score > next[edge.to] {
                next[edge.to] = score;
                from[edge.to] = edge.from;
            }
        }
        self.predecessor.push_back(from);
        let mut sum = 0.0;
        for state in 0..STATES {
            next[state] *= observation[state];
            sum += next[state];
        }
        if sum > 0.0 {
            for value in &mut next {
                *value /= sum;
            }
        } else {
            next.fill(1.0 / STATES as f64);
        }
        self.delta = next;
        if self.predecessor.len() > FIXED_LAG {
            self.predecessor.pop_front();
        }
    }

    fn path(&self) -> Vec<usize> {
        let mut path = vec![STATES - 1; self.predecessor.len()];
        if path.is_empty() {
            return path;
        }
        let mut best = 0.0;
        let last = path.len() - 1;
        for (state, &score) in self.delta.iter().enumerate() {
            if score > best {
                best = score;
                path[last] = state;
            }
        }
        for t in (1..path.len()).rev() {
            path[t - 1] = self.predecessor[t][path[t]];
        }
        path
    }
}

fn selected_frequency(state: usize, candidates: &[Candidate], centers: &[f64]) -> f64 {
    let center = if state < PITCH_BINS {
        centers[state]
    } else {
        -centers[state - PITCH_BINS]
    } as f32;
    if center <= 0.0 {
        return center as f64;
    }
    let mut selected = 0.0_f32;
    let mut minimum_distance = 10000.0_f64;
    for &(frequency, _) in candidates {
        let candidate = frequency as f32;
        let distance = (center - candidate).abs();
        if (distance as f64) < minimum_distance {
            minimum_distance = distance as f64;
            selected = candidate;
        }
    }
    selected as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_grid_and_transition_probabilities() {
        let centers = pitch_centers();
        assert_eq!(centers.len(), 345);
        assert_eq!(centers[0], 61.735);
        assert_eq!(centers[60], 123.47);
        let edges = transitions();
        assert_eq!(edges.len(), 15_060);
        for source in 0..STATES {
            let mass: f64 = edges
                .iter()
                .filter(|e| e.from == source)
                .map(|e| e.probability)
                .sum();
            assert!((mass - 1.0).abs() < 1e-14);
        }
        assert_eq!(edges[0].from, 0);
        assert_eq!(edges[0].to, 0);
        assert_eq!(edges[0].probability, 6.0 / 21.0 * 0.99);
        assert_eq!((edges[1].from, edges[1].to), (0, 345));
        assert_eq!((edges[2].from, edges[2].to), (345, 345));
        assert_eq!((edges[3].from, edges[3].to), (345, 0));
    }

    #[test]
    fn observation_overwrite_mass_and_boundary_rules() {
        let centers = pitch_centers();
        let obs = observations(&[(centers[100], 0.3), (centers[100], 0.5)], &centers);
        assert_eq!(obs[100], 0.25);
        assert_eq!(obs[345], (1.0 - 0.8 * 0.5) / 345.0);
        let ignored = observations(&[(MIN_HZ, 1.0), (centers[344], 1.0)], &centers);
        assert!(ignored[..345].iter().all(|&x| x == 0.0));
        assert!(ignored[345..].iter().all(|&x| x == 1.0 / 345.0));
    }

    #[test]
    fn selected_frequency_uses_f32_precision() {
        let centers = pitch_centers();
        let frequency = centers[100] + 0.123456789;
        assert_eq!(
            selected_frequency(100, &[(frequency, 0.1)], &centers),
            frequency as f32 as f64
        );
        assert_eq!(
            selected_frequency(445, &[], &centers),
            -centers[100] as f32 as f64
        );
        assert_eq!(selected_frequency(100, &[], &centers), 0.0);
    }

    #[test]
    fn no_voiced_candidates_produce_negative_frequencies() {
        assert!(track(&[]).is_empty());
        for count in [1, 100, 101, 150] {
            let result = track(&vec![vec![]; count]);
            assert_eq!(result.len(), count);
            assert!(result.iter().all(|&x| x < 0.0));
        }
    }

    #[test]
    fn fixed_lag_flush_boundary_duplicate() {
        let frames: Vec<_> = (0..105)
            .map(|i| vec![(220.0 + i as f64 * 0.0001, 1.0)])
            .collect();
        let result = track(&frames);
        assert_eq!(result.len(), frames.len());
        let expected_indices: Vec<_> = (1..=5).chain(5..105).collect();
        for (output, source) in result.iter().zip(expected_indices) {
            assert_eq!(*output, frames[source][0].0 as f32 as f64);
        }
    }
}
