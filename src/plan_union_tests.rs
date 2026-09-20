//! Independent set oracle for splitting a search into positional subdomains.
use crate::{candidates::Slot, plan::Plan};
use itertools::Itertools;
use std::collections::HashSet;

fn slots(holes: usize) -> [Slot; 12] {
    let mut slots = [Slot::Fixed(100); 12];
    for position in [2, 6, 9].into_iter().take(holes) {
        slots[position] = Slot::Hole;
    }
    slots
}

fn assert_partition(plan: &Plan) {
    let parent: HashSet<_> = plan.stream().collect();
    for (position, slot) in plan.slots().iter().enumerate() {
        if *slot != Slot::Hole {
            continue;
        }
        let children = plan
            .split_slot(position, 4096)
            .unwrap_or_else(|| panic!("unexpected split rejection: {plan:?}, slot {position}"));
        let mut union = HashSet::new();
        for child in &children {
            child.validate().unwrap();
            assert!(matches!(child.slots()[position], Slot::Fixed(_)));
            for other in 0..12 {
                if other != position {
                    assert_eq!(child.slots()[other], plan.slots()[other]);
                }
            }
            union.extend(child.stream());
        }
        assert_eq!(union, parent, "{plan:?}, split at slot {position}");

        // The same complete expansion must fit its exact budget. An incomplete
        // expansion must never be returned as though it covered the parent.
        let exact = plan.split_slot(position, children.len()).unwrap();
        let exact_union: HashSet<_> = exact.iter().flat_map(Plan::stream).collect();
        assert_eq!(exact_union, parent);
        if !children.is_empty() {
            assert!(
                plan.split_slot(position, children.len() - 1).is_none(),
                "a truncated expansion was accepted: {plan:?}, slot {position}"
            );
        }
    }
}

#[test]
fn split_template_union_matches_enumeration_for_every_hole() {
    let pools: Vec<Vec<u16>> = (0..=4)
        .flat_map(|n| (0u16..3).combinations_with_replacement(n))
        .collect();
    for holes in 1..=3 {
        for pool in &pools {
            for fill in &pools {
                assert_partition(&Plan::Template {
                    slots: slots(holes),
                    pool: pool.clone(),
                    fill: fill.clone(),
                });
            }
        }
    }
}

#[test]
fn split_batch_union_matches_enumeration_with_shared_words_and_all_quotas() {
    let pools: Vec<Vec<u16>> = (0..=4)
        .flat_map(|n| (0u16..3).combinations_with_replacement(n))
        .collect();
    for holes in 1..=3 {
        for post in &pools {
            for video in &pools {
                for post_open in 0..=holes {
                    assert_partition(&Plan::Batches {
                        slots: slots(holes),
                        post: post.clone(),
                        video: video.clone(),
                        post_open,
                    });
                }
            }
        }
    }
}

#[test]
fn split_handles_required_fill_transition_and_ambiguous_origins() {
    // Fixing the fill-only word leaves exactly enough pool occurrences for
    // the remaining holes; fixing a shared word must still permit repetition.
    assert_partition(&Plan::Template {
        slots: slots(3),
        pool: vec![1, 2],
        fill: vec![1, 3, 3],
    });
    assert_partition(&Plan::Template {
        slots: slots(3),
        pool: vec![1, 1],
        fill: vec![1, 2],
    });

    // At the selected slot, a shared 1 can come from either source. The
    // completions [1, 2] and [1, 3] require different origin assignments.
    assert_partition(&Plan::Batches {
        slots: slots(2),
        post: vec![1, 2],
        video: vec![1, 3],
        post_open: 1,
    });
    assert_partition(&Plan::Batches {
        slots: slots(3),
        post: vec![1, 1, 2],
        video: vec![1, 2, 2],
        post_open: 2,
    });
}

#[test]
fn split_distinguishes_empty_domains_invalid_requests_and_budget_exhaustion() {
    let empty = Plan::Template {
        slots: slots(1),
        pool: vec![],
        fill: vec![],
    };
    assert!(empty.split_slot(2, 0).unwrap().is_empty());
    let empty_batches = Plan::Batches {
        slots: slots(2),
        post: vec![],
        video: vec![],
        post_open: 1,
    };
    assert!(empty_batches.split_slot(2, 0).unwrap().is_empty());

    let plan = Plan::Template {
        slots: slots(2),
        pool: vec![0, 1, 2],
        fill: vec![],
    };
    assert!(plan.split_slot(0, 4096).is_none());
    assert!(plan.split_slot(12, 4096).is_none());
    assert!(plan.split_slot(usize::MAX, 4096).is_none());
    assert!(plan.split_slot(2, 0).is_none());
    assert!(plan.split_slot(2, 2).is_none());
    assert_eq!(plan.split_slot(2, 3).unwrap().len(), 3);

    let oversized_expansion = Plan::Template {
        slots: slots(1),
        pool: (0..2048).collect(),
        fill: vec![],
    };
    assert!(oversized_expansion.split_slot(2, 256).is_none());

    let mut invalid_slots = slots(2);
    invalid_slots[0] = Slot::Fixed(2048);
    for invalid in [
        Plan::Template {
            slots: invalid_slots,
            pool: vec![0, 1],
            fill: vec![],
        },
        Plan::Template {
            slots: slots(2),
            pool: vec![2048],
            fill: vec![],
        },
        Plan::Template {
            slots: slots(2),
            pool: vec![0],
            fill: vec![2048],
        },
        Plan::Batches {
            slots: slots(2),
            post: vec![0, 1, 2],
            video: vec![],
            post_open: 3,
        },
    ] {
        assert!(invalid.split_slot(2, 4096).is_none());
    }
}
