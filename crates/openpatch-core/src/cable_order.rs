//! Persisted visual plug order, independent of DSP cable-array accumulation.
use crate::{Cable, MAX_CABLES};
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CableEndpoint {
    Input,
    Output,
}

impl CableEndpoint {
    fn rank(self, cable: &Cable) -> Option<u16> {
        match self {
            Self::Input => cable.input_order,
            Self::Output => cable.output_order,
        }
    }
    fn set_rank(self, cable: &mut Cable, rank: u16) {
        match self {
            Self::Input => cable.input_order = Some(rank),
            Self::Output => cable.output_order = Some(rank),
        }
    }
    fn jack(self, cable: &Cable) -> (&str, &str) {
        match self {
            Self::Input => (&cable.to, &cable.input),
            Self::Output => (&cable.from, &cable.output),
        }
    }
}

fn all_order(cables: &[Cable], end: CableEndpoint) -> Vec<usize> {
    let mut order: Vec<_> = (0..cables.len()).collect();
    order.sort_by_key(|&index| (end.rank(&cables[index]).unwrap_or(0), index));
    order
}

/// Return indices at one jack bottom-to-top. Missing ranks are zero; ties
/// preserve cable-array order. Resolving never mutates or migrates old patches.
pub fn cable_endpoint_order(
    cables: &[Cable],
    end: CableEndpoint,
    node: &str,
    port: &str,
) -> Vec<usize> {
    let mut order: Vec<_> = (0..cables.len())
        .filter(|&index| end.jack(&cables[index]) == (node, port))
        .collect();
    order.sort_by_key(|&index| (end.rank(&cables[index]).unwrap_or(0), index));
    order
}

fn selection(
    cables: &[Cable],
    selected: &[usize],
    cable_limit: usize,
) -> Result<BTreeSet<usize>, String> {
    if cables.len() > cable_limit {
        return Err(format!("endpoint order exceeds {cable_limit} cables"));
    }
    if selected.is_empty() || selected.len() > MAX_CABLES {
        return Err(format!(
            "endpoint selection must contain 1..{MAX_CABLES} indices"
        ));
    }
    let mut seen = BTreeSet::new();
    for &index in selected {
        if index >= cables.len() {
            return Err("endpoint selection index is out of bounds".into());
        }
        if !seen.insert(index) {
            return Err("endpoint selection indices must be unique".into());
        }
    }
    Ok(seen)
}

/// Promote the selected plugs in supplied bottom-to-top order. An already-top
/// selection at every affected jack is a true no-op, including absent/tied
/// ranks. On an actual relative-order change, densely normalize this endpoint
/// family only. Opposite ranks and cable-array positions are untouched.
/// This control-thread helper allocates; never call it from audio processing.
pub fn promote_cable_endpoints(
    cables: &mut [Cable],
    end: CableEndpoint,
    selected: &[usize],
) -> Result<bool, String> {
    let seen = selection(cables, selected, MAX_CABLES)?;
    let current = all_order(cables, end);
    let changes = {
        let jacks: BTreeSet<_> = selected
            .iter()
            .map(|&index| end.jack(&cables[index]))
            .collect();
        jacks.iter().any(|&jack| {
            let old: Vec<_> = current
                .iter()
                .copied()
                .filter(|&index| end.jack(&cables[index]) == jack)
                .collect();
            let desired: Vec<_> = old
                .iter()
                .copied()
                .filter(|index| !seen.contains(index))
                .chain(
                    selected
                        .iter()
                        .copied()
                        .filter(|&index| end.jack(&cables[index]) == jack),
                )
                .collect();
            old != desired
        })
    };
    if !changes {
        return Ok(false);
    }
    let desired: Vec<_> = current
        .into_iter()
        .filter(|index| !seen.contains(index))
        .chain(selected.iter().copied())
        .collect();
    for (rank, index) in desired.into_iter().enumerate() {
        end.set_rank(&mut cables[index], (rank + 1) as u16);
    }
    Ok(true)
}

/// Give genuinely new cables fresh top ranks at both ends, in supplied order.
/// Imported/replaced documents must retain their ranks and must not call this.
/// Supports at most 2× the cable limit so module duplication can prepare a
/// transient candidate before the shared final graph validation/removal step.
/// Overflowing imported ranks are normalized without changing effective order.
pub fn assign_new_cable_orders(cables: &mut [Cable], selected: &[usize]) -> Result<(), String> {
    let seen = selection(cables, selected, MAX_CABLES * 2)?;
    for end in [CableEndpoint::Input, CableEndpoint::Output] {
        let old: Vec<_> = all_order(cables, end)
            .into_iter()
            .filter(|index| !seen.contains(index))
            .collect();
        let highest = old
            .iter()
            .map(|&index| end.rank(&cables[index]).unwrap_or(0))
            .max()
            .unwrap_or(0);
        if usize::from(highest) + selected.len() <= usize::from(u16::MAX) {
            for (offset, &index) in selected.iter().enumerate() {
                end.set_rank(
                    &mut cables[index],
                    (usize::from(highest) + offset + 1) as u16,
                );
            }
        } else {
            for (rank, index) in old.into_iter().chain(selected.iter().copied()).enumerate() {
                end.set_rank(&mut cables[index], (rank + 1) as u16);
            }
        }
    }
    Ok(())
}
