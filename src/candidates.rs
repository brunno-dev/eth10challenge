//! Distinct phrases, generated without allocations in Iterator::next.
use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex, OnceLock},
};

const COUNT_CACHE_LIMIT: usize = 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct CountKey {
    depth: usize,
    choices: [usize; 12],
}

#[derive(Debug, Default)]
struct CountCache {
    entries: HashMap<CountKey, Option<u128>>,
    order: VecDeque<CountKey>,
    #[cfg(test)]
    hits: usize,
}

#[derive(Debug, Default)]
struct LazyCountCache(OnceLock<Mutex<CountCache>>);

impl Clone for LazyCountCache {
    fn clone(&self) -> Self {
        // A deep Domain clone can subsequently change its pools. Only sharing
        // the same immutable Arc<Domain> is allowed to share cached counts.
        Self::default()
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Slot {
    Fixed(u16),
    Hole,
}
#[derive(Debug, PartialEq, Eq)]
pub enum Step {
    Candidate([u16; 12]),
    Skipped(usize),
}
pub trait PrefixCoverage {
    fn covers(&self, phrase: &[u16; 12], depth: usize) -> bool;
}
#[derive(Clone, Debug)]
struct Choice {
    word: u16,
    capacity: usize,
    fill: bool,
}
#[derive(Clone, Debug)]
struct Domain {
    choices: Vec<Choice>,
    pool_types: usize,
    holes: Vec<usize>,
    required: bool,
    split: Option<Split>,
    count_cache: LazyCountCache,
}
#[derive(Clone, Debug)]
struct Split {
    post: Vec<usize>,
    video: Vec<usize>,
    quota: [usize; 2],
}
#[derive(Clone, Debug)]
pub struct Candidates {
    domain: Arc<Domain>,
    remaining: Vec<usize>,
    phrase: [u16; 12],
    next_choice: [usize; 12],
    chosen: [usize; 12],
    from_pool: [bool; 12],
    free: usize,
    depth: usize,
    done: bool,
    minimum: [[usize; 2]; 13],
    count_cache_enabled: bool,
}
/// Only traversal state is saved; immutable search inputs are checked separately.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Cursor {
    next_choice: [usize; 12],
    chosen: [usize; 12],
    depth: usize,
    done: bool,
}
fn domain(slots: &[Slot; 12], pool: &[u16], fill: &[u16]) -> Domain {
    let holes: Vec<_> = (0..12).filter(|&i| slots[i] == Slot::Hole).collect();
    let required = pool.len() < holes.len();
    let mut choices: Vec<Choice> = Vec::new();
    for &word in pool {
        if let Some(c) = choices.iter_mut().find(|c| c.word == word) {
            c.capacity += 1;
        } else {
            choices.push(Choice {
                word,
                capacity: 1,
                fill: false,
            });
        }
    }
    let pool_types = choices.len();
    if required {
        for &word in fill {
            if let Some(c) = choices.iter_mut().find(|c| c.word == word) {
                c.fill = true;
            } else {
                choices.push(Choice {
                    word,
                    capacity: 0,
                    fill: true,
                });
            }
        }
    }
    Domain {
        choices,
        pool_types,
        holes,
        required,
        split: None,
        count_cache: LazyCountCache::default(),
    }
}
pub fn stream(slots: [Slot; 12], pool: Vec<u16>, fill: Vec<u16>) -> Candidates {
    let d = domain(&slots, &pool, &fill);
    let mut phrase = [0; 12];
    for (i, slot) in slots.iter().enumerate() {
        if let Slot::Fixed(w) = slot {
            phrase[i] = *w;
        }
    }
    Candidates {
        remaining: d.choices.iter().map(|c| c.capacity).collect(),
        free: d.holes.len().saturating_sub(pool.len()),
        domain: Arc::new(d),
        phrase,
        next_choice: [0; 12],
        chosen: [0; 12],
        from_pool: [false; 12],
        depth: 0,
        done: false,
        minimum: [[0; 2]; 13],
        count_cache_enabled: true,
    }
}
/// Distinct phrases with exactly `post_open` holes supplied by the post pool.
/// Shared words have one traversal branch, regardless of their possible origin.
pub fn stream_two_pools(
    slots: [Slot; 12],
    post_open: usize,
    post: Vec<u16>,
    video: Vec<u16>,
) -> Candidates {
    let h = slots.iter().filter(|s| **s == Slot::Hole).count();
    let valid =
        post_open <= h && post.len() >= post_open && video.len() >= h.saturating_sub(post_open);
    let mut it = stream(slots, post.iter().chain(&video).copied().collect(), vec![]);
    let d = Arc::make_mut(&mut it.domain);
    d.split = Some(Split {
        post: d
            .choices
            .iter()
            .map(|c| post.iter().filter(|&&w| w == c.word).count())
            .collect(),
        video: d
            .choices
            .iter()
            .map(|c| video.iter().filter(|&&w| w == c.word).count())
            .collect(),
        quota: [post_open, h.saturating_sub(post_open)],
    });
    it.done = !valid;
    it
}
impl Candidates {
    /// Disable memoization for comparison without changing traversal or cursors.
    /// Ordinary Iterator::next never initializes or accesses the cache.
    pub fn set_count_cache(&mut self, enabled: bool) {
        self.count_cache_enabled = enabled;
    }

    // Each word used k times needs at least max(0,k-video_capacity) post
    // occurrences, and vice versa. Those two lower bounds completely describe
    // feasible source assignments; their intervals contain no gaps.
    fn split_minimum(&self, choice: usize) -> [usize; 2] {
        let mut next = self.minimum[self.depth];
        if let Some(split) = &self.domain.split {
            let used = self.domain.choices[choice].capacity - self.remaining[choice];
            next[0] += usize::from(used >= split.video[choice]);
            next[1] += usize::from(used >= split.post[choice]);
        }
        next
    }
    fn split_available(&self, choice: usize) -> bool {
        self.domain.split.as_ref().is_none_or(|split| {
            let next = self.split_minimum(choice);
            next[0] <= split.quota[0] && next[1] <= split.quota[1]
        })
    }
    pub fn cursor(&self) -> Cursor {
        Cursor {
            next_choice: self.next_choice,
            chosen: self.chosen,
            depth: self.depth,
            done: self.done,
        }
    }
    pub fn restore(&mut self, c: &Cursor) -> Result<(), &'static str> {
        // Restore only onto a fresh iterator. Replay its active prefix to derive
        // capacities/free slots instead of trusting serialized counters.
        if self.depth != 0 || self.done || self.next_choice != [0; 12] {
            return Err("iterator is not fresh");
        }
        let h = self.domain.holes.len();
        if c.depth >= h.max(1) || (c.done && c.depth != 0) {
            return Err("invalid cursor depth");
        }
        for d in 0..=c.depth {
            let limit = if self.free > 0 {
                self.domain.choices.len()
            } else {
                self.domain.pool_types
            };
            if c.next_choice[d] > limit {
                return Err("invalid cursor choice");
            }
            if d == c.depth {
                break;
            }
            let choice = c.chosen[d];
            if choice >= limit || c.next_choice[d] != choice + 1 {
                return Err("invalid cursor prefix");
            }
            self.depth = d;
            if !self.split_available(choice) {
                return Err("invalid batch quota in cursor");
            }
            self.minimum[d + 1] = self.split_minimum(choice);
            self.from_pool[d] = self.remaining[choice] > 0;
            if self.from_pool[d] {
                self.remaining[choice] -= 1;
            } else if self.free > 0 && self.domain.choices[choice].fill {
                self.free -= 1;
            } else {
                return Err("unavailable cursor word");
            }
            self.phrase[self.domain.holes[d]] = self.domain.choices[choice].word;
        }
        self.next_choice = c.next_choice;
        self.chosen = c.chosen;
        self.depth = c.depth;
        self.done = c.done;
        Ok(())
    }
    fn backtrack(&mut self) {
        self.depth -= 1;
        if self.from_pool[self.depth] {
            self.remaining[self.chosen[self.depth]] += 1;
        } else {
            self.free += 1;
        }
    }
    /// Zero-based rank in the original domain's enumeration order. The current
    /// cursor is ignored. Invalid phrases and arithmetic overflow return None.
    pub fn rank_of(&self, phrase: &[u16; 12]) -> Option<u128> {
        if (0..12).any(|p| !self.domain.holes.contains(&p) && phrase[p] != self.phrase[p]) {
            return None;
        }
        let mut it = self.clone();
        it.remaining = it.domain.choices.iter().map(|c| c.capacity).collect();
        it.free = it
            .domain
            .holes
            .len()
            .saturating_sub(it.remaining.iter().sum());
        it.depth = 0;
        it.minimum = [[0; 2]; 13];
        it.chosen = [0; 12];
        let mut rank = 0u128;
        for &position in &self.domain.holes {
            let selected = it
                .domain
                .choices
                .iter()
                .position(|c| c.word == phrase[position])?;
            let limit = if it.free > 0 {
                it.domain.choices.len()
            } else {
                it.domain.pool_types
            };
            if selected >= limit {
                return None;
            }
            for choice in 0..=selected {
                let available = (it.remaining[choice] > 0
                    || (it.free > 0 && it.domain.choices[choice].fill))
                    && it.split_available(choice);
                if !available {
                    if choice == selected {
                        return None;
                    }
                    continue;
                }
                it.chosen[it.depth] = choice;
                it.minimum[it.depth + 1] = it.split_minimum(choice);
                it.from_pool[it.depth] = it.remaining[choice] > 0;
                if it.from_pool[it.depth] {
                    it.remaining[choice] -= 1;
                } else {
                    it.free -= 1;
                }
                it.depth += 1;
                if choice != selected {
                    rank = rank.checked_add(it.suffix_count()?)?;
                    it.backtrack();
                }
            }
        }
        (it.suffix_count()? == 1).then_some(rank)
    }

    /// Exact number of candidates after this cursor, without enumerating leaves.
    /// Only the active frontier and its untouched sibling subtrees are visited.
    /// Cloning preserves the producer/consumer cursor and the enumeration order.
    pub fn remaining_count(&self) -> Option<u128> {
        if self.done {
            return Some(0);
        }
        if self.domain.holes.is_empty() {
            return Some(1);
        }
        let mut it = self.clone();
        let mut total = 0u128;
        loop {
            if it.next_choice[it.depth] == 0 {
                total = total.checked_add(it.suffix_count()?)?;
                if it.depth == 0 {
                    return Some(total);
                }
                it.backtrack();
                continue;
            }
            let limit = if it.free > 0 {
                it.domain.choices.len()
            } else {
                it.domain.pool_types
            };
            for choice in it.next_choice[it.depth]..limit {
                if !(it.remaining[choice] > 0 || (it.free > 0 && it.domain.choices[choice].fill))
                    || !it.split_available(choice)
                {
                    continue;
                }
                it.chosen[it.depth] = choice;
                it.minimum[it.depth + 1] = it.split_minimum(choice);
                it.from_pool[it.depth] = it.remaining[choice] > 0;
                if it.from_pool[it.depth] {
                    it.remaining[choice] -= 1;
                } else {
                    it.free -= 1;
                }
                it.depth += 1;
                total = total.checked_add(it.suffix_count()?)?;
                it.backtrack();
            }
            if it.depth == 0 {
                return Some(total);
            }
            it.backtrack();
        }
    }
    /// Count the untouched suffix, including ambiguous source assignments only once.
    fn suffix_count(&self) -> Option<u128> {
        if !self.count_cache_enabled {
            return self.suffix_count_uncached();
        }
        // For one immutable domain, the prefix multiset determines remaining
        // capacities and free fill slots. Each source minimum is a sum of
        // max(0, multiplicity - opposite capacity), also independent of order.
        // depth identifies the unassigned positional suffix. No phrase order,
        // cursor choice frontier, or coverage predicate affects this full count.
        let mut key = CountKey {
            depth: self.depth,
            choices: [0; 12],
        };
        key.choices[..self.depth].copy_from_slice(&self.chosen[..self.depth]);
        key.choices[..self.depth].sort_unstable();
        let cache = self
            .domain
            .count_cache
            .0
            .get_or_init(|| Mutex::new(CountCache::default()));
        if let Ok(cached) = cache.lock() {
            if let Some(&count) = cached.entries.get(&key) {
                #[cfg(test)]
                {
                    let mut cached = cached;
                    cached.hits += 1;
                }
                return count;
            }
        }
        // Do not hold the lock during DP: producer and completed-cursor clones
        // may count independently. Poisoning only disables caching, never search.
        let count = self.suffix_count_uncached();
        if let Ok(mut cached) = cache.lock() {
            if !cached.entries.contains_key(&key) {
                // Keep admitting states as the search moves into new regions.
                // FIFO eviction bounds memory without freezing the first region.
                if cached.entries.len() == COUNT_CACHE_LIMIT {
                    let oldest = cached.order.pop_front().expect("cache eviction order");
                    cached.entries.remove(&oldest);
                }
                cached.entries.insert(key, count);
                cached.order.push_back(key);
            }
        }
        count
    }

    fn suffix_count_uncached(&self) -> Option<u128> {
        let h = self.domain.holes.len() - self.depth;
        let (a, b) = self.domain.split.as_ref().map_or((0, 0), |s| {
            (
                s.quota[0] - self.minimum[self.depth][0],
                s.quota[1] - self.minimum[self.depth][1],
            )
        });
        let index = |n: usize, p: usize, v: usize| (n * (a + 1) + p) * (b + 1) + v;
        let mut dp = vec![0u128; (h + 1) * (a + 1) * (b + 1)];
        dp[0] = 1;
        for (c, choice) in self.domain.choices.iter().enumerate() {
            let remaining = self.remaining[c];
            let required = self.domain.required && self.domain.split.is_none();
            let min = if required { remaining } else { 0 };
            let max = if required && choice.fill {
                h
            } else {
                remaining.min(h)
            };
            let mut next = vec![0u128; dp.len()];
            for n in 0..=h {
                for p in 0..=a {
                    for v in 0..=b {
                        let ways = dp[index(n, p, v)];
                        if ways == 0 {
                            continue;
                        }
                        for k in min..=max.min(h - n) {
                            let (pp, vv) = self.domain.split.as_ref().map_or((p, v), |s| {
                                let used = choice.capacity - remaining;
                                (
                                    p + (used + k).saturating_sub(s.video[c])
                                        - used.saturating_sub(s.video[c]),
                                    v + (used + k).saturating_sub(s.post[c])
                                        - used.saturating_sub(s.post[c]),
                                )
                            });
                            if pp > a || vv > b {
                                continue;
                            }
                            let insertions = (0..k)
                                .fold(1u128, |x, i| x * (n + k - i) as u128 / (i + 1) as u128);
                            let entry = &mut next[index(n + k, pp, vv)];
                            *entry = entry.checked_add(ways.checked_mul(insertions)?)?;
                        }
                    }
                }
            }
            dp = next;
        }
        let mut total = 0u128;
        for p in 0..=a {
            for v in 0..=b {
                total = total.checked_add(dp[index(h, p, v)])?;
            }
        }
        Some(total)
    }
    pub fn next_pruned(&mut self, max_raw: usize, pruning: &dyn PrefixCoverage) -> Option<Step> {
        self.next_step(max_raw, Some(pruning))
    }
    fn next_step(&mut self, max_raw: usize, pruning: Option<&dyn PrefixCoverage>) -> Option<Step> {
        if self.done || max_raw == 0 {
            return None;
        }
        if self.domain.holes.is_empty() {
            self.done = true;
            return Some(if pruning.is_some_and(|p| p.covers(&self.phrase, 0)) {
                Step::Skipped(1)
            } else {
                Step::Candidate(self.phrase)
            });
        }
        loop {
            // A resumed cursor can be inside a covered subtree. Only an untouched
            // suffix may be counted wholesale; otherwise descend in original order.
            if self.next_choice[self.depth] == 0
                && pruning.is_some_and(|p| p.covers(&self.phrase, self.depth))
            {
                if let Some(n) = self.suffix_count().filter(|&n| n <= max_raw as u128) {
                    self.next_choice[self.depth] = 0;
                    if self.depth == 0 {
                        self.done = true;
                    } else {
                        self.backtrack();
                    }
                    if n > 0 {
                        return Some(Step::Skipped(n as usize));
                    }
                    if self.done {
                        return None;
                    }
                    continue;
                }
            }
            let limit = if self.free > 0 {
                self.domain.choices.len()
            } else {
                self.domain.pool_types
            };
            let mut choice = self.next_choice[self.depth];
            while choice < limit {
                if (self.remaining[choice] > 0
                    || (self.free > 0 && self.domain.choices[choice].fill))
                    && self.split_available(choice)
                {
                    break;
                }
                choice += 1;
            }
            if choice >= limit {
                self.next_choice[self.depth] = 0;
                if self.depth == 0 {
                    self.done = true;
                    return None;
                }
                self.backtrack();
                continue;
            }
            self.next_choice[self.depth] = choice + 1;
            self.chosen[self.depth] = choice;
            self.minimum[self.depth + 1] = self.split_minimum(choice);
            // Canonical assignment: consume required pool occurrences first.
            self.from_pool[self.depth] = self.remaining[choice] > 0;
            if self.from_pool[self.depth] {
                self.remaining[choice] -= 1;
            } else {
                self.free -= 1;
            }
            self.phrase[self.domain.holes[self.depth]] = self.domain.choices[choice].word;
            self.depth += 1;
            if self.depth == self.domain.holes.len() {
                let out = self.phrase;
                self.backtrack();
                // A complete leaf has already been generated; leave it to the
                // ordinary predicate instead of reporting it as generation saved.
                return Some(Step::Candidate(out));
            }
            self.next_choice[self.depth] = 0;
        }
    }
}
impl Iterator for Candidates {
    type Item = [u16; 12];
    fn next(&mut self) -> Option<Self::Item> {
        match self.next_step(usize::MAX, None)? {
            Step::Candidate(phrase) => Some(phrase),
            Step::Skipped(_) => unreachable!("plain traversal never prunes"),
        }
    }
}
impl std::iter::FusedIterator for Candidates {}
/// Count distinct words, not assignments to pools (which may overlap).
pub fn count_two_pools(
    slots: &[Slot; 12],
    post_open: usize,
    post: &[u16],
    video: &[u16],
) -> Option<u128> {
    let it = stream_two_pools(*slots, post_open, post.to_vec(), video.to_vec());
    if it.done {
        return Some(0);
    }
    let h = it.domain.holes.len();
    let split = it.domain.split.as_ref().unwrap();
    let [a, b] = split.quota;
    let index = |n: usize, p: usize, v: usize| (n * (a + 1) + p) * (b + 1) + v;
    let mut dp = vec![0u128; (h + 1) * (a + 1) * (b + 1)];
    dp[0] = 1;
    for (c, choice) in it.domain.choices.iter().enumerate() {
        let mut next = vec![0u128; dp.len()];
        for n in 0..=h {
            for p in 0..=a {
                for v in 0..=b {
                    let ways = dp[index(n, p, v)];
                    if ways == 0 {
                        continue;
                    }
                    for k in 0..=choice.capacity.min(h - n) {
                        let pp = p + k.saturating_sub(split.video[c]);
                        let vv = v + k.saturating_sub(split.post[c]);
                        if pp > a || vv > b {
                            continue;
                        }
                        let insertions =
                            (0..k).fold(1u128, |x, i| x * (n + k - i) as u128 / (i + 1) as u128);
                        let entry = &mut next[index(n + k, pp, vv)];
                        *entry = entry.checked_add(ways.checked_mul(insertions)?)?;
                    }
                }
            }
        }
        dp = next;
    }
    let mut total = 0u128;
    for p in 0..=a {
        for v in 0..=b {
            total = total.checked_add(dp[index(h, p, v)])?;
        }
    }
    Some(total)
}
/// Exact distinct-phrase count; None means overflow. Adding k equal words to
/// n positions has C(n+k,k) arrangements. Mandatory types precede fill-only types.
pub fn count(slots: &[Slot; 12], pool: &[u16], fill: &[u16]) -> Option<u128> {
    let d = domain(slots, pool, fill);
    let h = d.holes.len();
    let mut dp = [0u128; 13];
    dp[0] = 1;
    for c in &d.choices {
        let min = if d.required { c.capacity } else { 0 };
        let max = if d.required && c.fill {
            h
        } else {
            c.capacity.min(h)
        };
        let mut next = [0u128; 13];
        for n in 0..=h {
            if dp[n] == 0 {
                continue;
            }
            for k in min..=max.min(h - n) {
                let ways = (0..k).fold(1u128, |v, i| v * (n + k - i) as u128 / (i + 1) as u128);
                next[n + k] = next[n + k].checked_add(dp[n].checked_mul(ways)?)?;
            }
        }
        dp = next;
    }
    Some(dp[h])
}
#[cfg(test)]
mod tests {
    use super::*;
    use itertools::Itertools;
    use std::collections::HashSet;
    fn holes(n: usize) -> [Slot; 12] {
        let mut s = [Slot::Fixed(99); 12];
        s[..n].fill(Slot::Hole);
        s
    }

    fn restored_prefix(base: &Candidates, prefix: &[u16]) -> Candidates {
        assert!(prefix.len() < base.domain.holes.len());
        let mut cursor = base.cursor();
        cursor.depth = prefix.len();
        for (depth, &word) in prefix.iter().enumerate() {
            let choice = base
                .domain
                .choices
                .iter()
                .position(|c| c.word == word)
                .unwrap();
            cursor.chosen[depth] = choice;
            cursor.next_choice[depth] = choice + 1;
        }
        cursor.next_choice[prefix.len()] = 0;
        let encoded = serde_json::to_vec(&cursor).unwrap();
        let mut restored = base.clone();
        restored
            .restore(&serde_json::from_slice(&encoded).unwrap())
            .unwrap();
        restored
    }

    #[test]
    fn suffix_cache_matches_enumerated_prefixes_with_fill_and_batch_overlap() {
        let mut cases = vec![
            stream(holes(4), vec![1, 2, 3, 4, 5], vec![]),
            stream(holes(4), vec![1, 2], vec![1, 2, 3]),
            stream(holes(4), vec![1, 1], vec![1, 2]),
            stream(holes(4), vec![], vec![1, 2, 3]),
        ];
        for quota in 0..=4 {
            cases.push(stream_two_pools(
                holes(4),
                quota,
                vec![1, 1, 2, 3],
                vec![1, 2, 2, 4],
            ));
        }
        for base in cases {
            let phrases: Vec<_> = base.clone().collect();
            assert!(base.domain.count_cache.0.get().is_none());
            for depth in 0..4 {
                let prefixes: HashSet<Vec<_>> = phrases
                    .iter()
                    .map(|phrase| phrase[..depth].to_vec())
                    .collect();
                for prefix in prefixes {
                    let expected = phrases
                        .iter()
                        .filter(|phrase| phrase[..depth] == prefix)
                        .count() as u128;
                    let state = restored_prefix(&base, &prefix);
                    let cursor_before = serde_json::to_vec(&state.cursor()).unwrap();
                    assert_eq!(state.suffix_count(), Some(expected));
                    let mut uncached = state.clone();
                    uncached.set_count_cache(false);
                    assert_eq!(uncached.suffix_count(), Some(expected));
                    assert_eq!(state.clone().suffix_count(), Some(expected));
                    assert_eq!(serde_json::to_vec(&state.cursor()).unwrap(), cursor_before);
                }
            }
            assert!(
                base.domain
                    .count_cache
                    .0
                    .get()
                    .unwrap()
                    .lock()
                    .unwrap()
                    .hits
                    > 0
            );
        }
    }

    #[test]
    fn suffix_cache_reuses_permuted_prefixes_and_cloned_restored_cursors() {
        for base in [
            stream(holes(4), vec![1, 2, 3, 4], vec![]),
            stream(holes(4), vec![1, 2], vec![1, 2, 3]),
            stream_two_pools(holes(4), 2, vec![1, 1, 2, 3], vec![1, 2, 2, 4]),
        ] {
            let first = restored_prefix(&base, &[1, 2]);
            let mut permuted = restored_prefix(&base, &[2, 1]);
            let expected = base.clone().filter(|phrase| phrase[..2] == [1, 2]).count() as u128;
            assert_eq!(first.suffix_count(), Some(expected));
            let cache = base.domain.count_cache.0.get().unwrap();
            assert_eq!(cache.lock().unwrap().entries.len(), 1);
            assert_eq!(cache.lock().unwrap().hits, 0);
            assert_eq!(permuted.suffix_count(), Some(expected));
            assert_eq!(cache.lock().unwrap().entries.len(), 1);
            assert_eq!(cache.lock().unwrap().hits, 1);
            let restored_again = restored_prefix(&base, &[2, 1]);
            assert_eq!(restored_again.clone().suffix_count(), Some(expected));
            assert_eq!(cache.lock().unwrap().hits, 2);

            permuted.set_count_cache(false);
            let disabled_clone = permuted.clone();
            assert!(!disabled_clone.count_cache_enabled);
            assert_eq!(disabled_clone.suffix_count(), Some(expected));
            assert_eq!(cache.lock().unwrap().hits, 2);
            permuted.set_count_cache(true);
            assert_eq!(permuted.suffix_count(), Some(expected));
            assert_eq!(cache.lock().unwrap().hits, 3);
        }
    }

    #[test]
    fn suffix_cache_is_lazy_bounded_and_separate_from_cursor_and_other_domains() {
        let base = stream(holes(4), (0..48).collect(), vec![]);
        let mut disabled = base.clone();
        disabled.set_count_cache(false);
        assert_eq!(disabled.suffix_count(), Some(48 * 47 * 46 * 45));
        assert!(base.domain.count_cache.0.get().is_none());
        for prefix in (0u16..48).combinations(2) {
            let state = restored_prefix(&base, &prefix);
            assert_eq!(state.suffix_count(), Some(46 * 45));
        }
        assert_eq!(
            base.domain
                .count_cache
                .0
                .get()
                .unwrap()
                .lock()
                .unwrap()
                .entries
                .len(),
            COUNT_CACHE_LIMIT
        );
        // Saturation replaces old entries and still reuses new residual states.
        let oldest = *base
            .domain
            .count_cache
            .0
            .get()
            .unwrap()
            .lock()
            .unwrap()
            .order
            .front()
            .unwrap();
        let root_count = base.suffix_count();
        assert_eq!(root_count, Some(48 * 47 * 46 * 45));
        {
            let cached = base.domain.count_cache.0.get().unwrap().lock().unwrap();
            assert!(!cached.entries.contains_key(&oldest));
            assert_eq!(cached.order.len(), COUNT_CACHE_LIMIT);
        }
        let hits = base
            .domain
            .count_cache
            .0
            .get()
            .unwrap()
            .lock()
            .unwrap()
            .hits;
        assert_eq!(base.suffix_count(), root_count);
        assert_eq!(
            base.domain
                .count_cache
                .0
                .get()
                .unwrap()
                .lock()
                .unwrap()
                .hits,
            hits + 1
        );
        assert_eq!(
            base.domain
                .count_cache
                .0
                .get()
                .unwrap()
                .lock()
                .unwrap()
                .entries
                .len(),
            COUNT_CACHE_LIMIT
        );

        let mut separate = stream(holes(4), vec![0, 1, 2, 3], vec![]);
        let cursor = restored_prefix(&base, &[0, 1]).cursor();
        separate.restore(&cursor).unwrap();
        assert!(separate.domain.count_cache.0.get().is_none());
        assert_eq!(separate.suffix_count(), Some(2));
        assert_eq!(
            separate
                .domain
                .count_cache
                .0
                .get()
                .unwrap()
                .lock()
                .unwrap()
                .entries
                .len(),
            1
        );
        // A future Arc::make_mut must not share a cache with a changed Domain.
        let deep_domain_clone = base.domain.as_ref().clone();
        assert!(deep_domain_clone.count_cache.0.get().is_none());

        let overflow = stream(holes(12), vec![], (0..2048).collect());
        assert_eq!(overflow.suffix_count(), None);
        assert_eq!(overflow.suffix_count(), None);
        let cached = overflow.domain.count_cache.0.get().unwrap().lock().unwrap();
        assert_eq!(cached.entries.len(), 1);
        assert_eq!(cached.hits, 1);
    }
    // Independent upstream-style oracle: enumerate assignments, then dedup
    // phrases. Production enumeration never allocates or visits duplicates.
    fn two_oracle(h: usize, a: usize, post: &[u16], video: &[u16]) -> HashSet<[u16; 12]> {
        let mut out = HashSet::new();
        if a > h || post.len() < a || video.len() < h - a {
            return out;
        }
        for side in (0..h).combinations(a) {
            let other: Vec<_> = (0..h).filter(|i| !side.contains(i)).collect();
            for p in post.iter().permutations(a) {
                for v in video.iter().permutations(h - a) {
                    let mut phrase = [99; 12];
                    for (&pos, &&word) in side.iter().zip(&p) {
                        phrase[pos] = word;
                    }
                    for (&pos, &&word) in other.iter().zip(&v) {
                        phrase[pos] = word;
                    }
                    out.insert(phrase);
                }
            }
        }
        out
    }
    #[test]
    fn two_pools_exhaustive_overlap_repetitions_and_quotas() {
        for post_len in 0..=3 {
            for video_len in 0..=3 {
                for post in (0..post_len).map(|_| 0..2u16).multi_cartesian_product() {
                    for video in (0..video_len).map(|_| 0..2u16).multi_cartesian_product() {
                        for h in 0..=4 {
                            for a in 0..=h {
                                let got: Vec<_> =
                                    stream_two_pools(holes(h), a, post.clone(), video.clone())
                                        .collect();
                                let unique: HashSet<_> = got.iter().copied().collect();
                                let want = two_oracle(h, a, &post, &video);
                                assert_eq!(
                                    got.len(),
                                    unique.len(),
                                    "duplicate h={h} a={a} post={post:?} video={video:?}"
                                );
                                assert_eq!(
                                    unique, want,
                                    "coverage h={h} a={a} post={post:?} video={video:?}"
                                );
                                assert_eq!(
                                    count_two_pools(&holes(h), a, &post, &video),
                                    Some(got.len() as u128)
                                );
                            }
                        }
                    }
                }
            }
        }
    }
    #[test]
    fn two_pools_cursor_boundaries_and_upstream_shapes() {
        for (h, a, post, video) in [
            (4, 2, vec![1, 2, 3], vec![4, 5, 6]),
            (4, 2, vec![1, 1, 2], vec![1, 2, 2]),
            (3, 0, vec![], vec![1, 2, 3]),
            (0, 0, vec![1], vec![2]),
        ] {
            let mut it = stream_two_pools(holes(h), a, post.clone(), video.clone());
            let all: Vec<_> = it.clone().collect();
            for n in 0..=all.len() {
                let encoded = serde_json::to_vec(&it.cursor()).unwrap();
                let mut restored = stream_two_pools(holes(h), a, post.clone(), video.clone());
                restored
                    .restore(&serde_json::from_slice(&encoded).unwrap())
                    .unwrap();
                assert_eq!(restored.collect::<Vec<_>>(), all[n..]);
                it.next();
            }
            let mut restored = stream_two_pools(holes(h), a, post, video);
            restored.restore(&it.cursor()).unwrap();
            assert_eq!(restored.next(), None);
        }
        assert_eq!(
            count_two_pools(
                &holes(8),
                4,
                &(0..8).collect::<Vec<_>>(),
                &(8..16).collect::<Vec<_>>()
            ),
            Some(70 * 1680 * 1680)
        );
        // The upstream README example actually has SIX unpinned post words.
        assert_eq!(
            count_two_pools(
                &holes(8),
                4,
                &(0..6).collect::<Vec<_>>(),
                &(6..11).collect::<Vec<_>>()
            ),
            Some(3_024_000)
        );
        assert_eq!(
            count_two_pools(
                &holes(12),
                6,
                &(0..2048).collect::<Vec<_>>(),
                &(2048..4096).collect::<Vec<_>>()
            ),
            None
        );
    }
    // Independent oracle: all small phrases, accepted by multiset membership.
    fn oracle(h: usize, pool: &[u16], fill: &[u16]) -> HashSet<[u16; 12]> {
        let alphabet: Vec<_> = pool.iter().chain(fill).copied().unique().collect();
        let mut out = HashSet::new();
        for words in (0..h)
            .map(|_| alphabet.iter().copied())
            .multi_cartesian_product()
        {
            let mut unused = pool.to_vec();
            let mut extras = Vec::new();
            for &w in &words {
                if let Some(i) = unused.iter().position(|&p| p == w) {
                    unused.remove(i);
                } else {
                    extras.push(w);
                }
            }
            let valid = if pool.len() >= h {
                extras.is_empty()
            } else {
                unused.is_empty() && extras.iter().all(|w| fill.contains(w))
            };
            if valid {
                let mut c = [99; 12];
                c[..h].copy_from_slice(&words);
                out.insert(c);
            }
        }
        out
    }
    #[test]
    fn exhaustive_small_spaces_match_independent_oracle() {
        for h in 0..=4 {
            for p in 0..=4 {
                for pool in (0..p).map(|_| 0..3u16).multi_cartesian_product() {
                    for mask in 0..8 {
                        let fill: Vec<_> = (0..3u16).filter(|i| mask & (1 << i) != 0).collect();
                        let got: Vec<_> = stream(holes(h), pool.clone(), fill.clone()).collect();
                        let unique: HashSet<_> = got.iter().copied().collect();
                        assert_eq!(
                            unique.len(),
                            got.len(),
                            "duplicate h={h} pool={pool:?} fill={fill:?}"
                        );
                        assert_eq!(
                            unique,
                            oracle(h, &pool, &fill),
                            "coverage h={h} pool={pool:?} fill={fill:?}"
                        );
                        assert_eq!(count(&holes(h), &pool, &fill), Some(got.len() as u128));
                    }
                }
            }
        }
    }
    #[test]
    fn repetitions_overlap_and_overflow() {
        assert_eq!(count(&holes(3), &[10, 10, 20], &[]), Some(3));
        assert_eq!(count(&holes(2), &[10], &[10, 20]), Some(3));
        assert_eq!(stream(holes(2), vec![], vec![1, 1]).count(), 1);
        assert_eq!(count(&holes(12), &[], &(0..2048).collect::<Vec<_>>()), None);
        assert_eq!(
            count(&holes(12), &(0..12).collect::<Vec<_>>(), &[]),
            Some(479_001_600)
        );
        assert_eq!(
            count(&holes(12), &[0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1], &[]),
            Some(12)
        );
    }
    #[test]
    fn pins_clone_and_exhaustion() {
        let mut s = holes(4);
        s[1] = Slot::Fixed(777);
        let mut a = stream(s, vec![1, 2], vec![2, 3]);
        assert_eq!(a.next().unwrap()[1], 777);
        let b = a.clone();
        assert_eq!(a.collect::<Vec<_>>(), b.collect::<Vec<_>>());
        let mut empty = stream(holes(2), vec![], vec![]);
        assert_eq!(empty.next(), None);
        assert_eq!(empty.next(), None);
    }
    #[test]
    fn serialized_cursor_resumes_without_replay_or_gaps() {
        for (pool, fill) in [
            (vec![1, 2], vec![2, 3]),
            (vec![1, 1, 2, 3], vec![]),
            (vec![], vec![1, 2]),
        ] {
            let original = stream(holes(4), pool.clone(), fill.clone());
            let expected: Vec<_> = original.clone().collect();
            let mut it = original;
            for n in 0..=expected.len() {
                let json = serde_json::to_vec(&it.cursor()).unwrap();
                let cursor: Cursor = serde_json::from_slice(&json).unwrap();
                let mut restored = stream(holes(4), pool.clone(), fill.clone());
                restored.restore(&cursor).unwrap();
                assert_eq!(restored.collect::<Vec<_>>(), expected[n..]);
                it.next();
            }
            let mut restored = stream(holes(4), pool, fill);
            restored.restore(&it.cursor()).unwrap();
            assert_eq!(restored.next(), None);
        }
        let mut fresh = stream(holes(2), vec![1, 2], vec![]);
        let mut bad = fresh.cursor();
        bad.depth = 12;
        assert!(fresh.restore(&bad).is_err());
    }

    #[test]
    fn remaining_counts_and_ranks_match_every_small_cursor() {
        let domains = [
            stream(holes(4), vec![2, 1], vec![1, 3]),
            stream(holes(4), vec![3, 1, 1, 2, 4], vec![]),
            stream(holes(4), vec![], vec![2, 1]),
            stream_two_pools(holes(4), 2, vec![2, 1, 1], vec![1, 3, 2]),
            stream(holes(0), vec![], vec![]),
        ];
        for mut it in domains {
            let all: Vec<_> = it.clone().collect();
            for (n, phrase) in all.iter().enumerate() {
                let before = serde_json::to_vec(&it.cursor()).unwrap();
                assert_eq!(it.remaining_count(), Some((all.len() - n) as u128));
                assert_eq!(it.rank_of(phrase), Some(n as u128));
                assert_eq!(serde_json::to_vec(&it.cursor()).unwrap(), before);
                let mut restored = it.clone();
                assert_eq!(restored.next(), Some(*phrase));
                assert_eq!(it.next(), Some(*phrase));
            }
            assert_eq!(it.remaining_count(), Some(0));
            assert_eq!(it.next(), None);
            assert_eq!(it.remaining_count(), Some(0));
            for (n, phrase) in all.iter().enumerate() {
                assert_eq!(it.rank_of(phrase), Some(n as u128));
            }
            assert_eq!(it.rank_of(&[2047; 12]), None);
        }
    }

    #[test]
    fn frontier_counts_do_not_truncate_at_machine_word_size() {
        let total = 64u128.pow(12);
        let mut it = stream(holes(12), vec![], (0..64).collect());
        assert_eq!(it.remaining_count(), Some(total));
        assert_eq!(it.next(), Some([0; 12]));
        assert_eq!(it.remaining_count(), Some(total - 1));
        assert_eq!(it.rank_of(&[63; 12]), Some(total - 1));
        let before = serde_json::to_vec(&it.cursor()).unwrap();
        assert_eq!(it.rank_of(&[64; 12]), None);
        assert_eq!(serde_json::to_vec(&it.cursor()).unwrap(), before);
    }
}
