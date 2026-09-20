//! Pure batch-size policy; measurements and GPU operations stay with the caller.
use std::time::Duration;

const INITIAL_SIZE: usize = 16_384;
const MIN_SIZE: usize = 256;
const FAST_THRESHOLD: Duration = Duration::from_millis(125);
const SLOW_THRESHOLD: Duration = Duration::from_millis(500);

/// Hysteresis around a 250 ms target prevents reacting to small timing changes.
pub struct BatchController {
    ceiling: usize,
    floor: usize,
    size: usize,
    consecutive_fast: u8,
}

impl BatchController {
    pub fn new(ceiling: usize) -> Self {
        assert!(ceiling >= 1, "batch ceiling must be positive");
        Self {
            ceiling,
            floor: MIN_SIZE.min(ceiling),
            size: INITIAL_SIZE.min(ceiling),
            consecutive_fast: 0,
        }
    }

    pub fn size(&self) -> usize {
        self.size
    }

    /// Observe one completed batch and return the size for future submissions.
    /// A queued batch using an old request cannot adjust the current setting.
    pub fn observe(&mut self, requested: usize, retained: usize, elapsed: Duration) -> usize {
        if requested != self.size || retained == 0 || retained > requested || elapsed.is_zero() {
            return self.size;
        }
        if elapsed > SLOW_THRESHOLD {
            // Even a partial batch is evidence of actual latency when slow.
            self.size = (self.size / 2).max(self.floor);
            self.consecutive_fast = 0;
        } else if retained == requested && elapsed < FAST_THRESHOLD {
            self.consecutive_fast += 1;
            if self.consecutive_fast == 2 {
                self.size = self.size.saturating_mul(2).min(self.ceiling);
                self.consecutive_fast = 0;
            }
        } else {
            // Partial or ordinary-duration batches interrupt fast growth evidence.
            self.consecutive_fast = 0;
        }
        self.size
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fast(controller: &mut BatchController) -> usize {
        let requested = controller.size();
        controller.observe(requested, requested, Duration::from_millis(100))
    }

    #[test]
    fn grows_after_two_fast_full_batches_and_stops_at_ceiling() {
        let mut controller = BatchController::new(50_000);
        assert_eq!(controller.size(), 16_384);
        assert_eq!(fast(&mut controller), 16_384);
        assert_eq!(fast(&mut controller), 32_768);
        assert_eq!(fast(&mut controller), 32_768);
        assert_eq!(fast(&mut controller), 50_000);
        for _ in 0..10 {
            assert_eq!(fast(&mut controller), 50_000);
        }
    }

    #[test]
    fn slow_samples_shrink_immediately_including_partial_batches() {
        let mut controller = BatchController::new(100_000);
        assert_eq!(fast(&mut controller), 16_384);
        assert_eq!(
            controller.observe(16_384, 1, Duration::from_millis(501)),
            8_192
        );
        // Slowdown resets the debounce; the next fast batch cannot grow yet.
        assert_eq!(fast(&mut controller), 8_192);
        for _ in 0..16 {
            let requested = controller.size();
            controller.observe(requested, requested, Duration::from_secs(1));
        }
        assert_eq!(controller.size(), 256);
    }

    #[test]
    fn low_ceilings_are_also_floors_and_always_positive() {
        for ceiling in [1, 2, 127, 255, 256] {
            let mut controller = BatchController::new(ceiling);
            assert_eq!(controller.size(), ceiling);
            assert_eq!(fast(&mut controller), ceiling);
            assert_eq!(fast(&mut controller), ceiling);
            assert_eq!(
                controller.observe(ceiling, 1, Duration::from_secs(1)),
                ceiling
            );
        }
    }

    #[test]
    fn stale_empty_and_zero_duration_samples_do_not_change_state() {
        let mut controller = BatchController::new(100_000);
        fast(&mut controller);
        for (requested, retained, elapsed) in [
            (16_384, 0, Duration::from_secs(1)),
            (16_384, 16_384, Duration::ZERO),
            (8_192, 8_192, Duration::from_secs(1)),
            (32_768, 32_768, Duration::from_millis(1)),
            (16_384, 16_385, Duration::from_millis(1)),
        ] {
            assert_eq!(controller.observe(requested, retained, elapsed), 16_384);
        }
        assert_eq!(fast(&mut controller), 32_768);
        // A lookahead batch carrying the previous size cannot shrink the new one.
        assert_eq!(
            controller.observe(16_384, 16_384, Duration::from_secs(2)),
            32_768
        );
    }

    #[test]
    fn partial_and_threshold_samples_interrupt_growth() {
        let mut controller = BatchController::new(100_000);
        for (retained, elapsed) in [
            (1, Duration::from_millis(1)),
            (16_384, Duration::from_millis(125)),
            (16_384, Duration::from_millis(250)),
            (16_384, Duration::from_millis(500)),
        ] {
            fast(&mut controller);
            assert_eq!(controller.observe(16_384, retained, elapsed), 16_384);
            assert_eq!(fast(&mut controller), 16_384);
            // Reset between cases, preserving each case's independent debounce.
            controller.observe(16_384, 1, Duration::from_millis(1));
        }
        assert_eq!(fast(&mut controller), 16_384);
        assert_eq!(fast(&mut controller), 32_768);
    }

    #[test]
    fn growth_saturates_without_integer_overflow() {
        let mut controller = BatchController::new(usize::MAX);
        for _ in 0..(usize::BITS * 2) {
            fast(&mut controller);
        }
        assert_eq!(controller.size(), usize::MAX);
        assert_eq!(fast(&mut controller), usize::MAX);
        assert_eq!(fast(&mut controller), usize::MAX);
    }
}
