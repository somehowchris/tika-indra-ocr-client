//! Paired document-cluster percentile bootstrap for a fixed selected corpus.
//! Each draw resamples whole document clusters, using the same draw for both
//! variants. Repeated OCR attempts never enter these counts.

use super::ScoreError;
use serde::Serialize;

#[derive(Default)]
pub(super) struct Cluster {
    pub reference_units: [u64; 3],
    pub left_errors: [u64; 3],
    pub right_errors: [u64; 3],
    pub cases: [usize; 3],
}

#[derive(Debug, Default, Serialize)]
pub(super) struct Estimate {
    pub paired_cases: usize,
    pub document_clusters: usize,
    pub reference_units: u64,
    pub left_errors: u64,
    pub right_errors: u64,
    pub left_rate: Option<f64>,
    pub right_rate: Option<f64>,
    pub right_minus_left: Option<f64>,
    pub ci_95_low: Option<f64>,
    pub ci_95_high: Option<f64>,
    pub bootstrap_samples: usize,
    pub undefined_bootstrap_samples: usize,
}

pub(super) struct Workspace {
    samples: [f64; 10_000],
}

impl Default for Workspace {
    fn default() -> Self {
        Self {
            samples: [0.0; 10_000],
        }
    }
}

impl Workspace {
    pub(super) fn estimate(
        &mut self,
        clusters: &[Cluster],
        seed: u64,
        resamples: usize,
    ) -> Result<[Estimate; 3], ScoreError> {
        if !(1000..=self.samples.len()).contains(&resamples) || clusters.len() > 128 {
            return Err(ScoreError::Invalid(
                "bootstrap requires 1000..10000 resamples and at most 128 document clusters",
            ));
        }
        let mut result: [Estimate; 3] = std::array::from_fn(|_| Estimate::default());
        for (metric, estimate) in result.iter_mut().enumerate() {
            let mut eligible = [0_usize; 128];
            let mut count = 0;
            for (index, cluster) in clusters
                .iter()
                .enumerate()
                .filter(|(_, cluster)| cluster.cases[metric] != 0)
            {
                eligible[count] = index;
                count += 1;
                estimate.paired_cases += cluster.cases[metric];
                estimate.reference_units += cluster.reference_units[metric];
                estimate.left_errors += cluster.left_errors[metric];
                estimate.right_errors += cluster.right_errors[metric];
            }
            estimate.document_clusters = count;
            if estimate.reference_units == 0 {
                continue;
            }
            estimate.left_rate =
                Some(estimate.left_errors as f64 / estimate.reference_units as f64);
            estimate.right_rate =
                Some(estimate.right_errors as f64 / estimate.reference_units as f64);
            estimate.right_minus_left = Some(
                (estimate.right_errors as f64 - estimate.left_errors as f64)
                    / estimate.reference_units as f64,
            );
            // One observed cluster provides no empirical between-cluster
            // variability. A zero-width resampling interval would mislead.
            if count < 2 {
                continue;
            }
            let mut rng = seed;
            let mut defined = 0;
            for _ in 0..resamples {
                let mut reference = 0_u64;
                let mut left = 0_u64;
                let mut right = 0_u64;
                for _ in 0..count {
                    let cluster = &clusters[eligible[sample(&mut rng, count as u64) as usize]];
                    reference += cluster.reference_units[metric];
                    left += cluster.left_errors[metric];
                    right += cluster.right_errors[metric];
                }
                if reference == 0 {
                    estimate.undefined_bootstrap_samples += 1;
                    continue;
                }
                self.samples[defined] = (right as f64 - left as f64) / reference as f64;
                defined += 1;
            }
            estimate.bootstrap_samples = resamples;
            // Do not silently condition a percentile interval on defined draws
            // if resampling empty references makes any draw undefined.
            if estimate.undefined_bootstrap_samples == 0 {
                let samples = &mut self.samples[..defined];
                samples.sort_unstable_by(f64::total_cmp);
                estimate.ci_95_low = Some(percentile(samples, 0.025));
                estimate.ci_95_high = Some(percentile(samples, 0.975));
            }
        }
        Ok(result)
    }
}

fn percentile(sorted: &[f64], quantile: f64) -> f64 {
    let position = quantile * (sorted.len() - 1) as f64;
    let lower = position.floor() as usize;
    let upper = position.ceil() as usize;
    sorted[lower] + (sorted[upper] - sorted[lower]) * (position - lower as f64)
}

const fn random(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9e3779b97f4a7c15);
    let mut value = *state;
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d049bb133111eb);
    value ^ (value >> 31)
}

fn sample(state: &mut u64, upper: u64) -> u64 {
    // Rejection sampling removes the modulo bias for non-power-of-two counts.
    let threshold = upper.wrapping_neg() % upper;
    loop {
        let value = random(state);
        if value >= threshold {
            return value % upper;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn analytical_two_cluster_distribution_preserves_document_weights() {
        // Two pages from A remain together. B has one page. Paired draws produce
        // differences 0, 1/3, or 1, with probabilities 1/4, 1/2, 1/4.
        let clusters = [
            Cluster {
                reference_units: [2, 2, 0],
                left_errors: [0; 3],
                right_errors: [0; 3],
                cases: [2, 2, 0],
            },
            Cluster {
                reference_units: [1, 1, 0],
                left_errors: [0; 3],
                right_errors: [1, 1, 0],
                cases: [1, 1, 0],
            },
        ];
        let mut workspace = Workspace::default();
        let result = workspace.estimate(&clusters, 20261017, 10_000).unwrap();
        assert_eq!(result[0].paired_cases, 3);
        assert_eq!(result[0].document_clusters, 2);
        assert_eq!(result[0].right_minus_left, Some(1.0 / 3.0));
        assert_eq!(result[0].ci_95_low, Some(0.0));
        assert_eq!(result[0].ci_95_high, Some(1.0));
        assert_eq!(result[2].right_minus_left, None);
    }

    #[test]
    fn paired_equal_outputs_have_exact_zero_difference_with_a_fixed_seed() {
        let clusters = [
            Cluster {
                reference_units: [10; 3],
                left_errors: [3; 3],
                right_errors: [3; 3],
                cases: [1; 3],
            },
            Cluster {
                reference_units: [20; 3],
                left_errors: [2; 3],
                right_errors: [2; 3],
                cases: [2; 3],
            },
        ];
        let mut workspace = Workspace::default();
        let result = workspace.estimate(&clusters, 781, 1000).unwrap();
        for metric in result {
            assert_eq!(metric.ci_95_low, Some(0.0));
            assert_eq!(metric.ci_95_high, Some(0.0));
        }
    }

    #[test]
    fn singleton_cluster_and_empty_reference_do_not_get_false_certainty() {
        let clusters = [Cluster {
            reference_units: [10, 0, 0],
            left_errors: [1, 2, 0],
            right_errors: [2, 3, 0],
            cases: [1, 1, 0],
        }];
        let mut workspace = Workspace::default();
        let result = workspace.estimate(&clusters, 1, 1000).unwrap();
        assert_eq!(result[0].right_minus_left, Some(0.1));
        assert_eq!(result[0].ci_95_low, None);
        assert_eq!(result[0].bootstrap_samples, 0);
        assert_eq!(result[1].right_minus_left, None);
        assert_eq!(result[1].left_errors, 2);
    }

    #[test]
    fn percentile_interpolates_linearly_and_sampling_is_reproducible() {
        assert_eq!(percentile(&[0.0, 1.0, 2.0, 3.0], 0.25), 0.75);
        let mut left = 31;
        let mut right = 31;
        for _ in 0..1000 {
            let a = sample(&mut left, 7);
            assert!(a < 7);
            assert_eq!(a, sample(&mut right, 7));
        }
    }
}
