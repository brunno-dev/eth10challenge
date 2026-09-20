use super::{Exclusions, MembershipIndex};
use crate::{
    candidates::Slot,
    local_history::{Domain, Record},
    plan::Plan,
};
use std::{collections::HashSet, sync::Arc};

type Phrase = [u16; 12];

fn fixture(specs: Vec<(Plan, usize)>) -> (Exclusions, HashSet<Phrase>) {
    let mut expected = HashSet::new();
    let mut local = Vec::new();
    for (plan, checked) in specs {
        // The oracle is the actual enumerated, confirmed prefix, independently
        // of Domain::contains, allows_word_at, and the index implementation.
        let sequence: Vec<_> = plan.stream().collect();
        assert!(checked > 0 && checked <= sequence.len());
        expected.extend(plan.stream().take(checked));
        let record = Record::confirmed(
            &plan,
            "english",
            [0; 20],
            false,
            checked as u128,
            sequence.get(checked).copied(),
        )
        .unwrap();
        let domain = Domain::from_record(&record);
        local.push((record, domain));
    }
    let index = MembershipIndex::new(&local).map(Arc::new);
    (
        Exclusions {
            ro1: None,
            local: Arc::new(local),
            index,
        },
        expected,
    )
}

fn plan_for(case: usize, tag: u16) -> Plan {
    let mut slots = [Slot::Fixed(0); 12];
    slots[0] = Slot::Fixed(tag);
    for position in [1, if case.is_multiple_of(2) { 4 } else { 5 }, 9] {
        slots[position] = Slot::Hole;
    }
    match case % 8 {
        0 => Plan::Template {
            slots,
            pool: vec![9, 9, 2, 7],
            fill: vec![],
        },
        1 => Plan::Template {
            slots,
            pool: vec![9],
            fill: vec![7, 2, 9],
        },
        2 => Plan::Template {
            slots,
            pool: vec![9, 9],
            fill: vec![7, 2],
        },
        3 => Plan::Template {
            slots,
            pool: vec![],
            fill: vec![9, 2, 7, 9],
        },
        4 => Plan::Batches {
            slots,
            post: vec![9, 2],
            video: vec![7, 7, 9],
            post_open: 1,
        },
        5 => Plan::Batches {
            slots,
            post: vec![9, 2, 2],
            video: vec![7, 9],
            post_open: 2,
        },
        6 => Plan::Batches {
            slots,
            post: vec![42],
            video: vec![9, 9, 7, 2],
            post_open: 0,
        },
        _ => Plan::Template {
            slots,
            pool: vec![7, 2, 9],
            fill: vec![42],
        },
    }
}

fn selected_count(plan: &Plan, index: usize) -> usize {
    let total = plan.stream().count();
    assert!(total > 1);
    if index.is_multiple_of(3) {
        total
    } else {
        1 + index % (total - 1)
    }
}

fn assert_membership(
    indexed: &Exclusions,
    linear: &Exclusions,
    expected: &HashSet<Phrase>,
    phrase: &Phrase,
) {
    let truth = expected.contains(phrase);
    assert_eq!(
        indexed.contains(phrase),
        truth,
        "index mismatch for {phrase:?}"
    );
    assert_eq!(
        linear.contains(phrase),
        truth,
        "linear mismatch for {phrase:?}"
    );
}

#[test]
fn membership_index_matches_enumerated_union_at_block_boundaries() {
    for size in [7, 8, 63, 64, 65, 129] {
        let specs: Vec<_> = (0..size)
            .map(|index| {
                // Separate tags make every record observable, including the last
                // bit of a full block and the first bit of a partial final block.
                let plan = plan_for(index, 100 + index as u16);
                let checked = selected_count(&plan, index);
                (plan, checked)
            })
            .collect();
        let (indexed, expected) = fixture(specs);
        assert_eq!(indexed.index.is_some(), size >= 8);
        if let Some(index) = &indexed.index {
            assert_eq!(index.blocks, size.div_ceil(64));
        }
        let mut linear = indexed.clone();
        linear.index = None;
        let cloned = indexed.clone();
        assert_eq!(
            indexed.fingerprint().unwrap(),
            linear.fingerprint().unwrap()
        );
        assert_eq!(
            indexed.fingerprint().unwrap(),
            cloned.fingerprint().unwrap()
        );

        // Exhaust every assignment of four small positions, including absent
        // words, duplicate excesses, missing required Fill words, invalid source
        // quotas, wrong fixed slots, and suffixes beyond partial boundaries.
        let alphabet = [0, 2, 7, 9, 42];
        for tag in (100..100 + size as u16).chain([99, 100 + size as u16]) {
            for a in alphabet {
                for b in alphabet {
                    for c in alphabet {
                        for d in alphabet {
                            let mut phrase = [0; 12];
                            phrase[0] = tag;
                            phrase[1] = a;
                            phrase[4] = b;
                            phrase[5] = c;
                            phrase[9] = d;
                            assert_membership(&indexed, &linear, &expected, &phrase);
                        }
                    }
                }
            }
        }
        assert!(expected.iter().any(|phrase| phrase[0] == 99 + size as u16));
        for phrase in &expected {
            assert!(cloned.contains(phrase));
        }
        // Every position is checked, including fixed pins and open slots after
        // an earlier prefix comparison may already have found a smaller rank.
        let valid = *expected.iter().next().unwrap();
        for position in 0..12 {
            for invalid in [2048, u16::MAX] {
                let mut phrase = valid;
                phrase[position] = invalid;
                assert_membership(&indexed, &linear, &expected, &phrase);
                assert!(!cloned.contains(&phrase));
            }
        }
    }
}

#[test]
fn membership_index_preserves_overlapping_records_and_exact_partial_frontiers() {
    // With identical positional conditions, the bit intersection cannot choose
    // the answer. Exact source/multiplicity/prefix checks still decide membership.
    let mut specs = Vec::new();
    for index in 0..8 {
        let plan = plan_for(index, 0);
        let checked = selected_count(&plan, index + 1);
        specs.push((plan, checked));
    }
    let (indexed, expected) = fixture(specs);
    let mut linear = indexed.clone();
    linear.index = None;
    assert!(indexed.index.is_some());
    for a in [0, 2, 7, 9, 42] {
        for b in [0, 2, 7, 9, 42] {
            for c in [0, 2, 7, 9, 42] {
                for d in [0, 2, 7, 9, 42] {
                    let mut phrase = [0; 12];
                    phrase[1] = a;
                    phrase[4] = b;
                    phrase[5] = c;
                    phrase[9] = d;
                    assert_membership(&indexed, &linear, &expected, &phrase);
                }
            }
        }
    }

    let mut slots = [Slot::Fixed(0); 12];
    for position in [1, 4, 9] {
        slots[position] = Slot::Hole;
    }
    let plan = Plan::Template {
        slots,
        pool: vec![9, 2, 7, 3],
        fill: vec![],
    };
    let sequence: Vec<_> = plan.stream().collect();
    let (indexed, expected) = fixture((1..=8).map(|checked| (plan.clone(), checked)).collect());
    let mut linear = indexed.clone();
    linear.index = None;
    assert_eq!(expected.len(), 8);
    assert!(
        !indexed.contains(&sequence[8]),
        "largest boundary is exclusive"
    );
    for phrase in sequence {
        assert_membership(&indexed, &linear, &expected, &phrase);
    }
    assert_eq!(
        indexed.fingerprint().unwrap(),
        linear.fingerprint().unwrap()
    );
}
