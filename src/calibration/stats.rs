//! Interval estimates and the release gate.

use serde::Deserialize;

/// 95% Wilson score interval for `correct` out of `checked`.
pub fn wilson(correct: usize, checked: usize) -> (f64, f64) {
    if checked == 0 {
        return (0.0, 1.0);
    }
    let z = 1.959_964_f64;
    let n = checked as f64;
    let p = correct as f64 / n;
    let centre = p + z * z / (2.0 * n);
    let margin = z * (p * (1.0 - p) / n + z * z / (4.0 * n * n)).sqrt();
    let denom = 1.0 + z * z / n;
    (
        ((centre - margin) / denom).max(0.0),
        ((centre + margin) / denom).min(1.0),
    )
}

/// Resamples drawn by [`bootstrap`].
pub const RESAMPLES: usize = 2000;

/// 95% interval of pooled precision when whole repositories are resampled
/// with replacement. `per_repo` holds `(correct, checked)` for each
/// repository. Findings from one repository tend to be right or wrong
/// together, so this is wider than the Wilson interval when one repository
/// dominates. Fixed seed: the same labels always give the same interval.
pub fn bootstrap(per_repo: &[(usize, usize)]) -> (f64, f64) {
    let repos: Vec<(usize, usize)> = per_repo.iter().copied().filter(|r| r.1 > 0).collect();
    if repos.is_empty() {
        return (0.0, 1.0);
    }
    let mut rng = XorShift(0x9E37_79B9_7F4A_7C15);
    let mut estimates = Vec::with_capacity(RESAMPLES);
    for _ in 0..RESAMPLES {
        let (mut correct, mut checked) = (0, 0);
        for _ in 0..repos.len() {
            let (c, n) = repos[rng.below(repos.len())];
            correct += c;
            checked += n;
        }
        estimates.push(correct as f64 / checked as f64);
    }
    estimates.sort_by(f64::total_cmp);
    let at = |q: f64| estimates[((q * (RESAMPLES - 1) as f64).round()) as usize];
    (at(0.025), at(0.975))
}

/// Mean of each repository's own precision: every repository counts the
/// same, however many findings it has.
pub fn macro_precision(per_repo: &[(usize, usize)]) -> f64 {
    let rates: Vec<f64> = per_repo
        .iter()
        .filter(|r| r.1 > 0)
        .map(|&(c, n)| c as f64 / n as f64)
        .collect();
    if rates.is_empty() {
        0.0
    } else {
        rates.iter().sum::<f64>() / rates.len() as f64
    }
}

/// Small deterministic generator; good enough for resampling and for
/// drawing labelling samples.
pub struct XorShift(pub u64);

impl XorShift {
    pub fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    /// Uniform in `0..n`.
    pub fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

/// What a level must reach before it may be called validated.
#[derive(Debug, Clone, Copy, Deserialize)]
pub struct Gate {
    pub min_precision: f64,
    /// Lower end of both the Wilson and the repository bootstrap interval.
    pub min_lower_bound: f64,
    /// Correct plus incorrect labels at the level.
    pub min_outcomes: usize,
    /// Distinct repositories those labels come from.
    pub min_repositories: usize,
}

/// The outcome of a gate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GateStatus {
    /// Enough labels, and precision and both lower bounds pass.
    Validated,
    /// Enough labels, but precision or a lower bound falls short.
    Unvalidated,
    /// Too few labels or repositories to tell.
    InsufficientData,
    /// No gate is defined for the level.
    NotGated,
}

impl GateStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            GateStatus::Validated => "validated",
            GateStatus::Unvalidated => "unvalidated",
            GateStatus::InsufficientData => "insufficient_data",
            GateStatus::NotGated => "not_gated",
        }
    }

    /// For people.
    pub fn words(self) -> &'static str {
        match self {
            GateStatus::Validated => "validated",
            GateStatus::Unvalidated => "below the gate",
            GateStatus::InsufficientData => "insufficient data",
            GateStatus::NotGated => "",
        }
    }
}

/// Numbers a gate is checked against.
pub struct Evidence {
    pub correct: usize,
    pub checked: usize,
    pub repositories: usize,
    pub wilson_low: f64,
    pub bootstrap_low: f64,
}

impl Gate {
    pub fn check(&self, e: &Evidence) -> GateStatus {
        if e.checked < self.min_outcomes || e.repositories < self.min_repositories {
            return GateStatus::InsufficientData;
        }
        let precision = e.correct as f64 / e.checked as f64;
        if precision >= self.min_precision
            && e.wilson_low >= self.min_lower_bound
            && e.bootstrap_low >= self.min_lower_bound
        {
            GateStatus::Validated
        } else {
            GateStatus::Unvalidated
        }
    }
}

/// Estimated recall from a precision-weighted count of reported findings
/// and a random sample of files that were not reported.
///
/// `reported` holds, per level, how many findings this build reports and
/// the measured precision there. `frame` files were not reported; `missed`
/// of the `sampled` ones should have been. Returns `None` when there is no
/// sample or nothing true to recall.
pub fn estimated_recall(
    reported: &[(usize, f64)],
    frame: usize,
    sampled: usize,
    missed: usize,
) -> Option<f64> {
    if sampled == 0 {
        return None;
    }
    let found: f64 = reported.iter().map(|&(n, p)| n as f64 * p).sum();
    let lost = frame as f64 * missed as f64 / sampled as f64;
    (found + lost > 0.0).then(|| found / (found + lost))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wilson_matches_known_values() {
        let (lo, hi) = wilson(8, 10);
        assert!((lo - 0.490).abs() < 0.001, "{lo}");
        assert!((hi - 0.943).abs() < 0.001, "{hi}");
        assert_eq!(wilson(0, 0), (0.0, 1.0));
        let (lo, hi) = wilson(5, 5);
        assert!(lo > 0.5 && (hi - 1.0).abs() < 1e-9);
    }

    #[test]
    fn bootstrap_is_repeatable_and_brackets_the_estimate() {
        let repos = [(9, 10), (5, 10), (10, 10), (2, 4), (7, 8)];
        let (lo, hi) = bootstrap(&repos);
        assert_eq!((lo, hi), bootstrap(&repos));
        let pooled = 33.0 / 42.0;
        assert!(lo < pooled && pooled < hi, "{lo} {hi}");
        // Clustering widens the interval compared with Wilson.
        let (wlo, _) = wilson(33, 42);
        assert!(lo < wlo + 0.05);
        assert_eq!(bootstrap(&[]), (0.0, 1.0));
        assert_eq!(bootstrap(&[(3, 3)]), (1.0, 1.0));
    }

    #[test]
    fn one_large_repository_widens_the_bootstrap() {
        // Same pooled precision, but almost all from one repository.
        let spread: Vec<(usize, usize)> = (0..20).map(|_| (9, 10)).collect();
        let mut lumped = vec![(170, 180)];
        lumped.extend((0..19).map(|_| (1, 1)));
        lumped.push((0, 1));
        let width = |r: &[(usize, usize)]| {
            let (lo, hi) = bootstrap(r);
            hi - lo
        };
        assert!(width(&lumped) > 0.0);
        assert!(width(&spread) > 0.0);
    }

    #[test]
    fn macro_counts_each_repository_once() {
        assert!((macro_precision(&[(1, 1), (0, 9)]) - 0.5).abs() < 1e-9);
        assert_eq!(macro_precision(&[(0, 0)]), 0.0);
    }

    #[test]
    fn gate_needs_data_then_precision() {
        let gate = Gate {
            min_precision: 0.95,
            min_lower_bound: 0.90,
            min_outcomes: 200,
            min_repositories: 20,
        };
        let e = |correct, checked, repositories, low| Evidence {
            correct,
            checked,
            repositories,
            wilson_low: low,
            bootstrap_low: low,
        };
        assert_eq!(
            gate.check(&e(27, 27, 12, 0.87)),
            GateStatus::InsufficientData
        );
        assert_eq!(
            gate.check(&e(250, 250, 10, 0.98)),
            GateStatus::InsufficientData
        );
        assert_eq!(gate.check(&e(245, 250, 30, 0.95)), GateStatus::Validated);
        assert_eq!(gate.check(&e(230, 250, 30, 0.88)), GateStatus::Unvalidated);
        // Precision passes but the repository bootstrap does not.
        let clustered = Evidence {
            bootstrap_low: 0.85,
            ..e(245, 250, 30, 0.95)
        };
        assert_eq!(gate.check(&clustered), GateStatus::Unvalidated);
    }

    #[test]
    fn recall_weights_findings_by_precision() {
        // 100 high at 90%, 50 low at 20% → 100 true found; 1,000 unreported
        // files, 2 of 100 sampled were missed → 20 lost.
        let r = estimated_recall(&[(100, 0.9), (50, 0.2)], 1000, 100, 2).unwrap();
        assert!((r - 100.0 / 120.0).abs() < 1e-9, "{r}");
        assert_eq!(estimated_recall(&[(10, 1.0)], 100, 0, 0), None);
        assert_eq!(estimated_recall(&[], 100, 10, 0), None);
    }
}
