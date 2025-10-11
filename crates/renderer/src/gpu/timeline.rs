use std::time::{Duration, Instant};

use crate::types::CrossfadeCurve;

impl CrossfadeCurve {
    fn sample(self, t: f32) -> f32 {
        let clamped = t.clamp(0.0, 1.0);
        match self {
            CrossfadeCurve::Linear => clamped,
            CrossfadeCurve::Smoothstep => clamped * clamped * (3.0 - 2.0 * clamped),
            CrossfadeCurve::EaseInOut => {
                if clamped < 0.5 {
                    2.0 * clamped * clamped
                } else {
                    -1.0 + (4.0 - 2.0 * clamped) * clamped
                }
            }
        }
    }
}

pub(crate) struct FadeEnvelope {
    start: Instant,
    duration: Duration,
    curve: CrossfadeCurve,
}

impl FadeEnvelope {
    pub fn new(duration: Duration, curve: CrossfadeCurve, now: Instant) -> Option<Self> {
        if duration <= Duration::ZERO {
            None
        } else {
            Some(Self {
                start: now,
                duration,
                curve,
            })
        }
    }

    pub fn mixes(&self, now: Instant) -> (f32, f32, bool) {
        let elapsed = now.saturating_duration_since(self.start);
        let progress = elapsed.as_secs_f32() / self.duration.as_secs_f32().max(f32::EPSILON);
        let mix = self.curve.sample(progress);
        let finished = progress >= 1.0;
        (1.0 - mix, mix, finished)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linear_curve_increases_monotonically() {
        let curve = CrossfadeCurve::Linear;
        let mut last = 0.0;
        for step in 0..=10 {
            let sample = curve.sample(step as f32 / 10.0);
            assert!(sample >= last - f32::EPSILON);
            last = sample;
        }
    }

    #[test]
    fn smoothstep_matches_expected_values() {
        let curve = CrossfadeCurve::Smoothstep;
        assert!((curve.sample(0.0) - 0.0).abs() < 1e-6);
        assert!((curve.sample(0.5) - 0.5).abs() < 1e-6);
        assert!((curve.sample(1.0) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn ease_in_out_accelerates_then_decelerates() {
        let curve = CrossfadeCurve::EaseInOut;
        let first = curve.sample(0.25);
        let mid = curve.sample(0.5);
        let last = curve.sample(0.75);
        assert!(first < mid);
        assert!(last > mid);
        assert!((curve.sample(0.0) - 0.0).abs() < 1e-6);
        assert!((curve.sample(1.0) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn fade_envelope_reports_mix_progress() {
        let start = Instant::now();
        let envelope = FadeEnvelope::new(Duration::from_millis(100), CrossfadeCurve::Linear, start)
            .expect("envelope");
        let (_prev_mix, curr_mix, finished) = envelope.mixes(start + Duration::from_millis(50));
        assert!((curr_mix - 0.5).abs() < 0.05);
        assert!(!finished);
    }
}
