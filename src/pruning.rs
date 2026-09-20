//! Conservative prefix proofs, shared by both enumeration backends.
use crate::{candidates::Slot, history::Exclusions, plan::Plan};
use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};

const MAX_DEPTH: usize = 3;
const MAX_PLANS: usize = 256;
const MAX_CHECKS: usize = 4096;

#[derive(Clone, Default)]
struct Node {
    terminal: bool,
    children: BTreeMap<u16, Node>,
}

impl Node {
    fn is_empty(&self) -> bool {
        !self.terminal && self.children.is_empty()
    }
}

#[derive(Clone, Default)]
pub struct Pruning {
    holes: Vec<usize>,
    root: Node,
}
impl crate::candidates::PrefixCoverage for Pruning {
    fn covers(&self, phrase: &[u16; 12], depth: usize) -> bool {
        self.covers(phrase, depth)
    }
}

struct Budget {
    plans: usize,
    checks: usize,
    deadline: Instant,
}

impl Budget {
    fn exhausted(&self) -> bool {
        self.checks == 0 || Instant::now() >= self.deadline
    }
}

impl Pruning {
    pub fn compile(plan: &Plan, rule: &Exclusions) -> Self {
        if !rule.has_local_records() {
            return Self::default();
        }
        Self::compile_with(plan, |plan, checks, deadline| {
            rule.covers_prefix(plan, checks, deadline)
        })
    }

    fn compile_with(
        plan: &Plan,
        mut proof: impl FnMut(&Plan, &mut usize, Instant) -> bool,
    ) -> Self {
        if plan.validate().is_err() {
            return Self::default();
        }
        let holes = plan
            .slots()
            .iter()
            .enumerate()
            .filter_map(|(i, slot)| (*slot == Slot::Hole).then_some(i))
            .collect::<Vec<_>>();
        let mut budget = Budget {
            plans: MAX_PLANS - 1, // The original root clone counts as a plan.
            checks: MAX_CHECKS,
            deadline: Instant::now() + Duration::from_millis(250),
        };
        let root = compile_node(vec![plan.clone()], 0, &holes, &mut proof, &mut budget);
        Self { holes, root }
    }

    pub fn is_empty(&self) -> bool {
        self.root.is_empty()
    }

    /// `depth` counts assigned original holes; unassigned phrase cells are ignored.
    pub fn covers(&self, phrase: &[u16; 12], depth: usize) -> bool {
        let mut node = &self.root;
        if node.terminal {
            return true;
        }
        for &position in self.holes.iter().take(depth) {
            let Some(child) = node.children.get(&phrase[position]) else {
                return false;
            };
            node = child;
            if node.terminal {
                return true;
            }
        }
        false
    }

    #[cfg(test)]
    pub fn from_prefixes(holes: Vec<usize>, prefixes: Vec<Vec<u16>>) -> Self {
        assert!(holes.iter().all(|&position| position < 12));
        let mut result = Self {
            holes,
            root: Node::default(),
        };
        for prefix in prefixes {
            assert!(prefix.len() <= result.holes.len());
            let mut node = &mut result.root;
            for word in prefix {
                node = node.children.entry(word).or_default();
            }
            node.terminal = true;
            node.children.clear();
        }
        result
    }
}

fn compile_node(
    family: Vec<Plan>,
    depth: usize,
    holes: &[usize],
    proof: &mut impl FnMut(&Plan, &mut usize, Instant) -> bool,
    budget: &mut Budget,
) -> Node {
    if family.is_empty() || budget.exhausted() {
        return Node::default();
    }
    // One word can have several source assignments. Its prefix is covered only
    // if every corresponding remainder is covered, possibly by different records.
    if family
        .iter()
        .all(|plan| proof(plan, &mut budget.checks, budget.deadline))
    {
        return Node {
            terminal: true,
            children: BTreeMap::new(),
        };
    }
    if depth >= MAX_DEPTH || depth >= holes.len() || budget.exhausted() {
        return Node::default();
    }
    let position = holes[depth];
    let mut grouped = BTreeMap::<u16, Vec<Plan>>::new();
    for plan in family {
        if budget.exhausted() {
            return Node::default();
        }
        let Some(children) = plan.split_slot(position, budget.plans) else {
            // All source variants must be grouped before proving any child.
            // A truncated family must never become a covered prefix.
            return Node::default();
        };
        budget.plans -= children.len();
        for child in children {
            let Slot::Fixed(word) = child.slots()[position] else {
                unreachable!("split_slot fixes the requested hole");
            };
            grouped.entry(word).or_default().push(child);
        }
    }
    let mut result = Node::default();
    for (word, children) in grouped {
        if budget.exhausted() {
            break;
        }
        let child = compile_node(children, depth + 1, holes, proof, budget);
        if !child.is_empty() {
            result.children.insert(word, child);
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::local_history::Domain;
    use std::collections::HashSet;

    fn compile_records(plan: &Plan, records: &[Plan]) -> Pruning {
        let domains = records.iter().map(Domain::new).collect::<Vec<_>>();
        Pruning::compile_with(plan, |query, checks, deadline| {
            domains.iter().any(|domain| {
                if *checks == 0 || Instant::now() >= deadline {
                    return false;
                }
                *checks -= 1;
                domain.covers_plan(query)
            })
        })
    }

    #[test]
    fn trie_uses_only_assigned_original_holes_and_terminal_ancestors() {
        let pruning = Pruning::from_prefixes(vec![2, 5, 8], vec![vec![7, 9], vec![4]]);
        let mut phrase = [0; 12];
        phrase[2] = 7;
        phrase[5] = 9;
        assert!(!pruning.covers(&phrase, 0));
        assert!(!pruning.covers(&phrase, 1));
        assert!(pruning.covers(&phrase, 2));
        phrase[8] = 99;
        assert!(pruning.covers(&phrase, 3));
        phrase[5] = 8;
        assert!(!pruning.covers(&phrase, 3));
        phrase[2] = 4;
        assert!(pruning.covers(&phrase, 1));
        assert!(Pruning::from_prefixes(vec![2], vec![vec![]]).covers(&phrase, 0));
    }

    #[test]
    fn compiled_prefixes_never_exclude_uncovered_batch_origins() {
        let mut slots = [Slot::Fixed(0); 12];
        slots[2..5].fill(Slot::Hole);
        let plan = Plan::Batches {
            slots,
            post: vec![1, 2],
            video: vec![1, 3, 4],
            post_open: 1,
        };
        let parts = plan.split_slot(2, 10).unwrap();
        let shared = parts
            .iter()
            .filter(|part| part.slots()[2] == Slot::Fixed(1))
            .cloned()
            .collect::<Vec<_>>();
        assert_eq!(shared.len(), 2);
        for records in [vec![shared[0].clone()], vec![shared[1].clone()], shared] {
            let pruning = compile_records(&plan, &records);
            let covered: HashSet<_> = records.iter().flat_map(Plan::stream).collect();
            for phrase in plan.stream() {
                for depth in 0..=3 {
                    assert!(!pruning.covers(&phrase, depth) || covered.contains(&phrase));
                }
            }
            if records.len() == 2 {
                let mut phrase = [0; 12];
                phrase[2] = 1;
                assert!(pruning.covers(&phrase, 1));
            } else {
                let mut phrase = [0; 12];
                phrase[2] = 1;
                assert!(!pruning.covers(&phrase, 1));
            }
        }
    }

    #[test]
    fn incomplete_family_split_and_expired_budget_produce_no_proof() {
        let mut slots = [Slot::Fixed(0); 12];
        slots[..2].fill(Slot::Hole);
        let plan = Plan::Batches {
            slots,
            post: vec![1, 2],
            video: vec![1, 3],
            post_open: 1,
        };
        let family = plan
            .split_slot(0, 4)
            .unwrap()
            .into_iter()
            .filter(|p| p.slots()[0] == Slot::Fixed(1))
            .collect::<Vec<_>>();
        // Enough for the first origin, insufficient to represent the second.
        let mut budget = Budget {
            plans: 2,
            checks: 100,
            deadline: Instant::now() + Duration::from_secs(1),
        };
        let mut proof = |p: &Plan, _: &mut usize, _: Instant| !p.slots().contains(&Slot::Hole);
        let node = compile_node(family, 1, &[0, 1], &mut proof, &mut budget);
        assert!(node.is_empty());
        budget.deadline = Instant::now();
        let node = compile_node(vec![plan], 0, &[0, 1], &mut |_, _, _| true, &mut budget);
        assert!(node.is_empty());
    }
}
