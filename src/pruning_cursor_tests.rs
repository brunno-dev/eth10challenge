//! Independent traversal checks: skipped ranges are compared with ordinary
//! enumeration, without consulting the pruning counter or coverage proof code.
use crate::candidates::{Cursor, Slot, Step};
use crate::plan::Plan;
use crate::pruning::Pruning;

fn slots(n: usize) -> [Slot; 12] {
    let mut slots = [Slot::Fixed(99); 12];
    // Noncontiguous holes catch accidental use of phrase positions as depths.
    for &position in [1, 3, 7, 10].iter().take(n) {
        slots[position] = Slot::Hole;
    }
    slots
}

fn walk(plan: &Plan, prefixes: &[Vec<u16>], start: usize, limits: &[usize]) -> bool {
    assert!(!limits.is_empty() && limits.iter().all(|&n| n > 0));
    let holes: Vec<_> = plan
        .slots()
        .iter()
        .enumerate()
        .filter_map(|(position, slot)| (*slot == Slot::Hole).then_some(position))
        .collect();
    let pruning = Pruning::from_prefixes(holes.clone(), prefixes.to_vec());
    let expected: Vec<_> = plan.stream().collect();
    let covered = |phrase: &[u16; 12]| {
        prefixes.iter().any(|prefix| {
            prefix.len() <= holes.len()
                && prefix
                    .iter()
                    .zip(&holes)
                    .all(|(&word, &position)| phrase[position] == word)
        })
    };
    let mut iterator = plan.stream();
    for _ in 0..start {
        assert!(iterator.next().is_some());
    }
    let mut raw = start;
    let mut turn = 0;
    let mut jumped = false;
    loop {
        let before = serde_json::to_vec(&iterator.cursor()).unwrap();
        assert!(iterator.next_pruned(0, &pruning).is_none());
        assert_eq!(serde_json::to_vec(&iterator.cursor()).unwrap(), before);
        let limit = limits[turn % limits.len()];
        match iterator.next_pruned(limit, &pruning) {
            Some(Step::Candidate(phrase)) => {
                assert_eq!(phrase, expected[raw], "candidate at raw offset {raw}");
                raw += 1;
            }
            Some(Step::Skipped(n)) => {
                assert!(n > 0 && n <= limit, "invalid skipped count {n}/{limit}");
                assert!(raw + n <= expected.len(), "skip exceeds remaining domain");
                assert!(
                    expected[raw..raw + n].iter().all(covered),
                    "skip includes an uncovered candidate at raw offset {raw}"
                );
                raw += n;
                jumped |= n > 1;
            }
            None => {
                assert_eq!(raw, expected.len(), "iterator finished before domain end");
                let encoded = serde_json::to_vec(&iterator.cursor()).unwrap();
                let cursor: Cursor = serde_json::from_slice(&encoded).unwrap();
                let mut restored = plan.stream();
                restored.restore(&cursor).unwrap();
                assert!(restored.next().is_none());
                assert!(iterator.next_pruned(limit, &pruning).is_none());
                break;
            }
        }
        // An old reader using only Iterator::next must resume at exactly the
        // same raw offset. Then keep testing from the deserialized iterator.
        let encoded = serde_json::to_vec(&iterator.cursor()).unwrap();
        let cursor: Cursor = serde_json::from_slice(&encoded).unwrap();
        let mut restored = plan.stream();
        restored.restore(&cursor).unwrap();
        assert_eq!(restored.clone().collect::<Vec<_>>(), expected[raw..]);
        iterator = restored;
        turn += 1;
        assert!(
            turn <= expected.len() + 1,
            "pruning failed to make progress"
        );
    }
    jumped
}

#[test]
fn pruning_template_fill_and_old_cursor_boundaries() {
    let mut jumped = false;
    for (pool, fill) in [
        (vec![1, 2, 3, 4, 5], vec![]),
        (vec![1, 1, 2, 3], vec![]),
        (vec![1, 2], vec![1, 2, 3]),
        (vec![1], vec![1, 2, 3]),
        (vec![], vec![1, 1, 2]),
        (vec![1, 2], vec![]),
    ] {
        let plan = Plan::Template {
            slots: slots(4),
            pool,
            fill,
        };
        let total = plan.stream().count();
        for start in 0..=total {
            jumped |= walk(
                &plan,
                &[vec![1], vec![2, 3]],
                start,
                &[1, 2, 3, 5, usize::MAX],
            );
        }
    }
    assert!(jumped, "the test must exercise a whole subtree skip");
}

#[test]
fn pruning_batch_overlap_quotas_and_old_cursor_boundaries() {
    let mut jumped = false;
    for (post, video) in [
        (vec![1, 2, 2, 4], vec![1, 3, 3, 5]),
        (vec![1, 1, 2, 3], vec![1, 2, 2, 3]),
    ] {
        for post_open in 0..=4 {
            let plan = Plan::Batches {
                slots: slots(4),
                post: post.clone(),
                video: video.clone(),
                post_open,
            };
            let total = plan.stream().count();
            for start in 0..=total {
                jumped |= walk(
                    &plan,
                    &[vec![1], vec![2, 3]],
                    start,
                    &[usize::MAX, 1, 2, 3, 5],
                );
            }
        }
    }
    assert!(jumped, "the test must exercise a whole subtree skip");

    let plan = Plan::Batches {
        slots: slots(4),
        post: vec![1, 2, 2],
        video: vec![1, 3, 3],
        post_open: 2,
    };
    let tails: Vec<_> = plan
        .stream()
        .filter(|phrase| phrase[1] == 1 && phrase[3] == 1)
        .map(|phrase| [phrase[7], phrase[10]])
        .collect();
    assert_eq!(tails, vec![[2, 3], [3, 2]]);
    assert!(walk(&plan, &[vec![1, 1]], 0, &[usize::MAX]));
}

#[test]
fn pruning_count_limits_root_and_last_subtree() {
    let plan = Plan::Template {
        slots: slots(4),
        pool: vec![1, 2, 3, 4],
        fill: vec![],
    };
    // Each first-word subtree contains 3! = 6 phrases. Test either side of
    // that boundary, both fresh and resumed after ordinary enumeration.
    for limit in [1, 5, 6, 7, 23, 24, 25, usize::MAX] {
        for start in 0..=24 {
            walk(&plan, &[vec![1], vec![4]], start, &[limit]);
            walk(&plan, &[vec![]], start, &[limit]);
        }
    }
    assert!(walk(&plan, &[vec![]], 0, &[24]));
    assert!(walk(&plan, &[vec![4]], 0, &[6]));
}

#[test]
fn pruning_fixed_domain_empty_domain_and_no_coverage() {
    for plan in [
        Plan::Template {
            slots: slots(0),
            pool: vec![],
            fill: vec![],
        },
        Plan::Template {
            slots: slots(3),
            pool: vec![],
            fill: vec![],
        },
        Plan::Template {
            slots: slots(3),
            pool: vec![1, 2, 3],
            fill: vec![],
        },
    ] {
        let mut policies = vec![vec![], vec![vec![]]];
        if plan.slots().contains(&Slot::Hole) {
            policies.push(vec![vec![777]]);
        }
        for prefixes in policies {
            for start in 0..=plan.stream().count() {
                walk(&plan, &prefixes, start, &[1, usize::MAX]);
            }
        }
    }
}
