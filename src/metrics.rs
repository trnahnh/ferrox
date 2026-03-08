#[cfg(feature = "metrics")]
mod inner {
    use hdrhistogram::Histogram;

    pub struct LatencyRecorder {
        histogram: Histogram<u64>,
    }

    impl LatencyRecorder {
        pub fn new() -> Self {
            Self {
                histogram: Histogram::new_with_bounds(1, 60_000_000_000, 3)
                    .expect("histogram bounds valid"),
            }
        }

        #[inline]
        pub fn start() -> std::time::Instant {
            std::time::Instant::now()
        }

        #[inline]
        pub fn record(&mut self, start: std::time::Instant) {
            let elapsed = start.elapsed().as_nanos() as u64;
            let _ = self.histogram.record(elapsed);
        }

        #[inline]
        pub fn record_nanos(&mut self, nanos: u64) {
            let _ = self.histogram.record(nanos);
        }

        pub fn p50(&self) -> u64 {
            self.histogram.value_at_quantile(0.50)
        }

        pub fn p90(&self) -> u64 {
            self.histogram.value_at_quantile(0.90)
        }

        pub fn p99(&self) -> u64 {
            self.histogram.value_at_quantile(0.99)
        }

        pub fn p999(&self) -> u64 {
            self.histogram.value_at_quantile(0.999)
        }

        pub fn max(&self) -> u64 {
            self.histogram.max()
        }

        pub fn mean(&self) -> f64 {
            self.histogram.mean()
        }

        pub fn count(&self) -> u64 {
            self.histogram.len()
        }

        pub fn reset(&mut self) {
            self.histogram.reset();
        }

        pub fn print_summary(&self) {
            println!("Latency (ns):");
            println!("  P50   : {:>10}", self.p50());
            println!("  P90   : {:>10}", self.p90());
            println!("  P99   : {:>10}", self.p99());
            println!("  P99.9 : {:>10}", self.p999());
            println!("  Max   : {:>10}", self.max());
            println!("  Mean  : {:>10.0}", self.mean());
            println!("  Count : {:>10}", self.count());
        }
    }

    impl std::fmt::Debug for LatencyRecorder {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("LatencyRecorder")
                .field("count", &self.count())
                .finish()
        }
    }
}

#[cfg(not(feature = "metrics"))]
mod inner {
    #[derive(Debug)]
    pub struct LatencyRecorder;

    impl LatencyRecorder {
        #[inline]
        pub fn new() -> Self {
            Self
        }

        #[inline]
        pub fn start() {}

        #[inline]
        pub fn record(&mut self, _start: ()) {}

        #[inline]
        pub fn record_nanos(&mut self, _nanos: u64) {}

        #[inline]
        pub fn p50(&self) -> u64 {
            0
        }

        #[inline]
        pub fn p90(&self) -> u64 {
            0
        }

        #[inline]
        pub fn p99(&self) -> u64 {
            0
        }

        #[inline]
        pub fn p999(&self) -> u64 {
            0
        }

        #[inline]
        pub fn max(&self) -> u64 {
            0
        }

        #[inline]
        pub fn mean(&self) -> f64 {
            0.0
        }

        #[inline]
        pub fn count(&self) -> u64 {
            0
        }

        #[inline]
        pub fn reset(&mut self) {}

        #[inline]
        pub fn print_summary(&self) {}
    }
}

pub use inner::LatencyRecorder;

#[cfg(feature = "metrics")]
pub type Timestamp = std::time::Instant;

#[cfg(not(feature = "metrics"))]
pub type Timestamp = ();

#[cfg(test)]
#[cfg(feature = "metrics")]
mod tests {
    use super::*;

    #[test]
    fn record_and_read_percentiles() {
        let mut recorder = LatencyRecorder::new();
        for i in 1..=1000u64 {
            recorder.record_nanos(i * 100);
        }
        assert_eq!(recorder.count(), 1000);
        assert!(recorder.p50() > 0);
        assert!(recorder.p99() > recorder.p50());
        assert!(recorder.max() >= 100_000);
    }

    #[test]
    fn empty_histogram() {
        let recorder = LatencyRecorder::new();
        assert_eq!(recorder.count(), 0);
        assert_eq!(recorder.p50(), 0);
        assert_eq!(recorder.max(), 0);
    }

    #[test]
    fn reset_clears_data() {
        let mut recorder = LatencyRecorder::new();
        recorder.record_nanos(1000);
        assert_eq!(recorder.count(), 1);
        recorder.reset();
        assert_eq!(recorder.count(), 0);
    }
}
