//! Confirmed negative search domains, including exact completed prefixes.
use crate::{candidates::Slot, plan::Plan};
use anyhow::{Context, Result};
use bitcoin::hashes::{sha256, Hash};
use serde::{Deserialize, Serialize};
use std::{fs, io::Write, path::Path};

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Record {
    version: u32,
    result: String,
    pub plan: Plan,
    language: String,
    target: [u8; 20],
    derivation_path: String,
    passphrase: String,
    no_checksum: bool,
    completed_candidates: u128,
    engine_policy: String,
    // Omit this field on v1 records so their existing serialized digest survives.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    next_phrase: Option<[u16; 12]>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    record: Record,
    sha256: String,
}

impl Record {
    pub fn completed(
        plan: &Plan,
        language: &str,
        target: [u8; 20],
        no_checksum: bool,
        progress: &crate::checkpoint::Progress,
    ) -> Result<Self> {
        anyhow::ensure!(
            progress.exclusion.is_none() && progress.excluded == 0,
            "local negative records require a search without exclusions"
        );
        anyhow::ensure!(
            progress.candidates.clone().next().is_none() && progress.checked == plan.count()?,
            "cannot record an incomplete search"
        );
        Self::confirmed(plan, language, target, no_checksum, progress.checked, None)
    }

    /// Certify only the caller's confirmed negative work. `next` is the first
    /// unprocessed phrase from a clone of the committed cursor, never queued work.
    /// Dependencies of transitive negative evidence must be checked by the caller.
    pub fn confirmed(
        plan: &Plan,
        language: &str,
        target: [u8; 20],
        no_checksum: bool,
        checked: u128,
        next: Option<[u16; 12]>,
    ) -> Result<Self> {
        let partial = next.is_some();
        let result = Self {
            version: if partial { 2 } else { 1 },
            result: if partial {
                "partial_negative"
            } else {
                "complete_negative"
            }
            .into(),
            // Partial coverage depends on encounter order, not sorted word ids.
            plan: if partial {
                plan.clone()
            } else {
                plan.canonical()
            },
            language: language.to_lowercase(),
            target,
            derivation_path: crate::eth::ETH_PATH.into(),
            passphrase: String::new(),
            no_checksum,
            completed_candidates: checked,
            engine_policy: if partial {
                "words-breaker-local-v2"
            } else {
                "words-breaker-local-v1"
            }
            .into(),
            next_phrase: next,
        };
        result.validate()?;
        Ok(result)
    }

    pub fn is_complete(&self) -> bool {
        self.version == 1 && self.next_phrase.is_none()
    }

    pub fn is_compatible(
        &self,
        language: bip39::Language,
        target: &[u8; 20],
        no_checksum: bool,
    ) -> bool {
        self.compatible(language, target, no_checksum).is_ok()
    }

    fn validate(&self) -> Result<()> {
        self.plan.validate()?;
        let total = self.plan.count()?;
        match (self.version, self.next_phrase) {
            (1, None) => {
                anyhow::ensure!(
                    self.result == "complete_negative"
                        && self.engine_policy == "words-breaker-local-v1",
                    "unsupported complete local record"
                );
                anyhow::ensure!(
                    self.completed_candidates == total,
                    "local record coverage count differs"
                );
            }
            (2, Some(next)) => {
                anyhow::ensure!(
                    self.result == "partial_negative"
                        && self.engine_policy == "words-breaker-local-v2",
                    "unsupported partial local record"
                );
                anyhow::ensure!(
                    self.completed_candidates > 0 && self.completed_candidates < total,
                    "partial local record needs a nonempty, unfinished prefix"
                );
                anyhow::ensure!(
                    Domain::new(&self.plan).contains(&next),
                    "partial record boundary is outside its domain"
                );
                anyhow::ensure!(
                    self.plan.stream().rank_of(&next) == Some(self.completed_candidates),
                    "partial record count does not match its enumeration boundary"
                );
            }
            _ => anyhow::bail!("unsupported local record version or boundary"),
        }
        Ok(())
    }
    pub fn digest(&self) -> Result<String> {
        Ok(sha256::Hash::hash(&serde_json::to_vec(self)?).to_string())
    }
    pub fn compatible(
        &self,
        language: bip39::Language,
        target: &[u8; 20],
        no_checksum: bool,
    ) -> Result<()> {
        anyhow::ensure!(
            crate::parse_language(&self.language)? == language
                && self.target == *target
                && self.no_checksum == no_checksum
                && self.derivation_path == crate::eth::ETH_PATH
                && self.passphrase.is_empty(),
            "local record target, language or derivation policy differs"
        );
        Ok(())
    }
    pub fn load(path: &Path) -> Result<Self> {
        anyhow::ensure!(
            fs::metadata(path)?.len() <= 4 * 1024 * 1024,
            "local record too large"
        );
        let envelope: Envelope =
            serde_json::from_slice(&fs::read(path)?).context("invalid local history record")?;
        let r = envelope.record;
        anyhow::ensure!(
            r.digest()? == envelope.sha256,
            "local record digest mismatch"
        );
        r.validate()?;
        Ok(r)
    }
    pub fn save(&self, path: &Path) -> Result<()> {
        // Publish atomically without replacing an existing record, even if a
        // second process races with the initial CLI existence check.
        let temp = path.with_extension(format!(
            "{}-{}.tmp",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_nanos()
        ));
        // If create_new fails, no cleanup may touch someone else's temp file.
        let mut f = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        let result = (|| -> Result<()> {
            f.write_all(&serde_json::to_vec_pretty(&Envelope {
                record: self.clone(),
                sha256: self.digest()?,
            })?)?;
            f.sync_all()?;
            drop(f);
            fs::hard_link(&temp, path).context("publishing local record without overwriting")?;
            Ok(())
        })();
        let _ = fs::remove_file(&temp);
        result
    }

    /// Publish a mutable progress record atomically, retaining the old file on
    /// serialization or I/O failure. Unrelated or regressing evidence is rejected.
    pub fn save_replace(&self, path: &Path) -> Result<()> {
        self.validate()?;
        if !path.exists() {
            // A racing creator is not overwritten by the initial publication.
            return self.save(path);
        }
        let previous = Self::load(path)?;
        anyhow::ensure!(
            self.target == previous.target
                && self.language == previous.language
                && self.no_checksum == previous.no_checksum
                && self.derivation_path == previous.derivation_path
                && self.passphrase == previous.passphrase
                && if self.is_complete() {
                    self.plan == previous.plan.canonical()
                } else {
                    !previous.is_complete() && self.plan == previous.plan
                },
            "cannot replace an unrelated local progress record"
        );
        anyhow::ensure!(
            self.completed_candidates >= previous.completed_candidates,
            "cannot replace local progress with an earlier prefix"
        );
        let bytes = serde_json::to_vec_pretty(&Envelope {
            record: self.clone(),
            sha256: self.digest()?,
        })?;
        let temp = path.with_extension(format!(
            "{}-{}.progress.tmp",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_nanos()
        ));
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        let result = (|| -> Result<()> {
            file.write_all(&bytes)?;
            file.sync_all()?;
            drop(file);
            fs::rename(&temp, path).context("replacing local progress record")?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temp);
        }
        result
    }
}

/// Compiled, exact membership. Capacities are per source, not word bans.
#[derive(Clone)]
pub struct Domain {
    slots: [Slot; 12],
    post: Box<[usize; 2048]>,
    other: Box<[usize; 2048]>,
    mode: Mode,
    prefix: Option<Prefix>,
}

#[derive(Clone)]
struct Prefix {
    rank: Box<[u16; 2048]>,
    next: [u16; 12],
}
#[derive(Clone)]
enum Mode {
    Pool,
    Fill(usize),
    Batches(usize, usize),
}
impl Domain {
    pub fn new(plan: &Plan) -> Self {
        let mut post = Box::new([0; 2048]);
        let mut other = Box::new([0; 2048]);
        let holes = plan.slots().iter().filter(|s| **s == Slot::Hole).count();
        let mode = match plan {
            Plan::Template { pool, fill, .. } => {
                for &w in pool {
                    post[w as usize] += 1;
                }
                for &w in fill {
                    other[w as usize] = 1;
                }
                if pool.len() < holes {
                    Mode::Fill(pool.len())
                } else {
                    Mode::Pool
                }
            }
            Plan::Batches {
                post: p,
                video,
                post_open,
                ..
            } => {
                for &w in p {
                    post[w as usize] += 1;
                }
                for &w in video {
                    other[w as usize] += 1;
                }
                Mode::Batches(*post_open, holes - post_open)
            }
        };
        Self {
            slots: *plan.slots(),
            post,
            other,
            mode,
            prefix: None,
        }
    }

    /// Include the record's exact boundary; `new` intentionally denotes a full
    /// structural plan and must not be used to load partial negative evidence.
    pub fn from_record(record: &Record) -> Self {
        let mut domain = Self::new(&record.plan);
        if let Some(next) = record.next_phrase {
            let mut rank = Box::new([u16::MAX; 2048]);
            let mut position = 0u16;
            let mut encounter = |words: &[u16]| {
                for &word in words {
                    if rank[word as usize] == u16::MAX {
                        rank[word as usize] = position;
                        position += 1;
                    }
                }
            };
            match &record.plan {
                Plan::Template { pool, fill, .. } => {
                    encounter(pool);
                    let holes = record
                        .plan
                        .slots()
                        .iter()
                        .filter(|&&s| s == Slot::Hole)
                        .count();
                    if pool.len() < holes {
                        encounter(fill);
                    }
                }
                Plan::Batches { post, video, .. } => {
                    encounter(post);
                    encounter(video);
                }
            }
            domain.prefix = Some(Prefix { rank, next });
        }
        domain
    }
    /// Necessary positional membership only. Source quotas, repeated-word
    /// capacities and partial boundaries still need `contains` after indexing.
    pub fn allows_word_at(&self, position: usize, word: u16) -> bool {
        if word >= 2048 {
            return false;
        }
        match self.slots[position] {
            Slot::Fixed(fixed) => fixed == word,
            Slot::Hole => {
                self.post[word as usize] > 0
                    || (!matches!(self.mode, Mode::Pool) && self.other[word as usize] > 0)
            }
        }
    }

    pub fn contains(&self, phrase: &[u16; 12]) -> bool {
        // An ordered negative prefix cannot contain its boundary or anything
        // after it. Reject those before rebuilding word multiplicities. Earlier
        // phrases still require the full structural/source-capacity check below.
        if let Some(prefix) = &self.prefix {
            let mut earlier = false;
            for (position, &slot) in self.slots.iter().enumerate() {
                if slot != Slot::Hole {
                    continue;
                }
                let Some(rank) = prefix.rank.get(phrase[position] as usize) else {
                    return false;
                };
                match rank.cmp(&prefix.rank[prefix.next[position] as usize]) {
                    std::cmp::Ordering::Less => {
                        earlier = true;
                        break;
                    }
                    std::cmp::Ordering::Greater => return false,
                    std::cmp::Ordering::Equal => (),
                }
            }
            if !earlier {
                return false;
            }
        }
        let mut words = [0u16; 12];
        let mut counts = [0usize; 12];
        let mut used = 0;
        for (&slot, &word) in self.slots.iter().zip(phrase) {
            if word >= 2048 {
                return false;
            }
            match slot {
                Slot::Fixed(w) => {
                    if w != word {
                        return false;
                    }
                }
                Slot::Hole => {
                    // Most records have different pools. A single absent word
                    // proves nonmembership without counting the remaining words.
                    if self.post[word as usize] == 0
                        && (matches!(self.mode, Mode::Pool) || self.other[word as usize] == 0)
                    {
                        return false;
                    }
                    let index = words[..used]
                        .iter()
                        .position(|&w| w == word)
                        .unwrap_or_else(|| {
                            words[used] = word;
                            used += 1;
                            used - 1
                        });
                    counts[index] += 1;
                }
            }
        }
        let mut consumed = 0;
        let mut minimum_post = 0;
        let mut minimum_video = 0;
        for (&word, &count) in words[..used].iter().zip(&counts[..used]) {
            let p = self.post[word as usize];
            let v = self.other[word as usize];
            match self.mode {
                Mode::Pool => {
                    if count > p {
                        return false;
                    }
                }
                Mode::Fill(_) => {
                    consumed += count.min(p);
                    if count > p && v == 0 {
                        return false;
                    }
                }
                Mode::Batches(_, _) => {
                    if count > p + v {
                        return false;
                    }
                    minimum_post += count.saturating_sub(v);
                    minimum_video += count.saturating_sub(p);
                }
            }
        }
        match self.mode {
            Mode::Pool => true,
            Mode::Fill(required) => consumed == required,
            Mode::Batches(p, v) => minimum_post <= p && minimum_video <= v,
        }
    }

    /// Prove that every phrase of `query` belongs to this confirmed domain.
    /// Pins already present in the record must remain fixed. For compatible
    /// pins, optimize each possible violation over query word multiplicities,
    /// without visiting permutations. Independent maxima need not share a
    /// witness: each must satisfy its bound for *all* query phrases.
    pub fn covers_plan(&self, query: &Plan) -> bool {
        if query.validate().is_err() {
            return false;
        }
        if let Some(prefix) = &self.prefix {
            // Preserve exact singleton membership, including the exclusive
            // boundary. A larger query additionally needs a prefix proof and
            // the complete structural proof below; neither is sufficient alone.
            let mut phrase = [0; 12];
            let mut fixed = true;
            for (i, &slot) in query.slots().iter().enumerate() {
                match slot {
                    Slot::Fixed(word) => phrase[i] = word,
                    Slot::Hole => fixed = false,
                }
            }
            if fixed {
                return self.contains(&phrase);
            }
            if !self.fixed_prefix_precedes_boundary(query, prefix) {
                return false;
            }
        }
        let mut pinned = [0usize; 2048];
        for (&old, &new) in self.slots.iter().zip(query.slots()) {
            match (old, new) {
                (Slot::Fixed(a), Slot::Fixed(b)) if a == b => (),
                (Slot::Fixed(_), _) => return false,
                (Slot::Hole, Slot::Fixed(w)) => pinned[w as usize] += 1,
                (Slot::Hole, Slot::Hole) => (),
            }
        }
        let q = Self::new(query);
        let holes = query.slots().iter().filter(|s| **s == Slot::Hole).count();
        let (quota_p, quota_v) = match q.mode {
            Mode::Batches(p, v) => (p, v),
            _ => (holes, 0),
        };
        let width = quota_v + 1;
        let size = (quota_p + 1) * width;
        // Per state: maximal count of word-capacity violations, maximal
        // required post contribution, maximal required video contribution.
        // -1 denotes an unreachable state. All scores are <= 12 for valid plans.
        let mut best = vec![[-1i16; 3]; size];
        let mut next = best.clone();
        best[0] = [0; 3];
        let mut options = Vec::new();
        for (word, &fixed) in pinned.iter().enumerate() {
            options.clear();
            let (min_p, max_p, max_v) = match q.mode {
                Mode::Pool => (0, q.post[word].min(quota_p), 0),
                Mode::Fill(_) => (
                    q.post[word],
                    if q.other[word] > 0 {
                        quota_p
                    } else {
                        q.post[word].min(quota_p)
                    },
                    0,
                ),
                Mode::Batches(_, _) => (0, q.post[word].min(quota_p), q.other[word].min(quota_v)),
            };
            for p in min_p..=max_p {
                for v in 0..=max_v {
                    let n = fixed + p + v;
                    let cap_p = self.post[word];
                    let cap_v = self.other[word];
                    let score = match self.mode {
                        Mode::Pool => [i16::from(n > cap_p), 0, 0],
                        Mode::Fill(_) => [i16::from(n < cap_p || (n > cap_p && cap_v == 0)), 0, 0],
                        Mode::Batches(_, _) => [
                            i16::from(n > cap_p + cap_v),
                            n.saturating_sub(cap_v) as i16,
                            n.saturating_sub(cap_p) as i16,
                        ],
                    };
                    options.push((p, v, score));
                }
            }
            if options.as_slice() == [(0, 0, [0; 3])] {
                continue;
            }
            next.fill([-1; 3]);
            for used_p in 0..=quota_p {
                for used_v in 0..=quota_v {
                    let previous = best[used_p * width + used_v];
                    if previous[0] < 0 {
                        continue;
                    }
                    for &(p, v, score) in &options {
                        if used_p + p > quota_p || used_v + v > quota_v {
                            continue;
                        }
                        let dest = &mut next[(used_p + p) * width + used_v + v];
                        for i in 0..3 {
                            dest[i] = dest[i].max(previous[i] + score[i]);
                        }
                    }
                }
            }
            std::mem::swap(&mut best, &mut next);
        }
        let maximum = best[quota_p * width + quota_v];
        if maximum[0] < 0 {
            return true;
        } // empty query domain
        if maximum[0] != 0 {
            return false;
        }
        match self.mode {
            Mode::Batches(p, v) => maximum[1] <= p as i16 && maximum[2] <= v as i16,
            _ => true,
        }
    }

    /// Sufficient order proof only: every old hole before the first difference
    /// must be fixed to the boundary value, and that difference must be smaller
    /// in the record's encounter order. An earlier unassigned hole is unknown.
    fn fixed_prefix_precedes_boundary(&self, query: &Plan, prefix: &Prefix) -> bool {
        for (position, &slot) in self.slots.iter().enumerate() {
            if slot != Slot::Hole {
                continue;
            }
            let Slot::Fixed(word) = query.slots()[position] else {
                return false;
            };
            match prefix.rank[word as usize].cmp(&prefix.rank[prefix.next[position] as usize]) {
                std::cmp::Ordering::Less => return true,
                std::cmp::Ordering::Greater => return false,
                std::cmp::Ordering::Equal => (),
            }
        }
        false // Equal to the first unprocessed phrase is never covered.
    }
}

#[cfg(test)]
#[path = "partial_coverage_tests.rs"]
mod partial_coverage_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use itertools::Itertools;
    use std::collections::HashSet;

    fn temporary_record(label: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "local-history-{label}-{}-{}.json",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn partial_prefix_matches_every_boundary_in_original_order_across_modes() {
        let mut slots = [Slot::Fixed(0); 12];
        for position in [1, 4, 9] {
            slots[position] = Slot::Hole;
        }
        let mut plans = vec![
            Plan::Template {
                slots,
                pool: vec![3, 1, 2, 0],
                fill: vec![],
            },
            Plan::Template {
                slots,
                pool: vec![3, 3, 1, 2],
                fill: vec![],
            },
            Plan::Template {
                slots,
                pool: vec![3, 1],
                fill: vec![2, 3, 0, 1],
            },
            Plan::Template {
                slots,
                pool: vec![],
                fill: vec![3, 1, 2, 0, 3],
            },
        ];
        for quota in 0..=3 {
            plans.push(Plan::Batches {
                slots,
                post: vec![3, 1, 3, 2],
                video: vec![2, 3, 0, 1],
                post_open: quota,
            });
        }
        for plan in plans {
            let sequence: Vec<_> = plan.stream().collect();
            for cutoff in 1..sequence.len() {
                let record = Record::confirmed(
                    &plan,
                    "english",
                    [0; 20],
                    false,
                    cutoff as u128,
                    Some(sequence[cutoff]),
                )
                .unwrap();
                assert!(!record.is_complete());
                assert_eq!(
                    record.plan, plan,
                    "partial evidence preserves encounter order"
                );
                let domain = Domain::from_record(&record);
                let expected: HashSet<_> = sequence[..cutoff].iter().copied().collect();
                assert!(
                    !domain.covers_plan(&plan),
                    "a partial record cannot certify the full plan"
                );
                for mut code in 0..64 {
                    let mut phrase = [0; 12];
                    for position in [1, 4, 9] {
                        phrase[position] = code % 4;
                        code /= 4;
                    }
                    assert_eq!(
                        domain.contains(&phrase),
                        expected.contains(&phrase),
                        "{plan:?}, cutoff {cutoff}, phrase {phrase:?}"
                    );
                    let fixed = Plan::Template {
                        slots: phrase.map(Slot::Fixed),
                        pool: vec![],
                        fill: vec![],
                    };
                    assert_eq!(domain.covers_plan(&fixed), expected.contains(&phrase));
                    phrase[0] = 3;
                    assert!(!domain.contains(&phrase), "original pins remain required");
                }
                assert!(
                    !domain.contains(&sequence[cutoff]),
                    "next phrase remains untested"
                );
            }
        }
    }

    #[test]
    fn partial_roundtrip_exclusions_and_atomic_progress_promotion_preserve_scope() {
        let mut slots = [Slot::Fixed(0); 12];
        slots[..3].fill(Slot::Hole);
        let plan = Plan::Template {
            slots,
            pool: vec![3, 1, 2],
            fill: vec![],
        };
        let phrases: Vec<_> = plan.stream().collect();
        let partial =
            Record::confirmed(&plan, "english", [0; 20], false, 2, Some(phrases[2])).unwrap();
        let path = temporary_record("partial");
        partial.save(&path).unwrap();
        let loaded = Record::load(&path).unwrap();
        assert_eq!(loaded.digest().unwrap(), partial.digest().unwrap());
        assert_eq!(loaded.plan, plan);
        assert!(loaded.is_compatible(bip39::Language::English, &[0; 20], false));
        assert!(!loaded.is_compatible(bip39::Language::English, &[1; 20], false));
        let rules = crate::history::Exclusions::load(false, std::slice::from_ref(&path)).unwrap();
        for (index, phrase) in phrases.iter().enumerate() {
            assert_eq!(rules.contains(phrase), index < 2);
        }
        assert!(!rules.covers_plan(&plan));
        assert!(!rules.covers_prefix(
            &plan,
            &mut 100,
            std::time::Instant::now() + std::time::Duration::from_secs(1)
        ));
        let fixed = Plan::Template {
            slots: phrases[0].map(Slot::Fixed),
            pool: vec![],
            fill: vec![],
        };
        assert!(rules.covers_plan(&fixed));
        assert!(rules.covers_prefix(
            &fixed,
            &mut 100,
            std::time::Instant::now() + std::time::Duration::from_secs(1)
        ));

        let later =
            Record::confirmed(&plan, "english", [0; 20], false, 4, Some(phrases[4])).unwrap();
        assert!(
            later.save(&path).is_err(),
            "immutable save still never overwrites"
        );
        later.save_replace(&path).unwrap();
        let before = fs::read(&path).unwrap();
        assert!(
            partial.save_replace(&path).is_err(),
            "progress cannot regress"
        );
        assert_eq!(fs::read(&path).unwrap(), before);
        let incompatible =
            Record::confirmed(&plan, "english", [1; 20], false, 4, Some(phrases[4])).unwrap();
        assert!(incompatible.save_replace(&path).is_err());
        assert_eq!(fs::read(&path).unwrap(), before);
        let complete = Record::confirmed(
            &plan,
            "english",
            [0; 20],
            false,
            phrases.len() as u128,
            None,
        )
        .unwrap();
        assert!(complete.is_complete());
        assert_ne!(
            complete.plan, plan,
            "full records keep v1 canonical representation"
        );
        complete.save_replace(&path).unwrap();
        assert!(Record::load(&path).unwrap().is_complete());
        assert!(later.save_replace(&path).is_err());
        assert!(
            crate::history::Exclusions::load(false, std::slice::from_ref(&path))
                .unwrap()
                .covers_plan(&plan)
        );
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn legacy_digest_stays_unchanged_and_mismatched_partial_boundaries_are_rejected() {
        let mut slots = [Slot::Fixed(0); 12];
        slots[..2].fill(Slot::Hole);
        let plan = Plan::Template {
            slots,
            pool: vec![2, 0, 1],
            fill: vec![],
        };
        let phrases: Vec<_> = plan.stream().collect();
        let complete = Record::confirmed(
            &plan,
            "english",
            [0; 20],
            false,
            phrases.len() as u128,
            None,
        )
        .unwrap();
        let legacy = format!(
            "{{\"version\":1,\"result\":\"complete_negative\",\"plan\":{},\"language\":\"english\",\"target\":[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0],\"derivation_path\":\"m/44'/60'/0'/0/0\",\"passphrase\":\"\",\"no_checksum\":false,\"completed_candidates\":6,\"engine_policy\":\"words-breaker-local-v1\"}}",
            serde_json::to_string(&plan.canonical()).unwrap()
        );
        assert_eq!(serde_json::to_string(&complete).unwrap(), legacy);
        assert_eq!(
            complete.digest().unwrap(),
            sha256::Hash::hash(legacy.as_bytes()).to_string()
        );
        let old: Record = serde_json::from_str(&legacy).unwrap();
        assert_eq!(old.digest().unwrap(), complete.digest().unwrap());
        assert!(Record::confirmed(&plan, "english", [0; 20], false, 0, Some(phrases[0])).is_err());
        assert!(Record::confirmed(&plan, "english", [0; 20], false, 2, Some(phrases[3])).is_err());
        assert!(Record::confirmed(&plan, "english", [0; 20], false, 2, None).is_err());

        let mut forged =
            Record::confirmed(&plan, "english", [0; 20], false, 2, Some(phrases[2])).unwrap();
        forged.next_phrase = Some(phrases[3]);
        let path = temporary_record("bad-boundary");
        // A freshly recomputed digest cannot legitimize an inconsistent boundary.
        fs::write(
            &path,
            serde_json::to_vec(&Envelope {
                sha256: forged.digest().unwrap(),
                record: forged,
            })
            .unwrap(),
        )
        .unwrap();
        assert!(Record::load(&path).is_err());
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn subset_proof_matches_exhaustive_sets_across_modes_and_pins() {
        fn random(state: &mut u64, bound: usize) -> usize {
            *state ^= *state << 13;
            *state ^= *state >> 7;
            *state ^= *state << 17;
            (*state as usize) % bound
        }
        fn plan(state: &mut u64, slots: [Slot; 12]) -> Plan {
            let mut pool = |n| (0..n).map(|_| random(state, 3) as u16).collect::<Vec<_>>();
            let a = pool(4);
            let b = pool(4);
            let a = a[..random(state, 5)].to_vec();
            let b = b[..random(state, 5)].to_vec();
            if random(state, 2) == 0 {
                Plan::Template {
                    slots,
                    pool: a,
                    fill: b,
                }
            } else {
                let holes = slots.iter().filter(|s| **s == Slot::Hole).count();
                Plan::Batches {
                    slots,
                    post: a,
                    video: b,
                    post_open: random(state, holes + 1),
                }
            }
        }
        let mut state = 0x528fd61u64;
        let mut proofs = 0;
        for case in 0..8_000 {
            let mut source_slots = [Slot::Fixed(0); 12];
            source_slots[..3].fill(Slot::Hole);
            if case % 3 == 0 {
                source_slots[0] = Slot::Fixed(random(&mut state, 3) as u16);
            }
            let mut query_slots = source_slots;
            for s in &mut query_slots[..3] {
                if *s == Slot::Hole && random(&mut state, 3) == 0 {
                    *s = Slot::Fixed(random(&mut state, 3) as u16);
                }
            }
            if case % 7 == 0 {
                query_slots[0] = Slot::Hole;
            }
            let source = plan(&mut state, source_slots);
            let query = plan(&mut state, query_slots);
            let source_set: HashSet<_> = source.stream().collect();
            let query_set: HashSet<_> = query.stream().collect();
            let want = query_set.is_subset(&source_set);
            let got = Domain::new(&source).covers_plan(&query);
            let compatible = source_slots
                .iter()
                .zip(query_slots)
                .all(|(a, b)| *a == Slot::Hole || *a == b);
            if compatible {
                assert_eq!(got, want, "case {case}: {source:?} vs {query:?}");
            } else {
                assert!(!got || want, "unsafe proof for incompatible pins");
            }
            if got && !query_set.is_empty() {
                proofs += 1;
            }
        }
        assert!(
            proofs > 300,
            "exercise successful nonempty proofs: {proofs}"
        );
    }

    #[test]
    fn subset_proof_rejects_quota_and_repetition_traps() {
        let mut slots = [Slot::Fixed(0); 12];
        slots[..4].fill(Slot::Hole);
        let old = Plan::Batches {
            slots,
            post: vec![1, 2, 3],
            video: vec![4, 5, 6],
            post_open: 2,
        };
        // Every individual word is allowed, but the total contribution is wrong.
        let bad_quota = Plan::Template {
            slots,
            pool: vec![1, 2, 3, 4],
            fill: vec![],
        };
        assert!(!Domain::new(&old).covers_plan(&bad_quota));
        let good = Plan::Template {
            slots,
            pool: vec![1, 2, 4, 5],
            fill: vec![],
        };
        assert!(Domain::new(&old).covers_plan(&good));
        let required = Plan::Template {
            slots,
            pool: vec![1, 2],
            fill: vec![1, 2, 3],
        };
        let lacks_required = Plan::Template {
            slots,
            pool: vec![1, 1, 3, 3],
            fill: vec![],
        };
        assert!(!Domain::new(&required).covers_plan(&lacks_required));
        let mut pinned = slots;
        pinned[0] = Slot::Fixed(1);
        let consumes_twice = Plan::Template {
            slots: pinned,
            pool: vec![1, 2, 4],
            fill: vec![],
        };
        assert!(!Domain::new(&old).covers_plan(&consumes_twice));
        let consumes_once = Plan::Template {
            slots: pinned,
            pool: vec![2, 4, 5],
            fill: vec![],
        };
        assert!(Domain::new(&old).covers_plan(&consumes_once));
        let overlap = Plan::Batches {
            slots,
            post: vec![1, 2, 3],
            video: vec![1, 2, 4],
            post_open: 2,
        };
        let repeats = Plan::Template {
            slots,
            pool: vec![1, 1, 2, 2],
            fill: vec![],
        };
        assert!(Domain::new(&overlap).covers_plan(&repeats));
        let excess = Plan::Template {
            slots,
            pool: vec![1, 1, 1, 2],
            fill: vec![],
        };
        assert!(!Domain::new(&overlap).covers_plan(&excess));
    }

    fn compare(plan: Plan) {
        let expected: HashSet<_> = plan.stream().collect();
        let domain = Domain::new(&plan);
        assert_eq!(plan.count().unwrap(), expected.len() as u128);
        let holes: Vec<_> = plan
            .slots()
            .iter()
            .enumerate()
            .filter_map(|(i, s)| (*s == Slot::Hole).then_some(i))
            .collect();
        for mut code in 0..4usize.pow(holes.len() as u32) {
            let mut phrase = [0; 12];
            for (i, s) in plan.slots().iter().enumerate() {
                if let Slot::Fixed(w) = s {
                    phrase[i] = *w;
                }
            }
            for &pos in &holes {
                phrase[pos] = (code % 4) as u16;
                code /= 4;
            }
            assert_eq!(
                domain.contains(&phrase),
                expected.contains(&phrase),
                "{plan:?}: {phrase:?}"
            );
        }
        let mut wrong_pin = [0; 12];
        wrong_pin[11] = 3;
        assert!(!domain.contains(&wrong_pin));
    }

    #[test]
    fn membership_matches_enumeration_with_repeats_fill_and_source_overlap() {
        let mut slots = [Slot::Fixed(0); 12];
        slots[..3].fill(Slot::Hole);
        let pools: Vec<Vec<u16>> = (0..=4)
            .flat_map(|n| (0..3).combinations_with_replacement(n))
            .collect();
        for pool in &pools {
            for mask in 0..8 {
                let fill = (0..3).filter(|i| mask & (1 << i) != 0).collect();
                compare(Plan::Template {
                    slots,
                    pool: pool.clone(),
                    fill,
                });
            }
            for video in &pools {
                for post_open in 0..=3 {
                    compare(Plan::Batches {
                        slots,
                        post: pool.clone(),
                        video: video.clone(),
                        post_open,
                    });
                }
            }
        }
    }

    #[test]
    fn record_roundtrip_partial_rejection_integrity_and_compatibility() {
        let mut slots = [Slot::Fixed(0); 12];
        slots[..2].fill(Slot::Hole);
        let plan = Plan::Template {
            slots,
            pool: vec![0, 1],
            fill: vec![],
        };
        let mut progress =
            crate::checkpoint::Progress::from_plan(&plan, "english", [0; 20], false, None, false)
                .unwrap();
        assert!(Record::completed(&plan, "english", [0; 20], false, &progress).is_err());
        let mut cursor = progress.candidates.clone();
        let n = cursor.by_ref().count();
        progress.completed(n, &cursor).unwrap();
        let record = Record::completed(&plan, "english", [0; 20], false, &progress).unwrap();
        let path = std::env::temp_dir().join(format!(
            "local-history-{}-{}.json",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        record.save(&path).unwrap();
        assert!(record.save(&path).is_err());
        let loaded = Record::load(&path).unwrap();
        assert!(loaded
            .compatible(bip39::Language::English, &[0; 20], false)
            .is_ok());
        assert!(loaded
            .compatible(bip39::Language::English, &[1; 20], false)
            .is_err());
        assert!(loaded
            .compatible(bip39::Language::English, &[0; 20], true)
            .is_err());
        assert!(loaded
            .compatible(bip39::Language::Spanish, &[0; 20], false)
            .is_err());
        let rules = crate::history::Exclusions::load(false, &[path.clone(), path.clone()]).unwrap();
        assert_eq!(
            rules.fingerprint().unwrap(),
            crate::history::Exclusions::load(false, std::slice::from_ref(&path))
                .unwrap()
                .fingerprint()
                .unwrap()
        );
        let reversed = Plan::Template {
            slots,
            pool: vec![1, 0],
            fill: vec![2, 3],
        };
        assert!(rules.covers_plan(&reversed));
        let mut extra_pin = slots;
        extra_pin[0] = Slot::Fixed(0);
        assert!(rules.covers_plan(&Plan::Template {
            slots: extra_pin,
            pool: vec![1],
            fill: vec![]
        }));
        assert!(!rules.covers_plan(&Plan::Template {
            slots,
            pool: vec![0, 1, 2],
            fill: vec![]
        }));
        let mut envelope: Envelope = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        envelope.record.target = [1; 20];
        fs::write(&path, serde_json::to_vec(&envelope).unwrap()).unwrap();
        assert!(Record::load(&path).is_err());
        envelope.record.version = 99;
        envelope.sha256 = envelope.record.digest().unwrap();
        fs::write(&path, serde_json::to_vec(&envelope).unwrap()).unwrap();
        assert!(Record::load(&path).is_err());
        progress.exclusion = Some(rules);
        assert!(Record::completed(&plan, "english", [0; 20], false, &progress).is_err());
        fs::remove_file(path).unwrap();
    }
}
