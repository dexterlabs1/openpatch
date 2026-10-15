//! Bounded presentation state for newly constructed cables. Never edits a patch.
use openpatch_core::{Cable, Patch};
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

const MAX_ACTIVE: usize = 128;
const REVEAL: Duration = Duration::from_millis(300);
const DURATION: Duration = Duration::from_millis(420);
use crate::construction_motion::MotionMode;

#[derive(Clone, Copy, Debug)]
pub(crate) struct Phase {
    pub progress: f32,
    pub settle: f32,
    pub fade: f32,
    pub reduced: bool,
}

#[derive(Default)]
pub(crate) struct Tracker {
    active: HashMap<String, Animation>,
    mode: MotionMode,
}

struct Animation {
    from: String,
    output: String,
    to: String,
    input: String,
    started: Instant,
}

impl Animation {
    fn new(cable: &Cable, now: Instant) -> Self {
        Self {
            from: cable.from.clone(),
            output: cable.output.clone(),
            to: cable.to.clone(),
            input: cable.input.clone(),
            started: now,
        }
    }

    fn matches_route(&self, cable: &Cable) -> bool {
        self.from == cable.from
            && self.output == cable.output
            && self.to == cable.to
            && self.input == cable.input
    }
}

// Treat duplicate IDs as ambiguous even when both routes are identical. Valid
// patches forbid them, but presentation should fail closed on partial input.
fn unique_cables(patch: &Patch) -> HashMap<&str, Option<&Cable>> {
    let mut cables = HashMap::with_capacity(patch.cables.len());
    for cable in &patch.cables {
        if let Some(id) = cable.id.as_deref().filter(|id| !id.is_empty()) {
            cables
                .entry(id)
                .and_modify(|entry| *entry = None)
                .or_insert(Some(cable));
        }
    }
    cables
}

impl Tracker {
    /// Observe one authorized construction transition. A stable ID absent from
    /// `before` is new; changing an existing ID's route never animates a move.
    /// The caller invokes this once per newly observed eligible revision, never
    /// to replay an earlier transition during ordinary redraws.
    /// Returns whether the caller should keep requesting animation redraws.
    #[cfg(test)]
    pub(crate) fn observe(&mut self, before: &Patch, after: &Patch, now: Instant) -> bool {
        let ids = after
            .cables
            .iter()
            .filter_map(|c| c.id.clone())
            .collect::<Vec<_>>();
        self.observe_ids(before, after, &ids, now)
    }
    pub(crate) fn observe_ids(
        &mut self,
        before: &Patch,
        after: &Patch,
        ids: &[String],
        now: Instant,
    ) -> bool {
        if self.mode == MotionMode::Off {
            return false;
        }
        self.prune(after, now);
        let after_cables = unique_cables(after);
        let prior_ids: HashSet<_> = before
            .cables
            .iter()
            .filter_map(|cable| cable.id.as_deref())
            .collect();
        for cable in &after.cables {
            let Some(id) = cable.id.as_deref().filter(|id| !id.is_empty()) else {
                continue;
            };
            if !ids.iter().any(|allowed| allowed == id)
                || prior_ids.contains(id)
                || after_cables.get(id).is_none_or(|entry| entry.is_none())
                || self.active.contains_key(id)
            {
                continue;
            }
            if self.active.len() == MAX_ACTIVE {
                break;
            }
            self.active
                .insert(id.to_owned(), Animation::new(cable, now));
        }
        !self.active.is_empty()
    }

    /// Eased progress for this exact ID and route. None means draw normally.
    /// Presentation metadata (RGB and plug ranks) does not restart animation.
    #[cfg(test)]
    pub(crate) fn progress(&self, cable: &Cable, now: Instant) -> Option<f32> {
        self.phase(cable, now).map(|p| p.progress)
    }
    pub(crate) fn phase(&self, cable: &Cable, now: Instant) -> Option<Phase> {
        let entry = self.active.get(cable.id.as_deref()?)?;
        if !entry.matches_route(cable) {
            return None;
        }
        let elapsed = now.saturating_duration_since(entry.started);
        if elapsed >= self.duration() {
            return None;
        }
        if self.mode == MotionMode::Reduced {
            return Some(Phase {
                progress: 1.0,
                settle: 0.0,
                fade: 1.0 - elapsed.as_secs_f32() / 0.3,
                reduced: true,
            });
        }
        let progress =
            crate::construction_motion::ease(elapsed.as_secs_f32() / REVEAL.as_secs_f32());
        let settle = ((elapsed.as_secs_f32() - 0.3) / 0.12).clamp(0.0, 1.0);
        Some(Phase {
            progress,
            settle,
            fade: 1.0 - settle,
            reduced: false,
        })
    }
    fn duration(&self) -> Duration {
        match self.mode {
            MotionMode::Full => DURATION,
            MotionMode::Reduced => REVEAL,
            MotionMode::Off => Duration::ZERO,
        }
    }
    pub(crate) fn set_mode(&mut self, mode: MotionMode) {
        if self.mode != mode {
            self.clear();
            self.mode = mode;
        }
    }
    pub(crate) fn len(&self) -> usize {
        self.active.len()
    }
    pub(crate) fn interrupt(&mut self, cable: &Cable) {
        if let Some(id) = &cable.id {
            self.active.remove(id);
        }
    }

    /// Validate entries against a new patch, removing deleted or changed routes.
    pub(crate) fn prune(&mut self, patch: &Patch, now: Instant) -> bool {
        self.prune_with(&unique_cables(patch), now);
        !self.active.is_empty()
    }

    /// Allocation-free ordinary redraw expiry, touching at most 128 entries.
    /// A hidden window resumes with completed cables and no animation backlog.
    /// The host must observe or clear on every intervening revision transition.
    pub(crate) fn tick(&mut self, now: Instant) -> bool {
        let duration = self.duration();
        self.active
            .retain(|_, entry| now.saturating_duration_since(entry.started) < duration);
        !self.active.is_empty()
    }

    fn prune_with(&mut self, cables: &HashMap<&str, Option<&Cable>>, now: Instant) {
        let duration = self.duration();
        self.active.retain(|id, entry| {
            now.saturating_duration_since(entry.started) < duration
                && cables
                    .get(id.as_str())
                    .and_then(|cable| *cable)
                    .is_some_and(|cable| entry.matches_route(cable))
        });
    }

    pub(crate) fn clear(&mut self) {
        self.active.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cable(id: &str) -> Cable {
        Cable {
            id: Some(id.into()),
            from: "source".into(),
            output: "out".into(),
            to: "destination".into(),
            input: "signal".into(),
            color: None,
            input_order: None,
            output_order: None,
        }
    }

    fn patch(cables: Vec<Cable>) -> Patch {
        let mut patch = Patch::new_v2();
        patch.cables = cables;
        patch
    }

    #[test]
    fn new_cables_ease_out_and_expire_on_redraw_without_another_transition() {
        let now = Instant::now();
        let mut tracker = Tracker::default();
        let empty = patch(vec![]);
        let after = patch(vec![cable("new")]);
        assert!(tracker.observe(&empty, &after, now));
        assert_eq!(tracker.progress(&after.cables[0], now), Some(0.0));
        assert_eq!(
            tracker.progress(&after.cables[0], now + REVEAL / 2),
            Some(0.875)
        );
        assert!(tracker.prune(&after, now + DURATION - Duration::from_nanos(1)));
        assert_eq!(tracker.progress(&after.cables[0], now + DURATION), None);
        assert!(!tracker.prune(&after, now + DURATION));
        assert!(tracker.active.is_empty());
        // A delayed/hidden-window redraw expires instead of replaying a queue.
        assert!(!tracker.tick(now + Duration::from_secs(30)));
    }

    #[test]
    fn cosmetic_changes_and_cable_reordering_do_not_restart_existing_entries() {
        let now = Instant::now();
        let mut tracker = Tracker::default();
        let before = patch(vec![cable("old")]);
        let after = patch(vec![cable("old"), cable("new")]);
        tracker.observe(&before, &after, now);
        let mut cosmetics = after.clone();
        for cable in &mut cosmetics.cables {
            cable.color = Some([1, 2, 3]);
            cable.input_order = Some(9);
            cable.output_order = Some(7);
        }
        cosmetics.cables.reverse();
        let half = now + REVEAL / 2;
        assert!(tracker.observe(&after, &cosmetics, half));
        assert_eq!(tracker.active.len(), 1);
        assert_eq!(tracker.progress(&cosmetics.cables[0], half), Some(0.875));
        assert_eq!(tracker.progress(&cosmetics.cables[1], half), None);
        assert!(!tracker.prune(&cosmetics, now + DURATION));
    }

    #[test]
    fn changing_any_route_endpoint_prunes_and_never_animates_a_move() {
        for endpoint in ["from", "output", "to", "input"] {
            let now = Instant::now();
            let mut tracker = Tracker::default();
            let before = patch(vec![cable("same")]);
            tracker.observe(&patch(vec![]), &before, now);
            let mut after = before.clone();
            let route = &mut after.cables[0];
            match endpoint {
                "from" => route.from = "different".into(),
                "output" => route.output = "different".into(),
                "to" => route.to = "different".into(),
                "input" => route.input = "different".into(),
                _ => unreachable!(),
            }
            // ID-only lookup must not attach stale progress to a changed route.
            assert_eq!(tracker.progress(route, now), None);
            assert!(!tracker.observe(&before, &after, now + Duration::from_millis(1)));
            assert!(tracker.active.is_empty());
        }
    }

    #[test]
    fn removal_and_id_reuse_require_a_fresh_observed_addition() {
        let now = Instant::now();
        let mut tracker = Tracker::default();
        let empty = patch(vec![]);
        let first = patch(vec![cable("reused")]);
        tracker.observe(&empty, &first, now);
        assert!(!tracker.observe(&first, &empty, now + Duration::from_millis(20)));
        let mut reused = first.clone();
        reused.cables[0].to = "another destination".into();
        assert_eq!(
            tracker.progress(&reused.cables[0], now + Duration::from_millis(20)),
            None
        );
        let added = now + Duration::from_millis(30);
        assert!(tracker.observe(&empty, &reused, added));
        assert_eq!(tracker.progress(&reused.cables[0], added), Some(0.0));
        assert_eq!(tracker.progress(&first.cables[0], added), None);
        assert!(!tracker.prune(&empty, added));
        // Restoring the exact original ID and route is also a fresh valid
        // addition after removal, regardless of an earlier identical commit.
        let restored = added + Duration::from_millis(10);
        assert!(tracker.observe(&empty, &first, restored));
        assert_eq!(tracker.progress(&first.cables[0], restored), Some(0.0));
    }

    #[test]
    fn identical_stacked_routes_have_independent_stable_ids() {
        let now = Instant::now();
        let mut tracker = Tracker::default();
        let before = patch(vec![cable("old")]);
        let after = patch(vec![cable("old"), cable("stack1"), cable("stack2")]);
        assert!(tracker.observe(&before, &after, now));
        assert_eq!(tracker.active.len(), 2);
        assert_eq!(tracker.progress(&after.cables[0], now), None);
        assert_eq!(tracker.progress(&after.cables[1], now), Some(0.0));
        assert_eq!(tracker.progress(&after.cables[2], now), Some(0.0));
        let remaining = patch(vec![after.cables[0].clone(), after.cables[2].clone()]);
        tracker.prune(&remaining, now);
        assert_eq!(tracker.active.len(), 1);
        assert_eq!(tracker.progress(&after.cables[1], now), None);
        assert_eq!(tracker.progress(&after.cables[2], now), Some(0.0));
    }

    #[test]
    fn cap_drops_excess_additions_instead_of_queuing_them_for_later() {
        let now = Instant::now();
        let mut tracker = Tracker::default();
        let empty = patch(vec![]);
        let after = patch(
            (0..MAX_ACTIVE + 20)
                .map(|id| cable(&format!("c{id}")))
                .collect(),
        );
        tracker.observe(&empty, &after, now);
        assert_eq!(tracker.active.len(), MAX_ACTIVE);
        assert_eq!(tracker.progress(&after.cables[MAX_ACTIVE], now), None);
        assert!(!tracker.prune(&after, now + DURATION));
        assert!(!tracker.observe(&after, &after, now + DURATION));
        let mut next = after.clone();
        next.cables.push(cable("actually-new"));
        assert!(tracker.observe(&after, &next, now + DURATION));
        assert_eq!(tracker.active.len(), 1);
        assert_eq!(
            tracker.progress(next.cables.last().unwrap(), now + DURATION),
            Some(0.0)
        );
    }

    #[test]
    fn anonymous_empty_and_duplicate_ids_fail_closed() {
        let now = Instant::now();
        let mut anonymous = cable("unused");
        anonymous.id = None;
        let mut tracker = Tracker::default();
        let malformed = patch(vec![
            anonymous,
            cable(""),
            cable("duplicate"),
            cable("duplicate"),
        ]);
        assert!(!tracker.observe(&patch(vec![]), &malformed, now));
        let unique = patch(vec![cable("duplicate")]);
        tracker.observe(&patch(vec![]), &unique, now);
        assert!(!tracker.prune(&malformed, now));
        assert_eq!(tracker.progress(&malformed.cables[2], now), None);
    }

    #[test]
    fn repeated_observation_preserves_start_time_and_clear_resets_all_state() {
        let now = Instant::now();
        let empty = patch(vec![]);
        let after = patch(vec![cable("new")]);
        let mut tracker = Tracker::default();
        tracker.observe(&empty, &after, now);
        tracker.observe(&empty, &after, now + REVEAL / 2);
        assert_eq!(
            tracker.progress(&after.cables[0], now + REVEAL / 2),
            Some(0.875)
        );
        assert_eq!(
            tracker.progress(&after.cables[0], now - Duration::from_millis(1)),
            Some(0.0)
        );
        tracker.clear();
        assert!(tracker.active.is_empty());
        assert_eq!(tracker.progress(&after.cables[0], now), None);
        assert!(tracker.observe(&empty, &after, now + DURATION));
    }

    #[test]
    fn idle_tick_expires_active_entries_without_inspecting_a_patch() {
        let now = Instant::now();
        let mut tracker = Tracker::default();
        let after = patch(vec![cable("new")]);
        tracker.observe(&patch(vec![]), &after, now);
        assert!(tracker.tick(now + REVEAL / 2));
        assert!(!tracker.tick(now + DURATION));
        assert!(tracker.active.is_empty());
    }
}
