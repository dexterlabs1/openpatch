//! Graph-owned exclusive automation claims. All keys, slots and scratch are
//! prepared off the callback; runtime operations only compare existing keys
//! and update indices. A vacant claim is a tombstone, never a fallback writer.
use crate::{Node, MAX_NODES};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Writer {
    pub node: usize,
    pub lane: usize,
}

#[derive(Debug)]
struct Entry {
    key: (String, String),
    owner: Option<Writer>,
}

#[derive(Debug)]
pub(crate) struct SequenceOwnership {
    entries: Vec<Entry>,
    writer_ids: Vec<Option<String>>,
    pub slots: Vec<[Option<usize>; 8]>,
    pub acquire: Vec<bool>,
}

impl SequenceOwnership {
    pub fn new(nodes: &[Node]) -> Self {
        let keys: BTreeSet<_> = nodes
            .iter()
            .filter_map(|node| node.rackmcp_sequence.as_ref())
            .flat_map(|state| state.targets.iter())
            .map(|target| (target.module.clone(), target.parameter.clone()))
            .collect();
        Self {
            entries: keys
                .into_iter()
                .map(|key| Entry { key, owner: None })
                .collect(),
            writer_ids: nodes
                .iter()
                .map(|node| (node.kind == "rackmcp_sequencer").then(|| node.id.clone()))
                .collect(),
            slots: vec![[None; 8]; nodes.len()],
            acquire: vec![false; nodes.len()],
        }
    }

    pub fn slot(&self, module: &str, parameter: &str) -> Option<usize> {
        self.entries
            .binary_search_by(|entry| {
                (entry.key.0.as_str(), entry.key.1.as_str()).cmp(&(module, parameter))
            })
            .ok()
    }

    pub fn claim(&mut self, node: usize) {
        // Native node/lane order is the explicit cold-graph policy. Subsequent
        // calls follow host event order, independently of DSP traversal order.
        for (lane, slot) in self.slots[node].iter().enumerate() {
            if let Some(slot) = slot {
                self.entries[*slot].owner = Some(Writer { node, lane });
            }
        }
    }

    pub fn owns(&self, node: usize, lane: usize) -> bool {
        self.slots[node][lane]
            .is_some_and(|slot| self.entries[slot].owner == Some(Writer { node, lane }))
    }

    pub fn restore(
        &mut self,
        old: &Self,
        nodes: &BTreeMap<String, usize>,
        excluded: &[bool; MAX_NODES],
    ) {
        for (slot, entry) in self.entries.iter_mut().enumerate() {
            entry.owner = old
                .slot(&entry.key.0, &entry.key.1)
                .and_then(|index| old.entries[index].owner)
                .and_then(|owner| {
                    let id = old.writer_ids[owner.node].as_ref()?;
                    let &node = nodes.get(id)?;
                    if excluded[node]
                        || self.writer_ids[node].is_none()
                        || self.slots[node][owner.lane] != Some(slot)
                    {
                        return None;
                    }
                    Some(Writer {
                        node,
                        lane: owner.lane,
                    })
                });
        }
    }
}
