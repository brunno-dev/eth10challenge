//! Index-based candidate streaming for the search.
//!
//! Every search mode is one template: 12 slots, each either a fixed BIP-39 index
//! or a hole, plus a pool of words that fill the holes **without replacement**.
//! If the pool is smaller than the number of holes, the leftover holes each draw
//! independently from the *fill set* — by default the whole wordlist, but
//! narrowing it is the single most effective lever there is, since each such
//! hole multiplies the space by the fill set's size.
//!
//! Supplying 12 loose words is just the special case of 12 holes and a 12-word
//! pool, so the two modes share one enumerator and one candidate count.
//!
//! Candidates are yielded as `[u16; 12]` arrays of word indices — the compact
//! form the GPU kernel consumes.

use itertools::Itertools;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slot {
    /// A position whose word is known.
    Fixed(u16),
    /// A position to be filled from the pool, or from the full wordlist once the
    /// pool runs out.
    Hole,
}

/// Exactly how many candidates [`stream`] will yield.
///
/// With `h` holes, a pool of `p` and a fill set of `f`:
/// - `p >= h`: arrange `h` of the `p` pool words over the holes — `p!/(p-h)!`.
/// - `p < h`: place all `p` pool words into distinct holes (`h!/(h-p)!` ways),
///   then draw each of the `h-p` remaining holes from the fill set — `f^(h-p)`.
///
/// Returns `u128` because a few holes drawn from the full list overflow `u64`
/// quickly — `2048^6` alone is ~7.4e19.
pub fn count(slots: &[Slot; 12], pool_len: usize, fill_len: usize) -> u128 {
    let h = slots.iter().filter(|s| **s == Slot::Hole).count();
    if pool_len >= h {
        ((pool_len - h + 1)..=pool_len).map(|x| x as u128).product()
    } else {
        let w = h - pool_len;
        let placements: u128 = ((w + 1)..=h).map(|x| x as u128).product();
        placements * (fill_len as u128).pow(w as u32)
    }
}

/// Streams every candidate the template describes.
///
/// Nothing is collected up front: memory stays flat no matter how large the
/// space is.
/// `fill` is the set of word indices a hole may take once the pool is spent.
pub fn stream(
    slots: [Slot; 12],
    pool: Vec<u16>,
    fill: Vec<u16>,
) -> Box<dyn Iterator<Item = [u16; 12]> + Send> {
    let holes: Vec<usize> = (0..12).filter(|&i| slots[i] == Slot::Hole).collect();
    let mut base = [0u16; 12];
    for (i, s) in slots.iter().enumerate() {
        if let Slot::Fixed(w) = *s {
            base[i] = w;
        }
    }

    let h = holes.len();
    let p = pool.len();

    if h == 0 {
        return Box::new(std::iter::once(base));
    }

    if p >= h {
        // Every hole gets a distinct pool word; surplus pool words mean we also
        // choose *which* ones, which `permutations(h)` already enumerates.
        return Box::new(pool.into_iter().permutations(h).map(move |arr| {
            let mut out = base;
            for (&slot, w) in holes.iter().zip(arr) {
                out[slot] = w;
            }
            out
        }));
    }

    // Fewer pool words than holes. Pick which holes fall back to the fill set
    // first; the pool then permutes over the rest. Choosing the fallback slots
    // up front is what keeps this duplicate-free — inserting unknown words one
    // at a time generates each candidate w! times over.
    let w = h - p;
    let all_holes = holes.clone();
    Box::new(holes.into_iter().combinations(w).flat_map(move |wild| {
        let rest: Vec<usize> = all_holes
            .iter()
            .copied()
            .filter(|i| !wild.contains(i))
            .collect();
        let pool = pool.clone();
        let fill = fill.clone();
        pool.into_iter().permutations(p).flat_map(move |arr| {
            let mut tmpl = base;
            for (&slot, word) in rest.iter().zip(&arr) {
                tmpl[slot] = *word;
            }
            fill_holes(tmpl, wild.clone(), fill.clone())
        })
    }))
}

/// Expands the given slots over the fill set, one nesting level per slot.
fn fill_holes(
    tmpl: [u16; 12],
    slots: Vec<usize>,
    fill: Vec<u16>,
) -> Box<dyn Iterator<Item = [u16; 12]> + Send> {
    match slots.split_first() {
        None => Box::new(std::iter::once(tmpl)),
        Some((&slot, rest)) => {
            let rest = rest.to_vec();
            Box::new(fill.clone().into_iter().flat_map(move |word| {
                let mut t = tmpl;
                t[slot] = word;
                fill_holes(t, rest.clone(), fill.clone())
            }))
        }
    }
}
