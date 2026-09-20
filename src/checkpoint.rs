//! Checkpoints contain the next cursor after completed work, never producer lookahead.
#[cfg(test)]
use crate::candidates::Slot;
use crate::candidates::{Candidates, Cursor};
use crate::plan::Plan;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Config {
    slots: Vec<Option<u16>>,
    pool: Vec<u16>,
    fill: Vec<u16>,
    language: String,
    target: [u8; 20],
    // Defaults preserve version-1 checkpoints made before these selectors.
    #[serde(default)]
    batches: Option<BatchConfig>,
    #[serde(default)]
    no_checksum: bool,
    #[serde(default)]
    exclusion: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct BatchConfig {
    video: Vec<u16>,
    post_open: usize,
}
impl Config {
    fn for_plan(
        plan: &Plan,
        language: &str,
        target: [u8; 20],
        no_checksum: bool,
        exclusion: Option<&crate::history::Exclusions>,
    ) -> Result<Self> {
        if let Some(rule) = exclusion {
            rule.compatible(crate::parse_language(language)?, &target, no_checksum)?;
        }
        let (pool, fill, batches) = match plan {
            Plan::Template { pool, fill, .. } => (pool.clone(), fill.clone(), None),
            Plan::Batches {
                post,
                video,
                post_open,
                ..
            } => (
                post.clone(),
                vec![],
                Some(BatchConfig {
                    video: video.clone(),
                    post_open: *post_open,
                }),
            ),
        };
        Ok(Self {
            slots: plan
                .slots()
                .iter()
                .map(|s| match s {
                    crate::candidates::Slot::Fixed(w) => Some(*w),
                    crate::candidates::Slot::Hole => None,
                })
                .collect(),
            pool,
            fill,
            language: language.to_lowercase(),
            target,
            batches,
            no_checksum,
            exclusion: exclusion.map(|r| r.fingerprint()).transpose()?,
        })
    }
}
#[derive(Serialize, Deserialize)]
struct Saved {
    version: u32,
    config: Config,
    checked: u128,
    #[serde(default)]
    excluded: u128,
    cursor: Cursor,
}
pub struct Progress {
    config: Config,
    pub candidates: Candidates,
    pub checked: u128,
    pub excluded: u128,
    pub exclusion: Option<crate::history::Exclusions>,
    pub pruning: Option<std::sync::Arc<crate::pruning::Pruning>>,
    pub pruned_this_run: u128,
    pub metrics: crate::metrics::SearchMetrics,
    path: Option<PathBuf>,
    last_save: Instant,
}
impl Progress {
    #[cfg(test)]
    pub fn new(
        slots: [Slot; 12],
        pool: &[u16],
        fill: &[u16],
        language: &str,
        target: [u8; 20],
        path: Option<PathBuf>,
        resume: bool,
    ) -> Result<Self> {
        Self::from_plan(
            &Plan::Template {
                slots,
                pool: pool.to_vec(),
                fill: fill.to_vec(),
            },
            language,
            target,
            false,
            path,
            resume,
        )
    }
    #[cfg(test)]
    pub fn from_plan(
        plan: &Plan,
        language: &str,
        target: [u8; 20],
        no_checksum: bool,
        path: Option<PathBuf>,
        resume: bool,
    ) -> Result<Self> {
        Self::with_history(plan, language, target, no_checksum, path, resume, None)
    }
    #[allow(clippy::too_many_arguments)]
    pub fn with_history(
        plan: &Plan,
        language: &str,
        target: [u8; 20],
        no_checksum: bool,
        path: Option<PathBuf>,
        resume: bool,
        exclusion: Option<crate::history::Exclusions>,
    ) -> Result<Self> {
        let config = Config::for_plan(plan, language, target, no_checksum, exclusion.as_ref())?;
        let mut candidates = plan.stream();
        let mut checked = 0;
        let mut excluded = 0;
        if resume {
            let bytes = fs::read(path.as_ref().context("--resume requires --checkpoint")?)
                .context("reading checkpoint")?;
            let saved: Saved = serde_json::from_slice(&bytes).context("invalid checkpoint")?;
            anyhow::ensure!(
                matches!(saved.version, 1..=3) && saved.config == config,
                "checkpoint version or search inputs differ"
            );
            candidates
                .restore(&saved.cursor)
                .map_err(anyhow::Error::msg)?;
            checked = saved.checked;
            anyhow::ensure!(saved.excluded <= saved.checked, "invalid excluded count");
            excluded = saved.excluded;
        } else if let Some(p) = &path {
            anyhow::ensure!(
                !p.exists(),
                "checkpoint already exists; use --resume or a new path"
            );
        }
        let progress = Self {
            config,
            candidates,
            checked,
            excluded,
            exclusion,
            pruning: None,
            pruned_this_run: 0,
            metrics: Default::default(),
            path,
            last_save: Instant::now(),
        };
        if !resume {
            progress.save()?;
        }
        Ok(progress)
    }
    #[cfg(test)]
    pub fn completed(&mut self, n: usize, cursor: &Candidates) -> Result<()> {
        self.completed_filtered(n, 0, cursor)
    }
    #[cfg(test)]
    pub fn completed_filtered(
        &mut self,
        n: usize,
        excluded: usize,
        cursor: &Candidates,
    ) -> Result<()> {
        self.completed_pruned(n, excluded, 0, cursor)
    }
    pub fn completed_pruned(
        &mut self,
        n: usize,
        excluded: usize,
        pruned: usize,
        cursor: &Candidates,
    ) -> Result<()> {
        anyhow::ensure!(pruned <= excluded, "pruned count exceeds exclusions");
        anyhow::ensure!(excluded <= n, "excluded count exceeds batch size");
        self.pruned_this_run += pruned as u128;
        self.excluded = self
            .excluded
            .checked_add(excluded as u128)
            .context("exclusion count overflow")?;
        self.checked = self
            .checked
            .checked_add(n as u128)
            .context("progress overflow")?;
        self.candidates = cursor.clone();
        if self.last_save.elapsed() >= Duration::from_secs(10) {
            self.save()?;
            self.last_save = Instant::now();
        }
        Ok(())
    }
    pub fn save(&self) -> Result<()> {
        if let Some(path) = &self.path {
            let saved = Saved {
                // Older binaries ignore unknown JSON fields. Use version 2 for
                // new modes so they cannot silently resume with old semantics.
                version: if self.config.exclusion.is_some() {
                    3
                } else if self.config.no_checksum || self.config.batches.is_some() {
                    2
                } else {
                    1
                },
                config: self.config.clone(),
                checked: self.checked,
                excluded: self.excluded,
                cursor: self.candidates.cursor(),
            };
            atomic_write(path, &serde_json::to_vec(&saved)?)?;
        }
        Ok(())
    }
}
/// Export only after the caller has established that this run did not find a
/// match. A checkpoint alone cannot distinguish a negative batch from a batch
/// that returned a match; it only certifies the completed traversal frontier.
pub fn export_record(
    plan: &Plan,
    language: &str,
    target: [u8; 20],
    no_checksum: bool,
    path: &Path,
    exclusion: Option<&crate::history::Exclusions>,
) -> Result<crate::local_history::Record> {
    plan.validate()?;
    let config = Config::for_plan(plan, language, target, no_checksum, exclusion)?;
    // Read through a limit as well as checking metadata: a file being replaced
    // or enlarged concurrently must not bypass the allocation bound.
    const LIMIT: u64 = 4 * 1024 * 1024;
    anyhow::ensure!(fs::metadata(path)?.len() <= LIMIT, "checkpoint too large");
    let mut bytes = Vec::new();
    std::io::Read::read_to_end(
        &mut std::io::Read::take(fs::File::open(path)?, LIMIT + 1),
        &mut bytes,
    )?;
    anyhow::ensure!(bytes.len() as u64 <= LIMIT, "checkpoint too large");
    let saved: Saved = serde_json::from_slice(&bytes).context("invalid checkpoint")?;
    anyhow::ensure!(
        matches!(saved.version, 1..=3) && saved.config == config,
        "checkpoint version or search inputs differ"
    );
    let count = plan.count()?;
    anyhow::ensure!(saved.checked > 0, "checkpoint has no completed candidates");
    anyhow::ensure!(
        saved.checked <= count,
        "checkpoint count exceeds search domain"
    );
    anyhow::ensure!(saved.excluded <= saved.checked, "invalid excluded count");
    // Additional proven negatives may have been loaded after validating the
    // original fingerprint on resume. They can increase excluded even when
    // config.exclusion is None. As above, the caller must certify the negative
    // outcome; a checkpoint alone never establishes that outcome.
    let mut candidates = plan.stream();
    candidates
        .restore(&saved.cursor)
        .map_err(anyhow::Error::msg)?;
    let remaining = candidates
        .remaining_count()
        .context("checkpoint remaining count overflow")?;
    anyhow::ensure!(
        count.checked_sub(remaining) == Some(saved.checked),
        "checkpoint count does not match its traversal cursor"
    );
    let next = candidates.next();
    anyhow::ensure!(
        next.is_none() == (saved.checked == count),
        "checkpoint completion differs from its traversal cursor"
    );
    crate::local_history::Record::confirmed(
        plan,
        language,
        target,
        no_checksum,
        saved.checked,
        next,
    )
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_nanos();
    let temp = path.with_extension(format!("{}.{}.tmp", std::process::id(), stamp));
    let result = (|| -> Result<()> {
        let mut f = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        drop(f);
        fs::rename(&temp, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result.with_context(|| format!("saving checkpoint {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn export_fixture() -> (PathBuf, Plan, Saved) {
        let path = std::env::temp_dir().join(format!(
            "checkpoint-export-{}-{}.json",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let mut slots = [Slot::Fixed(0); 12];
        slots[..3].fill(Slot::Hole);
        let plan = Plan::Template {
            slots,
            pool: vec![2, 1, 3],
            fill: vec![],
        };
        let mut it = plan.stream();
        it.next();
        it.next();
        let saved = Saved {
            version: 1,
            config: Config::for_plan(&plan, "english", [0; 20], false, None).unwrap(),
            checked: 2,
            excluded: 0,
            cursor: it.cursor(),
        };
        (path, plan, saved)
    }

    #[test]
    fn export_validates_selectors_and_exact_cursor_rank() {
        let (path, plan, mut saved) = export_fixture();
        let write = |saved: &Saved| fs::write(&path, serde_json::to_vec(saved).unwrap()).unwrap();
        write(&saved);
        assert!(export_record(&plan, "english", [0; 20], false, &path, None).is_ok());
        assert!(export_record(&plan, "english", [1; 20], false, &path, None).is_err());
        assert!(export_record(&plan, "english", [0; 20], true, &path, None).is_err());
        let mut changed = plan.clone();
        if let Plan::Template { pool, .. } = &mut changed {
            pool.reverse();
        }
        assert!(export_record(&changed, "english", [0; 20], false, &path, None).is_err());
        for checked in [0, 1, 3, 6, 7] {
            saved.checked = checked;
            write(&saved);
            assert!(
                export_record(&plan, "english", [0; 20], false, &path, None).is_err(),
                "checked={checked}"
            );
        }
        saved.checked = 2;
        saved.excluded = 1;
        write(&saved);
        // Additional runtime negatives do not change the original fingerprint.
        assert!(export_record(&plan, "english", [0; 20], false, &path, None).is_ok());
        saved.excluded = 3;
        write(&saved);
        assert!(export_record(&plan, "english", [0; 20], false, &path, None).is_err());
        saved.excluded = 0;
        let mut json = serde_json::to_value(&saved).unwrap();
        json["cursor"]["depth"] = 12.into();
        fs::write(&path, serde_json::to_vec(&json).unwrap()).unwrap();
        assert!(export_record(&plan, "english", [0; 20], false, &path, None).is_err());
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn export_accepts_only_nonempty_consistent_complete_frontiers() {
        let (path, plan, mut saved) = export_fixture();
        let write = |saved: &Saved| fs::write(&path, serde_json::to_vec(saved).unwrap()).unwrap();
        let mut it = plan.stream();
        saved.checked = 0;
        saved.cursor = it.cursor();
        write(&saved);
        assert!(export_record(&plan, "english", [0; 20], false, &path, None).is_err());
        saved.checked = 6;
        for _ in 0..6 {
            it.next();
        }
        // Valid engines may save immediately after the final leaf, before next()
        // has marked the iterator done. Both frontier representations are valid.
        saved.cursor = it.cursor();
        write(&saved);
        assert!(export_record(&plan, "english", [0; 20], false, &path, None).is_ok());
        assert!(it.next().is_none());
        saved.cursor = it.cursor();
        write(&saved);
        assert!(export_record(&plan, "english", [0; 20], false, &path, None).is_ok());
        saved.checked = 5;
        write(&saved);
        assert!(export_record(&plan, "english", [0; 20], false, &path, None).is_err());
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn export_requires_original_exclusion_fingerprint() {
        let (path, plan, mut saved) = export_fixture();
        let target =
            crate::eth::parse_address("0x9c2f44efad0c1e852a09df9939e6daf061140caf").unwrap();
        let exclusion = crate::history::Exclusions::ro1().unwrap();
        saved.version = 3;
        saved.config = Config::for_plan(&plan, "english", target, false, Some(&exclusion)).unwrap();
        saved.excluded = 1;
        fs::write(&path, serde_json::to_vec(&saved).unwrap()).unwrap();
        assert!(export_record(&plan, "english", target, false, &path, Some(&exclusion)).is_ok());
        assert!(export_record(&plan, "english", target, false, &path, None).is_err());
        saved.config.exclusion = Some("modified-evidence".into());
        fs::write(&path, serde_json::to_vec(&saved).unwrap()).unwrap();
        assert!(export_record(&plan, "english", target, false, &path, Some(&exclusion)).is_err());
        fs::remove_file(path).unwrap();
    }
    #[test]
    fn exclusion_checkpoint_binds_policy_evidence_and_counts() {
        let path = std::env::temp_dir().join(format!(
            "ro1-checkpoint-{}-{}.json",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let plan = Plan::Template {
            slots: [Slot::Fixed(0); 12],
            pool: vec![],
            fill: vec![],
        };
        let target =
            crate::eth::parse_address("0x9c2f44efad0c1e852a09df9939e6daf061140caf").unwrap();
        let make = |resume, enabled| {
            Progress::with_history(
                &plan,
                "english",
                target,
                false,
                Some(path.clone()),
                resume,
                if enabled {
                    Some(crate::history::Exclusions::ro1().unwrap())
                } else {
                    None
                },
            )
        };
        let mut p = make(false, true).unwrap();
        let mut cursor = p.candidates.clone();
        cursor.next();
        p.completed_filtered(1, 1, &cursor).unwrap();
        p.save().unwrap();
        let restored = make(true, true).unwrap();
        assert_eq!((restored.checked, restored.excluded), (1, 1));
        assert!(restored.candidates.clone().next().is_none());
        assert!(make(true, false).is_err());
        let bytes = fs::read(&path).unwrap();
        let mut saved: Saved = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(saved.version, 3);
        saved.config.exclusion = Some("RO1:changed-evidence".into());
        fs::write(&path, serde_json::to_vec(&saved).unwrap()).unwrap();
        assert!(make(true, true).is_err());
        saved.config.exclusion = None;
        saved.excluded = 0;
        saved.version = 1;
        fs::write(&path, serde_json::to_vec(&saved).unwrap()).unwrap();
        assert!(make(true, true).is_err());
        assert!(make(true, false).is_ok());
        fs::remove_file(path).unwrap();
    }
    #[test]
    fn save_resume_and_input_mismatch() {
        let path = std::env::temp_dir().join(format!(
            "words-breaker-{}-{}.json",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let mut slots = [Slot::Fixed(0); 12];
        slots[..3].fill(Slot::Hole);
        let make = |resume, target| {
            Progress::new(
                slots,
                &[1, 2],
                &[2, 3],
                "english",
                target,
                Some(path.clone()),
                resume,
            )
        };
        let mut p = make(false, [0; 20]).unwrap();
        let mut it = p.candidates.clone();
        let expected: Vec<_> = it.clone().collect();
        it.next();
        it.next();
        p.completed(2, &it).unwrap();
        p.save().unwrap();
        let restored = make(true, [0; 20]).unwrap();
        assert_eq!(restored.checked, 2);
        assert_eq!(restored.candidates.collect::<Vec<_>>(), expected[2..]);
        // Real version-1 layout: the newly introduced fields are absent.
        let mut legacy: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        legacy["config"].as_object_mut().unwrap().remove("batches");
        legacy["config"]
            .as_object_mut()
            .unwrap()
            .remove("no_checksum");
        fs::write(&path, serde_json::to_vec(&legacy).unwrap()).unwrap();
        assert_eq!(
            make(true, [0; 20]).unwrap().candidates.collect::<Vec<_>>(),
            expected[2..]
        );
        assert!(make(true, [1; 20]).is_err());
        assert!(make(false, [0; 20]).is_err());
        fs::remove_file(path).unwrap();
    }
    #[test]
    fn batch_checkpoint_preserves_mode_and_checksum_policy() {
        let path = std::env::temp_dir().join(format!(
            "batch-checkpoint-{}-{}.json",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let mut slots = [Slot::Fixed(0); 12];
        slots[..4].fill(Slot::Hole);
        let plan = Plan::Batches {
            slots,
            post: vec![1, 1, 2],
            video: vec![1, 2, 3],
            post_open: 2,
        };
        let expected: Vec<_> = plan.stream().collect();
        let mut progress =
            Progress::from_plan(&plan, "english", [0; 20], true, Some(path.clone()), false)
                .unwrap();
        let mut cursor = progress.candidates.clone();
        for _ in 0..7 {
            cursor.next();
        }
        progress.completed(7, &cursor).unwrap();
        progress.save().unwrap();
        let saved: Saved = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(saved.version, 2);
        let restored =
            Progress::from_plan(&plan, "english", [0; 20], true, Some(path.clone()), true).unwrap();
        assert_eq!(restored.checked, 7);
        assert_eq!(restored.candidates.collect::<Vec<_>>(), expected[7..]);
        assert!(
            Progress::from_plan(&plan, "english", [0; 20], false, Some(path.clone()), true)
                .is_err()
        );
        let swapped = Plan::Batches {
            slots,
            post: vec![1, 2, 3],
            video: vec![1, 1, 2],
            post_open: 2,
        };
        assert!(
            Progress::from_plan(&swapped, "english", [0; 20], true, Some(path.clone()), true)
                .is_err()
        );
        fs::remove_file(path).unwrap();
    }
}
