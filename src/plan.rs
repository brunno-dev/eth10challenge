//! Search selectors shared by CPU, GPU and checkpoint configuration.
use crate::candidates::{self, Candidates, Slot};
use anyhow::{Context, Result};

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub enum Plan {
    Template {
        slots: [Slot; 12],
        pool: Vec<u16>,
        fill: Vec<u16>,
    },
    Batches {
        slots: [Slot; 12],
        post: Vec<u16>,
        video: Vec<u16>,
        post_open: usize,
    },
}
impl Plan {
    /// Cover this domain by fixing one hole. Children are exhaustive, but may
    /// overlap when a word can come from either batch. They are proof domains,
    /// not a new enumeration order. Never return a truncated list as a cover.
    pub fn split_slot(&self, position: usize, max_children: usize) -> Option<Vec<Self>> {
        if self.validate().is_err() || self.slots().get(position) != Some(&Slot::Hole) {
            return None;
        }
        let holes = self.slots().iter().filter(|s| **s == Slot::Hole).count();
        // Collect only compact branch descriptors before allocating any plans.
        // 0 = template pool/post, 1 = template fill/video.
        let mut branches = Vec::new();
        let mut add = |words: &[u16], origin: u8, excluded: &[u16]| -> Option<()> {
            let mut seen = [false; 2048];
            for &w in excluded {
                seen[w as usize] = true;
            }
            for &w in words {
                if seen[w as usize] {
                    continue;
                }
                seen[w as usize] = true;
                if branches.len() == max_children {
                    return None;
                }
                branches.push((w, origin));
            }
            Some(())
        };
        match self {
            Self::Template { pool, fill, .. } => {
                add(pool, 0, &[])?;
                if pool.len() < holes {
                    add(fill, 1, pool)?;
                }
            }
            Self::Batches {
                post,
                video,
                post_open,
                ..
            } => {
                if *post_open > 0 {
                    add(post, 0, &[])?;
                }
                if holes > *post_open {
                    add(video, 1, &[])?;
                }
            }
        }
        Some(
            branches
                .into_iter()
                .map(|(word, origin)| {
                    let mut child = self.clone();
                    let consume = |pool: &mut Vec<u16>| {
                        let i = pool
                            .iter()
                            .position(|&w| w == word)
                            .expect("branch word belongs to pool");
                        pool.remove(i);
                    };
                    match &mut child {
                        Self::Template { slots, pool, .. } => {
                            slots[position] = Slot::Fixed(word);
                            if origin == 0 {
                                consume(pool);
                            }
                        }
                        Self::Batches {
                            slots,
                            post,
                            video,
                            post_open,
                        } => {
                            slots[position] = Slot::Fixed(word);
                            if origin == 0 {
                                consume(post);
                                *post_open -= 1;
                            } else {
                                consume(video);
                            }
                        }
                    }
                    child
                })
                .collect(),
        )
    }

    pub fn count(&self) -> Result<u128> {
        match self {
            Self::Template { slots, pool, fill } => candidates::count(slots, pool, fill),
            Self::Batches {
                slots,
                post,
                video,
                post_open,
            } => candidates::count_two_pools(slots, *post_open, post, video),
        }
        .context("candidate count exceeds u128")
    }

    /// Canonical domain representation; enumeration order is intentionally absent.
    pub fn canonical(&self) -> Self {
        let mut result = self.clone();
        match &mut result {
            Self::Template { slots, pool, fill } => {
                pool.sort_unstable();
                if pool.len() >= slots.iter().filter(|s| **s == Slot::Hole).count() {
                    fill.clear();
                } else {
                    fill.sort_unstable();
                    fill.dedup();
                }
            }
            Self::Batches { post, video, .. } => {
                post.sort_unstable();
                video.sort_unstable();
            }
        }
        result
    }

    pub fn validate(&self) -> Result<()> {
        anyhow::ensure!(
            self.slots()
                .iter()
                .all(|s| !matches!(s, Slot::Fixed(w) if *w >= 2048)),
            "record has invalid pinned word"
        );
        let valid = |words: &[u16]| words.len() <= 65_536 && words.iter().all(|&w| w < 2048);
        match self {
            Self::Template { pool, fill, .. } => {
                anyhow::ensure!(valid(pool) && valid(fill), "invalid record pool")
            }
            Self::Batches {
                slots,
                post,
                video,
                post_open,
            } => {
                let holes = slots.iter().filter(|s| **s == Slot::Hole).count();
                anyhow::ensure!(
                    valid(post)
                        && valid(video)
                        && *post_open <= holes
                        && *post_open <= 6
                        && holes - post_open <= 6,
                    "invalid record quotas"
                );
            }
        }
        Ok(())
    }
    pub fn slots(&self) -> &[Slot; 12] {
        match self {
            Self::Template { slots, .. } | Self::Batches { slots, .. } => slots,
        }
    }
    pub fn report_pool(&self) -> Vec<u16> {
        match self {
            Self::Template { pool, .. } => pool.clone(),
            Self::Batches { post, video, .. } => post.iter().chain(video).copied().collect(),
        }
    }
    pub fn stream(&self) -> Candidates {
        match self {
            Self::Template { slots, pool, fill } => {
                candidates::stream(*slots, pool.clone(), fill.clone())
            }
            Self::Batches {
                slots,
                post,
                video,
                post_open,
            } => candidates::stream_two_pools(*slots, *post_open, post.clone(), video.clone()),
        }
    }
    pub fn describe(&self, wordlist: &[&str]) -> Result<()> {
        match self {
            Self::Template { slots, pool, fill } => {
                crate::describe_search(slots, pool, fill, wordlist)
            }
            Self::Batches {
                slots,
                post,
                video,
                post_open,
            } => {
                let h = slots.iter().filter(|s| **s == Slot::Hole).count();
                let count = candidates::count_two_pools(slots, *post_open, post, video)
                    .context("candidate count exceeds u128; narrow the search")?;
                println!("Two batches of 6: {} pinned, {h} open slot(s); post fills {post_open} from {}, video fills {} from {}",
                    12-h, post.len(), h-post_open, video.len());
                println!(
                    "Searching {} distinct candidates ({count} exact; streamed)...",
                    crate::format_u128(count)
                );
                Ok(())
            }
        }
    }
}
