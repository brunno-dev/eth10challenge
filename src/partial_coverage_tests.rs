use super::{Domain, Plan, Record, Slot};
use std::collections::HashSet;

fn domain_at(plan: &Plan, sequence: &[[u16; 12]], cutoff: usize) -> Domain {
    let record = Record::confirmed(
        plan,
        "english",
        [0; 20],
        false,
        cutoff as u128,
        Some(sequence[cutoff]),
    )
    .unwrap();
    Domain::from_record(&record)
}

/// Include restrictions in every position order, including queries that leave
/// an earlier hole open while fixing a later one. Keep all source assignments.
fn restrictions(plan: &Plan) -> Vec<Plan> {
    let mut queries = vec![plan.clone()];
    let mut seen = HashSet::from([serde_json::to_string(plan).unwrap()]);
    let mut index = 0;
    while index < queries.len() {
        let query = queries[index].clone();
        index += 1;
        for position in 0..12 {
            if query.slots()[position] != Slot::Hole {
                continue;
            }
            for child in query.split_slot(position, 32).unwrap() {
                if seen.insert(serde_json::to_string(&child).unwrap()) {
                    queries.push(child);
                }
            }
        }
    }
    queries
}

#[test]
fn partial_coverage_exhaustive_small_domains_never_exclude_unprocessed_phrases() {
    let mut slots = [Slot::Fixed(0); 12];
    slots[1] = Slot::Hole;
    slots[8] = Slot::Hole;
    // Every list of length 0..=2 over two words: reversed encounter order,
    // duplicate multiplicities, shared sources, and empty source quotas.
    let lists = [
        vec![],
        vec![1],
        vec![2],
        vec![1, 1],
        vec![1, 2],
        vec![2, 1],
        vec![2, 2],
    ];
    let mut plans = Vec::new();
    for first in &lists {
        for second in &lists {
            plans.push(Plan::Template {
                slots,
                pool: first.clone(),
                fill: second.clone(),
            });
            for post_open in 0..=2 {
                plans.push(Plan::Batches {
                    slots,
                    post: first.clone(),
                    video: second.clone(),
                    post_open,
                });
            }
        }
    }
    let mut proofs = 0;
    let mut open_proofs = 0;
    for plan in plans {
        let sequence: Vec<_> = plan.stream().collect();
        let queries: Vec<_> = restrictions(&plan)
            .into_iter()
            .map(|query| {
                let phrases: Vec<_> = query.stream().collect();
                (query, phrases)
            })
            .collect();
        for cutoff in 1..sequence.len() {
            let domain = domain_at(&plan, &sequence, cutoff);
            let covered: HashSet<_> = sequence[..cutoff].iter().copied().collect();
            assert!(!domain.covers_plan(&plan));
            for (query, phrases) in &queries {
                let proven = domain.covers_plan(query);
                let expected = phrases.iter().all(|phrase| covered.contains(phrase));
                assert!(
                    !proven || expected,
                    "false coverage at cutoff {cutoff}: record {plan:?}, query {query:?}"
                );
                if !query.slots().contains(&Slot::Hole) {
                    assert_eq!(proven, expected, "fixed phrase must remain exact");
                }
                if proven && !phrases.is_empty() {
                    proofs += 1;
                    if query.slots().contains(&Slot::Hole) {
                        open_proofs += 1;
                    }
                }
            }
        }
    }
    assert!(proofs > 100, "exercise many successful exact proofs");
    assert!(open_proofs > 20, "exercise the new whole-subtree proofs");
}

#[test]
fn partial_coverage_uses_encounter_order_and_keeps_the_boundary_exclusive() {
    let mut slots = [Slot::Fixed(0); 12];
    for position in [1, 4, 9] {
        slots[position] = Slot::Hole;
    }
    let plan = Plan::Template {
        slots,
        pool: vec![9, 2, 7, 4],
        fill: vec![],
    };
    let sequence: Vec<_> = plan.stream().collect();
    let cutoff = sequence.iter().position(|phrase| phrase[1] == 2).unwrap();
    let domain = domain_at(&plan, &sequence, cutoff);
    let parts = plan.split_slot(1, 4).unwrap();
    let earlier = parts
        .iter()
        .find(|query| query.slots()[1] == Slot::Fixed(9))
        .unwrap();
    assert!(
        domain.covers_plan(earlier),
        "9 occurs before 2 in this record"
    );
    for query in parts
        .iter()
        .filter(|query| query.slots()[1] != Slot::Fixed(9))
    {
        assert!(!domain.covers_plan(query));
    }
    let fixed = |phrase: [u16; 12]| Plan::Template {
        slots: phrase.map(Slot::Fixed),
        pool: vec![],
        fill: vec![],
    };
    assert!(domain.covers_plan(&fixed(sequence[cutoff - 1])));
    assert!(!domain.covers_plan(&fixed(sequence[cutoff])));
    assert!(!domain.covers_plan(&fixed(sequence[cutoff + 1])));

    // Equal first word, then a smaller fixed second word, still proves an open
    // suffix. A later fixed word cannot prove anything with the first hole open.
    let cutoff = sequence
        .iter()
        .position(|phrase| phrase[1] == 9 && phrase[4] == 7)
        .unwrap();
    let domain = domain_at(&plan, &sequence, cutoff);
    let descendants = earlier.split_slot(4, 4).unwrap();
    let earlier_second = descendants
        .iter()
        .find(|query| query.slots()[4] == Slot::Fixed(2))
        .unwrap();
    assert!(domain.covers_plan(earlier_second));
    for query in plan.split_slot(4, 4).unwrap() {
        assert!(!domain.covers_plan(&query));
    }
}

#[test]
fn partial_coverage_order_proof_alone_cannot_bypass_source_or_pin_constraints() {
    let mut slots = [Slot::Fixed(0); 12];
    for position in [1, 4, 9] {
        slots[position] = Slot::Hole;
    }
    let plan = Plan::Batches {
        slots,
        post: vec![9, 2],
        video: vec![9, 2, 7],
        post_open: 1,
    };
    let sequence: Vec<_> = plan.stream().collect();
    let cutoff = sequence.iter().position(|phrase| phrase[1] == 2).unwrap();
    let domain = domain_at(&plan, &sequence, cutoff);
    let parts: Vec<_> = plan
        .split_slot(1, 8)
        .unwrap()
        .into_iter()
        .filter(|query| query.slots()[1] == Slot::Fixed(9))
        .collect();
    assert_eq!(parts.len(), 2, "shared word keeps both source assignments");
    assert!(parts.iter().all(|query| domain.covers_plan(query)));

    // All queries begin with the earlier word, but these variants have a phrase
    // outside the record's structural domain and must not become exclusions.
    let mut restricted = slots;
    restricted[1] = Slot::Fixed(9);
    for query in [
        Plan::Template {
            slots: restricted,
            pool: vec![7, 7],
            fill: vec![],
        },
        Plan::Template {
            slots: restricted,
            pool: vec![42, 2],
            fill: vec![],
        },
        Plan::Batches {
            slots: restricted,
            post: vec![7],
            video: vec![7],
            post_open: 1,
        },
    ] {
        assert!(!domain.covers_plan(&query));
    }
    restricted[0] = Slot::Fixed(1);
    let wrong_pin = Plan::Template {
        slots: restricted,
        pool: vec![2, 7],
        fill: vec![],
    };
    assert!(!domain.covers_plan(&wrong_pin));
}
