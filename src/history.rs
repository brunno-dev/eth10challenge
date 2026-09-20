//! Exact membership in an externally reported search, never a word blacklist.
use anyhow::{Context, Result};
use bip39::Language;
use bitcoin::hashes::{sha256, Hash};
use serde::Deserialize;

pub const CATALOG: &str = include_str!("../history/catalog.json");
const POOL: &str = include_str!("../history/ro1/reading-order-pool.json");
const TARGET: &str = "0x9c2f44efad0c1e852a09df9939e6daf061140caf";

#[derive(Deserialize)]
struct Pool {
    video_pre_fog: Vec<String>,
    video_between_fog_and_parrot: Vec<String>,
    post_reading_order_after_dutch: Vec<String>,
}

#[derive(Clone)]
pub struct Ro1 {
    pre: [u8; 2048],
    mid: [u8; 2048],
    post: [u8; 2048],
    anchors: [u16; 3],
    fiber: u16,
    fork: u16,
}

impl Ro1 {
    pub fn compatible(language: Language, target: &[u8; 20], no_checksum: bool) -> Result<()> {
        anyhow::ensure!(
            language == Language::English,
            "RO1 requires the English wordlist"
        );
        anyhow::ensure!(
            *target == crate::eth::parse_address(TARGET)?,
            "RO1 covers only the challenge target"
        );
        anyhow::ensure!(
            !no_checksum,
            "RO1 cannot exclude searches with --no-checksum"
        );
        anyhow::ensure!(
            crate::eth::ETH_PATH == "m/44'/60'/0'/0/0",
            "RO1 derivation path differs"
        );
        // Both current engines use the empty BIP-39 passphrase. Any future
        // passphrase selector must participate in this compatibility check.
        Ok(())
    }

    pub fn fingerprint() -> String {
        // Bump the model revision whenever membership semantics change.
        let input = format!("ro1-membership-v1\n{CATALOG}\n{POOL}");
        format!("RO1:{}", sha256::Hash::hash(input.as_bytes()))
    }

    pub fn new() -> Result<Self> {
        let data: Pool = serde_json::from_str(POOL)?;
        let words = Language::English.word_list();
        let index = |w: &str| -> Result<u16> {
            words
                .binary_search(&w)
                .map(|i| i as u16)
                .ok()
                .with_context(|| format!("invalid RO1 word {w}"))
        };
        let ranks = |list: &[String]| -> Result<[u8; 2048]> {
            let mut result = [0; 2048];
            for (rank, word) in list.iter().enumerate() {
                let i = index(word)? as usize;
                anyhow::ensure!(result[i] == 0, "duplicate word within RO1 source");
                result[i] = (rank + 1).try_into()?;
            }
            Ok(result)
        };
        anyhow::ensure!(
            (
                data.video_pre_fog.len(),
                data.video_between_fog_and_parrot.len(),
                data.post_reading_order_after_dutch.len()
            ) == (25, 2, 19),
            "RO1 pool shape changed"
        );
        let rule = Self {
            pre: ranks(&data.video_pre_fog)?,
            mid: ranks(&data.video_between_fog_and_parrot)?,
            post: ranks(&data.post_reading_order_after_dutch)?,
            anchors: [index("dutch")?, index("fog")?, index("parrot")?],
            fiber: index("fiber")?,
            fork: index("fork")?,
        };
        // This disjointness makes mid-video assignment unambiguous. Assert it
        // instead of silently broadening the rule when the source changes.
        for i in 0..2048 {
            anyhow::ensure!(
                rule.mid[i] == 0 || rule.post[i] == 0,
                "RO1 middle pools overlap"
            );
        }
        for w in rule.anchors.into_iter().chain([rule.fork]) {
            anyhow::ensure!(
                rule.pre[w as usize] == 0
                    && rule.mid[w as usize] == 0
                    && rule.post[w as usize] == 0,
                "RO1 anchor/fork occurs in ordered pools"
            );
        }
        anyhow::ensure!(
            rule.post[rule.fiber as usize] != 0 && rule.pre[rule.fiber as usize] == 0,
            "RO1 fiber assignment changed"
        );
        Ok(rule)
    }

    /// Matches arrangements enumerated by the archived Python generator.
    /// Checksum-invalid arrangements are covered structurally; the normal
    /// checksum filter already rejects them, so no derivation is lost.
    pub fn contains(&self, phrase: &[u16; 12]) -> bool {
        if [phrase[0], phrase[4], phrase[11]] != self.anchors
            || phrase.iter().any(|&w| w >= 2048)
            || phrase.iter().filter(|&&w| w == self.fork).count() != 1
            || phrase.iter().filter(|&&w| w == self.fiber).count() != 1
        {
            return false;
        }
        let mut mid_count = 0;
        let mut last_mid = 0;
        for &w in &phrase[5..11] {
            let rank = self.mid[w as usize];
            if rank != 0 {
                if rank <= last_mid {
                    return false;
                }
                last_mid = rank;
                mid_count += 1;
            }
        }
        if !(1..=2).contains(&mid_count) {
            return false;
        }
        // At most three possible pre-fog source assignments. Words shared
        // between video and post remain legal, including cross-source repeats.
        for mask in [0b011u8, 0b101, 0b110, 0b111] {
            if mask.count_ones() != 4 - mid_count {
                continue;
            }
            let mut last_pre = 0;
            let mut last_post = 0;
            let mut post_count = 0;
            let mut valid = true;
            for (pos, &w) in phrase.iter().enumerate().take(11).skip(1) {
                if pos == 4 {
                    continue;
                }
                let i = w as usize;
                if pos < 4 && mask & (1 << (pos - 1)) != 0 {
                    let rank = self.pre[i];
                    if rank <= last_pre {
                        valid = false;
                        break;
                    }
                    last_pre = rank;
                } else if w != self.fork && !(pos > 4 && self.mid[i] != 0) {
                    let rank = self.post[i];
                    if rank <= last_post {
                        valid = false;
                        break;
                    }
                    last_post = rank;
                    post_count += 1;
                }
            }
            if valid && post_count == 4 {
                return true;
            }
        }
        false
    }
}

#[derive(Clone)]
pub struct Exclusions {
    ro1: Option<Ro1>,
    local: std::sync::Arc<Vec<(crate::local_history::Record, crate::local_history::Domain)>>,
    index: Option<std::sync::Arc<MembershipIndex>>,
}

/// Intersect necessary word/position conditions 64 records at a time. A set bit
/// is only a possible match: exact multiplicity, source quotas and the partial
/// frontier are always checked by Domain::contains. This never proves coverage.
struct MembershipIndex {
    masks: Vec<u64>,
    blocks: usize,
}

impl MembershipIndex {
    fn new(local: &[(crate::local_history::Record, crate::local_history::Domain)]) -> Option<Self> {
        // Avoid overhead for short histories and bound the disposable index to
        // 6 MiB. Larger histories retain the exact linear membership fallback.
        if !(8..=2048).contains(&local.len()) {
            return None;
        }
        let blocks = local.len().div_ceil(64);
        let mut masks = vec![0u64; 12 * 2048 * blocks];
        for (record, (_, domain)) in local.iter().enumerate() {
            let bit = 1u64 << (record % 64);
            for position in 0..12 {
                for word in 0..2048u16 {
                    if domain.allows_word_at(position, word) {
                        masks[(position * 2048 + word as usize) * blocks + record / 64] |= bit;
                    }
                }
            }
        }
        Some(Self { masks, blocks })
    }

    fn contains(
        &self,
        phrase: &[u16; 12],
        local: &[(crate::local_history::Record, crate::local_history::Domain)],
    ) -> bool {
        let mut offsets = [0; 12];
        for (position, &word) in phrase.iter().enumerate() {
            if word >= 2048 {
                return false;
            }
            offsets[position] = (position * 2048 + word as usize) * self.blocks;
        }
        for block in 0..self.blocks {
            let mut possible = u64::MAX;
            for &offset in &offsets {
                possible &= self.masks[offset + block];
                if possible == 0 {
                    break;
                }
            }
            while possible != 0 {
                let bit = possible.trailing_zeros() as usize;
                if local[block * 64 + bit].1.contains(phrase) {
                    return true;
                }
                possible &= possible - 1;
            }
        }
        false
    }
}

struct ProofBudget {
    nodes: usize,
    checks: usize,
    deadline: Option<std::time::Instant>,
}
impl ProofBudget {
    fn expired(&self) -> bool {
        self.deadline
            .is_some_and(|deadline| std::time::Instant::now() >= deadline)
    }
    fn check(&mut self) -> bool {
        if self.checks == 0 || self.expired() {
            return false;
        }
        self.checks -= 1;
        true
    }
}
impl Exclusions {
    #[cfg(any(test, feature = "cuda"))]
    pub fn ro1() -> Result<Self> {
        Self::load(true, &[])
    }
    pub fn load(ro1: bool, paths: &[std::path::PathBuf]) -> Result<Self> {
        let mut local = Vec::new();
        let mut digests = std::collections::HashSet::new();
        for path in paths {
            let record = crate::local_history::Record::load(path)
                .with_context(|| format!("loading {}", path.display()))?;
            if digests.insert(record.digest()?) {
                let domain = crate::local_history::Domain::from_record(&record);
                local.push((record, domain));
            }
        }
        Ok(Self {
            ro1: if ro1 { Some(Ro1::new()?) } else { None },
            index: MembershipIndex::new(&local).map(std::sync::Arc::new),
            local: std::sync::Arc::new(local),
        })
    }
    pub fn compatible(
        &self,
        language: Language,
        target: &[u8; 20],
        no_checksum: bool,
    ) -> Result<()> {
        if self.ro1.is_some() {
            Ro1::compatible(language, target, no_checksum)?;
        }
        for (r, _) in self.local.iter() {
            r.compatible(language, target, no_checksum)?;
        }
        Ok(())
    }
    pub fn fingerprint(&self) -> Result<String> {
        if self.local.is_empty() && self.ro1.is_some() {
            return Ok(Ro1::fingerprint());
        }
        let mut ids = self
            .local
            .iter()
            .map(|(r, _)| r.digest())
            .collect::<Result<Vec<_>>>()?;
        ids.sort_unstable();
        if self.ro1.is_some() {
            ids.push(Ro1::fingerprint());
        }
        Ok(format!(
            "coverage-v1:{}",
            sha256::Hash::hash(ids.join("\n").as_bytes())
        ))
    }
    pub fn contains(&self, phrase: &[u16; 12]) -> bool {
        self.ro1.as_ref().is_some_and(|r| r.contains(phrase))
            || match &self.index {
                Some(index) => index.contains(phrase, &self.local),
                None => self.local.iter().any(|(_, d)| d.contains(phrase)),
            }
    }
    pub fn has_local_records(&self) -> bool {
        !self.local.is_empty()
    }
    /// Bounded, sufficient proof used while compiling enumeration prefixes.
    pub fn covers_prefix(
        &self,
        plan: &crate::plan::Plan,
        checks: &mut usize,
        deadline: std::time::Instant,
    ) -> bool {
        for (_, domain) in self.local.iter() {
            if *checks == 0 || std::time::Instant::now() >= deadline {
                return false;
            }
            *checks -= 1;
            if domain.covers_plan(plan) {
                return true;
            }
        }
        false
    }
    /// Conservative whole-domain proof. Unknown overlap falls back to per-row checks.
    pub fn covers_plan(&self, plan: &crate::plan::Plan) -> bool {
        self.covers_with_budget(
            plan,
            ProofBudget {
                nodes: 256,
                checks: 4096,
                deadline: Some(std::time::Instant::now() + std::time::Duration::from_millis(250)),
            },
        )
    }

    fn covers_with_budget(&self, plan: &crate::plan::Plan, mut budget: ProofBudget) -> bool {
        if plan.validate().is_err() || budget.nodes == 0 {
            return false;
        }
        budget.nodes -= 1; // root; children reserve their budget before allocation
        self.prove_union(plan, &mut budget)
    }

    fn prove_union(&self, plan: &crate::plan::Plan, budget: &mut ProofBudget) -> bool {
        if budget.expired() {
            return false;
        }
        let mut phrase = [0; 12];
        let mut fixed = true;
        for (i, slot) in plan.slots().iter().enumerate() {
            if let crate::candidates::Slot::Fixed(w) = slot {
                phrase[i] = *w;
            } else {
                fixed = false;
            }
        }
        if fixed && self.ro1.as_ref().is_some_and(|r| r.contains(&phrase)) {
            return true;
        }
        let mut possible = usize::from(self.ro1.as_ref().is_some_and(|r| {
            [0, 4, 11].into_iter().zip(r.anchors).all(|(i, word)|
                !matches!(plan.slots()[i], crate::candidates::Slot::Fixed(w) if w != word))
        }));
        // The general proof does not require canonicalization: avoid re-sorting
        // every record at every node of the union proof.
        for (record, domain) in self.local.iter() {
            if !budget.check() {
                return false;
            }
            possible += usize::from(record.plan.slots().iter().zip(plan.slots()).all(|(a,b)|
                !matches!((a,b), (crate::candidates::Slot::Fixed(x), crate::candidates::Slot::Fixed(y)) if x != y)));
            if if fixed {
                domain.contains(&phrase)
            } else {
                domain.covers_plan(plan)
            } {
                return true;
            }
        }
        if fixed
            || possible == 0
            || self.local.len() + usize::from(self.ro1.is_some()) < 2
            || budget.expired()
        {
            return false;
        }
        // Prefer a hole pinned by the records: such a split often proves a
        // whole region in one step. Ties choose the first hole deterministically.
        let position = (0..12)
            .filter(|&i| plan.slots()[i] == crate::candidates::Slot::Hole)
            .max_by_key(|&i| {
                (
                    self.local
                        .iter()
                        .filter(|(r, _)| {
                            matches!(r.plan.slots()[i], crate::candidates::Slot::Fixed(_))
                        })
                        .count(),
                    12 - i,
                )
            });
        let Some(position) = position else {
            return false;
        };
        let Some(children) = plan.split_slot(position, budget.nodes) else {
            return false;
        };
        // Reserve every sibling before descending: total constructed plans,
        // including pending siblings, can never exceed the global node limit.
        budget.nodes -= children.len();
        children.iter().all(|child| self.prove_union(child, budget))
    }
}

pub fn report(plan: &crate::plan::Plan, limit: usize, rule: &Exclusions) -> Result<()> {
    if rule.covers_plan(plan) {
        println!("History coverage: {0}/{0} arrangements; entire domain proven covered without enumeration", plan.count()?);
        return Ok(());
    }
    let mut it = plan.stream();
    let start = std::time::Instant::now();
    let mut examined = 0usize;
    let mut covered = 0usize;
    for phrase in it.by_ref().take(limit) {
        examined += 1;
        covered += usize::from(rule.contains(&phrase));
    }
    let exhausted = it.next().is_none();
    println!(
        "History coverage: {covered}/{examined} arrangements in {:.3}s; {}",
        start.elapsed().as_secs_f64(),
        if exhausted {
            "complete coverage inspection"
        } else {
            "prefix only; remaining overlap unknown"
        }
    );
    println!("No PBKDF2 derivations performed. Coverage uses the explicitly selected evidence.");
    Ok(())
}

#[cfg(test)]
#[path = "history_index_tests.rs"]
mod history_index_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use itertools::Itertools;

    fn local_rules(plans: Vec<crate::plan::Plan>) -> Exclusions {
        let local = plans
            .into_iter()
            .map(|plan| {
                let mut progress = crate::checkpoint::Progress::from_plan(
                    &plan, "english", [0; 20], false, None, false,
                )
                .unwrap();
                let mut cursor = progress.candidates.clone();
                let count = cursor.by_ref().count();
                progress.completed(count, &cursor).unwrap();
                let record = crate::local_history::Record::completed(
                    &plan, "english", [0; 20], false, &progress,
                )
                .unwrap();
                let domain = crate::local_history::Domain::new(&plan);
                (record, domain)
            })
            .collect::<Vec<_>>();
        Exclusions {
            ro1: None,
            index: MembershipIndex::new(&local).map(std::sync::Arc::new),
            local: std::sync::Arc::new(local),
        }
    }

    fn budget(nodes: usize, checks: usize) -> ProofBudget {
        ProofBudget {
            nodes,
            checks,
            deadline: None,
        }
    }

    #[test]
    fn union_proof_requires_every_branch_and_respects_global_budgets() {
        use crate::{candidates::Slot, plan::Plan};
        let mut slots = [Slot::Fixed(0); 12];
        slots[..2].fill(Slot::Hole);
        let plan = Plan::Template {
            slots,
            pool: vec![1, 2],
            fill: vec![],
        };
        let parts = plan.split_slot(0, 2).unwrap();
        let rules = local_rules(parts.clone());
        assert!(rules.local.iter().all(|(_, d)| !d.covers_plan(&plan)));
        assert!(rules.covers_with_budget(&plan, budget(3, 5)));
        assert!(!rules.covers_with_budget(&plan, budget(0, 5)));
        assert!(!rules.covers_with_budget(&plan, budget(1, 5)));
        assert!(!rules.covers_with_budget(&plan, budget(2, 5)));
        assert!(!rules.covers_with_budget(&plan, budget(3, 4)));
        assert!(!rules.covers_with_budget(&plan, budget(3, 0)));
        assert!(!rules.covers_with_budget(
            &plan,
            ProofBudget {
                nodes: 100,
                checks: 100,
                deadline: Some(std::time::Instant::now())
            }
        ));
        assert!(!local_rules(vec![parts[0].clone()]).covers_plan(&plan));

        let larger = Plan::Template {
            slots,
            pool: vec![1, 2, 3],
            fill: vec![],
        };
        let mut parts = larger.split_slot(0, 3).unwrap();
        assert!(local_rules(parts.clone()).covers_with_budget(&larger, budget(20, 100)));
        parts.pop();
        assert!(!local_rules(parts).covers_with_budget(&larger, budget(20, 100)));

        let mut nested_slots = slots;
        nested_slots[2..4].fill(Slot::Hole);
        let nested = Plan::Template {
            slots: nested_slots,
            pool: vec![1, 2, 3, 4],
            fill: vec![],
        };
        let mut parts = nested
            .split_slot(2, 4)
            .unwrap()
            .into_iter()
            .flat_map(|child| child.split_slot(1, 3).unwrap())
            .collect::<Vec<_>>();
        assert!(local_rules(parts.clone()).covers_with_budget(&nested, budget(100, 1000)));
        parts.pop();
        assert!(!local_rules(parts).covers_with_budget(&nested, budget(100, 1000)));
    }

    #[test]
    fn union_proof_overlapping_sources_and_fill_agree_with_sets() {
        use crate::{candidates::Slot, plan::Plan};
        use std::collections::HashSet;
        let mut slots = [Slot::Fixed(0); 12];
        slots[..3].fill(Slot::Hole);
        let pools: Vec<Vec<u16>> = (0..=3)
            .flat_map(|n| (0..3).combinations_with_replacement(n))
            .collect();
        let mut cases = 0;
        for (i, pool) in pools.iter().enumerate() {
            for (j, other) in pools.iter().enumerate() {
                let plan = if (i + j) % 2 == 0 {
                    Plan::Template {
                        slots,
                        pool: pool.clone(),
                        fill: other.clone(),
                    }
                } else {
                    Plan::Batches {
                        slots,
                        post: pool.clone(),
                        video: other.clone(),
                        post_open: (i + j) % 4,
                    }
                };
                let parts = plan.split_slot((i + j) % 3, 6).unwrap();
                if parts.len() < 2 {
                    continue;
                }
                let rules = local_rules(parts.clone());
                let source: HashSet<_> = plan.stream().collect();
                assert!(
                    rules.covers_with_budget(&plan, budget(512, 4096)),
                    "complete split {plan:?}"
                );
                for removed in 0..parts.len() {
                    let remaining: Vec<_> = parts
                        .iter()
                        .enumerate()
                        .filter(|(k, _)| *k != removed)
                        .map(|(_, p)| p.clone())
                        .collect();
                    let covered: HashSet<_> = remaining.iter().flat_map(|p| p.stream()).collect();
                    let expected = source.is_subset(&covered);
                    let got = local_rules(remaining).covers_with_budget(&plan, budget(512, 4096));
                    // Single-record pin alignment can conservatively return
                    // unknown; every positive still requires actual inclusion.
                    assert!(
                        !got || expected,
                        "false proof after removing {removed}: {plan:?}"
                    );
                }
                cases += 1;
            }
        }
        assert!(cases > 200);
    }

    fn ids(phrase: &str) -> [u16; 12] {
        phrase
            .split_whitespace()
            .map(|w| Language::English.word_list().binary_search(&w).unwrap() as u16)
            .collect::<Vec<_>>()
            .try_into()
            .unwrap()
    }

    // Independent oracle: try every partition of the nine holes. Validate the
    // resulting two source subsequences directly, without the optimized masks.
    fn oracle(rule: &Ro1, p: &[u16; 12]) -> bool {
        let holes = [1, 2, 3, 5, 6, 7, 8, 9, 10];
        if [p[0], p[4], p[11]] != rule.anchors {
            return false;
        }
        for chosen in holes.into_iter().combinations(4) {
            let mut before = vec![];
            let mut after = vec![];
            let mut post = vec![];
            for pos in holes {
                if chosen.contains(&pos) {
                    if pos < 4 {
                        before.push(rule.pre[p[pos] as usize]);
                    } else {
                        after.push(rule.mid[p[pos] as usize]);
                    }
                } else {
                    post.push(p[pos]);
                }
            }
            let ordered = |v: &[u8]| !v.is_empty() && v[0] > 0 && v.windows(2).all(|x| x[0] < x[1]);
            if !ordered(&before) || !ordered(&after) {
                continue;
            }
            if post.iter().filter(|&&w| w == rule.fork).count() != 1 || !post.contains(&rule.fiber)
            {
                continue;
            }
            let ranks: Vec<_> = post
                .into_iter()
                .filter(|&w| w != rule.fork)
                .map(|w| rule.post[w as usize])
                .collect();
            if ordered(&ranks) {
                return true;
            }
        }
        false
    }

    #[test]
    fn exact_membership_and_mutations() {
        let rule = Ro1::new().unwrap();
        let examples = [
            "dutch update winter cattle fog lake also forest wood fiber fork parrot",
            "dutch update winter follow fog lake cattle forest wood fiber fork parrot",
            // The same word appears once in each source. It must stay covered.
            "dutch video seed cattle fog lake also fiber video word fork parrot",
            "dutch update fork winter fog lake also cattle forest wood fiber parrot",
        ];
        for example in examples {
            let p = ids(example);
            assert!(rule.contains(&p));
            assert!(oracle(&rule, &p));
            for a in 0..12 {
                for b in 0..12 {
                    let mut changed = p;
                    changed.swap(a, b);
                    assert_eq!(
                        rule.contains(&changed),
                        oracle(&rule, &changed),
                        "{example}: swap {a}/{b}"
                    );
                }
            }
        }
        let mut state = 47u64;
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        let choices: Vec<_> = (0u16..2048)
            .filter(|&w| {
                rule.pre[w as usize] > 0 || rule.post[w as usize] > 0 || rule.mid[w as usize] > 0
            })
            .chain([rule.fork, rule.fiber, 0])
            .collect();
        for _ in 0..10_000 {
            let mut p = ids(examples[0]);
            for _ in 0..(next() % 6 + 1) {
                p[(next() % 12) as usize] = choices[(next() as usize) % choices.len()];
            }
            assert_eq!(rule.contains(&p), oracle(&rule, &p));
        }
        let mut invalid = ids(examples[0]);
        invalid[2] = 2048;
        assert!(!rule.contains(&invalid));
    }

    #[test]
    fn configuration_and_evidence() {
        let target = crate::eth::parse_address(TARGET).unwrap();
        assert!(Ro1::compatible(Language::English, &target, false).is_ok());
        assert!(Ro1::compatible(Language::English, &target, true).is_err());
        assert!(Ro1::compatible(Language::Portuguese, &target, false).is_err());
        assert!(Ro1::compatible(Language::English, &[0; 20], false).is_err());
        let catalog: serde_json::Value = serde_json::from_str(CATALOG).unwrap();
        for evidence in catalog["evidence"].as_array().unwrap() {
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("history")
                .join(evidence["file"].as_str().unwrap());
            let bytes = std::fs::read(path).unwrap();
            assert_eq!(
                sha256::Hash::hash(&bytes).to_string(),
                evidence["sha256"].as_str().unwrap()
            );
        }
        let normalized = Language::English.word_list().join("\n") + "\n";
        assert_eq!(
            sha256::Hash::hash(normalized.as_bytes()).to_string(),
            "2f5eed53a4727b4bf8880d8f3f199efc90e58503646d9ff8eff3a2ed3b24dbda"
        );
    }

    #[test]
    #[ignore = "full 167,688,000-arrangement reconstruction; no PBKDF2"]
    fn audit_full_ro1() {
        let rule = Ro1::new().unwrap();
        let data: Pool = serde_json::from_str(POOL).unwrap();
        let index = |w: &String| {
            Language::English
                .word_list()
                .binary_search(&w.as_str())
                .unwrap() as u16
        };
        let pre: Vec<_> = data.video_pre_fog.iter().map(index).collect();
        let mid: Vec<_> = data
            .video_between_fog_and_parrot
            .iter()
            .map(index)
            .collect();
        let post: Vec<_> = data
            .post_reading_order_after_dutch
            .iter()
            .map(index)
            .filter(|&w| w != rule.fiber)
            .collect();
        let mut arrangements = 0u64;
        let mut valid = 0u64;
        for k in [2, 3] {
            for pre_pos in [1, 2, 3].into_iter().combinations(k) {
                for mid_pos in (5..11).combinations(4 - k) {
                    let vpos: Vec<_> = pre_pos.iter().chain(&mid_pos).copied().collect();
                    let ppos: Vec<_> = [1, 2, 3, 5, 6, 7, 8, 9, 10]
                        .into_iter()
                        .filter(|p| !vpos.contains(p))
                        .collect();
                    for fork_pos in &ppos {
                        let ordered_pos: Vec<_> =
                            ppos.iter().filter(|p| *p != fork_pos).copied().collect();
                        for mut post_words in post.iter().copied().combinations(3) {
                            post_words.push(rule.fiber);
                            post_words.sort_by_key(|&w| rule.post[w as usize]);
                            let mut p = [0; 12];
                            p[0] = rule.anchors[0];
                            p[4] = rule.anchors[1];
                            p[11] = rule.anchors[2];
                            p[*fork_pos] = rule.fork;
                            for (&pos, &w) in ordered_pos.iter().zip(&post_words) {
                                p[pos] = w;
                            }
                            for before in pre.iter().copied().combinations(k) {
                                for after in mid.iter().copied().combinations(4 - k) {
                                    for (&pos, &w) in vpos.iter().zip(before.iter().chain(&after)) {
                                        p[pos] = w;
                                    }
                                    assert!(rule.contains(&p));
                                    arrangements += 1;
                                    valid += u64::from(crate::eth::checked_entropy(&p).is_some());
                                }
                            }
                        }
                    }
                }
            }
            eprintln!("RO1 shape {k}/{} reconstructed: {arrangements} arrangements, {valid} checksum survivors so far", 4-k);
        }
        assert_eq!(arrangements, 167_688_000);
        assert_eq!(valid, 10_484_919);
    }
}
