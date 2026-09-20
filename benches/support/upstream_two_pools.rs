// Test-only baseline from lmajowka/eth10challenge commit 855fe1e (MIT).
use crate::current::Slot;
use itertools::Itertools;
pub fn stream_two_pools(
    slots: [Slot; 12],
    a: usize,
    pool1: Vec<u16>,
    pool2: Vec<u16>,
) -> Box<dyn Iterator<Item = [u16; 12]> + Send> {
    let holes: Vec<usize> = (0..12).filter(|&i| slots[i] == Slot::Hole).collect();
    let mut base = [0u16; 12];
    for (i, s) in slots.iter().enumerate() {
        if let Slot::Fixed(w) = *s {
            base[i] = w;
        }
    }

    let b = holes.len() - a;
    let all_holes = holes.clone();
    Box::new(holes.into_iter().combinations(a).flat_map(move |side1| {
        let side2: Vec<usize> = all_holes
            .iter()
            .copied()
            .filter(|i| !side1.contains(i))
            .collect();
        let pool1 = pool1.clone();
        let pool2 = pool2.clone();
        pool1.into_iter().permutations(a).flat_map(move |arr1| {
            let mut tmpl = base;
            for (&slot, &w) in side1.iter().zip(&arr1) {
                tmpl[slot] = w;
            }
            let side2 = side2.clone();
            pool2.clone().into_iter().permutations(b).map(move |arr2| {
                let mut out = tmpl;
                for (&slot, &w) in side2.iter().zip(&arr2) {
                    out[slot] = w;
                }
                out
            })
        })
    }))
}
