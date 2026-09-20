//! Isolate residual counting; synthetic coverage, no history loading or crypto.
#[allow(dead_code)]
#[path = "../src/candidates.rs"]
mod candidates;
use candidates::{PrefixCoverage, Slot, Step};
use std::{hint::black_box, time::Instant};

struct CoveredAfterTwo;
const RAW_LIMIT: usize = 20_000_000;
impl PrefixCoverage for CoveredAfterTwo {
    fn covers(&self, _: &[u16; 12], depth: usize) -> bool {
        depth >= 2
    }
}

fn measure(enabled: bool, batch: usize) -> (f64, usize, usize) {
    let mut stream = candidates::stream([Slot::Hole; 12], (0..12).collect(), vec![]);
    stream.set_count_cache(enabled);
    let mut raw = 0;
    let mut generated = 0;
    let mut steps = 0;
    let start = Instant::now();
    while raw < RAW_LIMIT {
        let limit = (batch - raw % batch).min(RAW_LIMIT - raw);
        match black_box(stream.next_pruned(limit, &CoveredAfterTwo)).unwrap() {
            Step::Candidate(phrase) => {
                black_box(phrase);
                generated += 1;
                raw += 1;
            }
            Step::Skipped(n) => raw += n,
        }
        steps += 1;
    }
    (start.elapsed().as_secs_f64(), generated, steps)
}

fn main() {
    println!("batch,trial,cache,seconds,raw,generated,steps");
    for batch in [4096, 65536] {
        let reference = measure(false, batch);
        for trial in 0..=5 {
            let order = if trial % 2 == 0 {
                [false, true]
            } else {
                [true, false]
            };
            for enabled in order {
                let (seconds, generated, steps) = measure(enabled, batch);
                assert_eq!((generated, steps), (reference.1, reference.2));
                if trial > 0 {
                    println!(
                        "{batch},{trial},{enabled},{seconds:.6},{RAW_LIMIT},{generated},{steps}"
                    );
                }
            }
        }
    }
}
