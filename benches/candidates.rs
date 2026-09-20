//! Run: cargo bench --no-default-features --bench candidates
//! Historical baseline exists only here, never in the search binary.
#[allow(dead_code)]
#[path = "../src/candidates.rs"]
mod current;
#[allow(dead_code)]
#[path = "support/legacy_candidates.rs"]
mod legacy;
#[path = "support/upstream_two_pools.rs"]
mod upstream_two_pools;
use std::{hint::black_box, time::Instant};
fn measure(mut make: impl FnMut() -> Box<dyn Iterator<Item = [u16; 12]>>) -> f64 {
    let mut samples = Vec::new();
    for _ in 0..5 {
        let it = make();
        let start = Instant::now();
        let n = it.take(1_000_000).fold(0usize, |n, c| {
            black_box(c);
            n + 1
        });
        samples.push(n as f64 / start.elapsed().as_secs_f64());
    }
    samples.sort_by(f64::total_cmp);
    samples[2]
}
fn main() {
    for (name, p) in [
        ("12 known words", 12),
        ("10 known + 2 fill slots", 10),
        ("fill only", 0),
    ] {
        let old = measure(|| {
            legacy::stream(
                [legacy::Slot::Hole; 12],
                (0..p).collect(),
                (0..2048).collect(),
            )
        });
        let new = measure(|| {
            Box::new(current::stream(
                [current::Slot::Hole; 12],
                (0..p).collect(),
                (0..2048).collect(),
            ))
        });
        println!(
            "{name}: old={old:.0}/s new={new:.0}/s speedup={:.2}x (enumeration only)",
            new / old
        );
    }
    let mut slots = [current::Slot::Fixed(99); 12];
    slots[..8].fill(current::Slot::Hole);
    let old = measure(|| {
        upstream_two_pools::stream_two_pools(slots, 4, (0..8).collect(), (8..16).collect())
    });
    let new = measure(|| {
        Box::new(current::stream_two_pools(
            slots,
            4,
            (0..8).collect(),
            (8..16).collect(),
        ))
    });
    println!(
        "Two pools, 8 holes: upstream={old:.0}/s new={new:.0}/s speedup={:.2}x (enumeration only)",
        new / old
    );
}
