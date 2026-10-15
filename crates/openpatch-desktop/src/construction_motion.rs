//! Original, bounded construction presentation. Receipts authorize cues; clocks
//! only expire/draw them. This module never edits graph, audio, view or selection.
use openpatch_core::Patch;
use serde_json::Value;
use std::{
    collections::BTreeSet,
    time::{Duration, Instant},
};
pub(crate) const MAX_TARGETS: usize = 128;
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum MotionMode {
    #[default]
    Full,
    Reduced,
    Off,
}
impl MotionMode {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Full => "Full",
            Self::Reduced => "Reduced",
            Self::Off => "Off",
        }
    }
}
#[derive(Clone, Debug)]
pub(crate) struct ModuleCue {
    pub id: String,
    pub kind: String,
    pub started: Instant,
}
#[derive(Clone, Debug)]
pub(crate) struct ParameterCue {
    pub id: String,
    pub kind: String,
    pub parameter: String,
    pub old: f32,
    pub new: f32,
    pub started: Instant,
}
#[derive(Default)]
pub(crate) struct Tracker {
    pub mode: MotionMode,
    pub cables: crate::cable_animation::Tracker,
    plan: Option<u64>,
    seen: Option<(u64, u64, u64)>,
    pub modules: Vec<ModuleCue>,
    pub parameters: Vec<ParameterCue>,
    pub focus: Vec<ModuleCue>,
}
pub(crate) enum Observation {
    Preserve,
    Clear,
    Commit,
}
pub(crate) fn ease(t: f32) -> f32 {
    1.0 - (1.0 - t.clamp(0.0, 1.0)).powi(3)
}
pub(crate) fn fraction(started: Instant, now: Instant, millis: u64) -> f32 {
    (now.saturating_duration_since(started).as_secs_f32() / (millis as f32 / 1000.0))
        .clamp(0.0, 1.0)
}
fn effective(patch: &Patch, id: &str, kind: &str, param: &str, catalog: &Value) -> Option<f32> {
    let mut nodes = patch.nodes.iter().filter(|n| n.id == id);
    let node = nodes.next()?;
    if nodes.next().is_some() || node.kind != kind {
        return None;
    }
    let spec = catalog["modules"]
        .as_array()?
        .iter()
        .find(|m| m["kind"] == kind)?["params"]
        .get(param)?;
    if spec["hidden"] == true {
        return None;
    }
    let value = node
        .params
        .get(param)
        .copied()
        .or_else(|| spec["default"].as_f64().map(|v| v as f32))?;
    value.is_finite().then_some(value)
}
impl Tracker {
    pub fn clear(&mut self) {
        self.cables.clear();
        self.modules.clear();
        self.parameters.clear();
        self.focus.clear();
    }
    pub fn set_mode(&mut self, mode: MotionMode) -> bool {
        if self.mode == mode {
            return false;
        }
        self.mode = mode;
        self.cables.set_mode(mode);
        self.clear();
        true
    }
    pub fn tick(&mut self, now: Instant) -> bool {
        let cable_active = self.cables.tick(now);
        let reduced = self.mode == MotionMode::Reduced;
        self.modules
            .retain(|m| fraction(m.started, now, if reduced { 300 } else { 320 }) < 1.0);
        self.parameters
            .retain(|p| fraction(p.started, now, if reduced { 300 } else { 360 }) < 1.0);
        self.focus
            .retain(|m| fraction(m.started, now, if reduced { 300 } else { 420 }) < 1.0);
        cable_active
            || !self.modules.is_empty()
            || !self.parameters.is_empty()
            || !self.focus.is_empty()
    }
    #[cfg(test)]
    pub fn progress(&self, cable: &openpatch_core::Cable, now: Instant) -> Option<f32> {
        self.cables.progress(cable, now)
    }
    pub fn parameter(&self, id: &str, kind: &str, name: &str) -> Option<&ParameterCue> {
        self.parameters
            .iter()
            .find(|p| p.id == id && p.kind == kind && p.parameter == name)
    }
    pub fn interrupt_parameter(&mut self, id: &str, name: &str) {
        self.parameters
            .retain(|p| !(p.id == id && p.parameter == name));
    }
    pub fn interrupt_module(&mut self, id: &str) {
        self.modules.retain(|p| p.id != id);
        self.focus.retain(|p| p.id != id);
    }
    #[allow(clippy::too_many_arguments)]
    pub fn observe(
        &mut self,
        before: &Patch,
        after: &Patch,
        previous: u64,
        revision: u64,
        snapshot: &Value,
        catalog: &Value,
        now: Instant,
    ) -> Observation {
        self.tick(now);
        // Later receipt overflow or removal must not leave an old-value badge
        // attached to a different current control or module incarnation.
        self.modules
            .retain(|m| after.nodes.iter().any(|n| n.id == m.id && n.kind == m.kind));
        self.focus
            .retain(|m| after.nodes.iter().any(|n| n.id == m.id && n.kind == m.kind));
        self.parameters
            .retain(|p| effective(after, &p.id, &p.kind, &p.parameter, catalog) == Some(p.new));
        let plan = snapshot["plan_id"].as_u64();
        let new_plan = self.plan != plan;
        if new_plan {
            self.clear();
            self.seen = None;
            self.plan = plan;
        }
        let receipt = &snapshot["last_commit"];
        let key = receipt["plan_id"]
            .as_u64()
            .zip(receipt["step_index"].as_u64())
            .zip(receipt["revision"].as_u64())
            .map(|((p, s), r)| (p, s, r));
        let Some((age_ms, invalidate_cables)) = receipt["age_ms"]
            .as_u64()
            .zip(receipt["invalidate_existing_cables"].as_bool())
        else {
            self.seen = key;
            self.clear();
            return Observation::Clear;
        };
        if key.is_some() && key == self.seen {
            if (previous == revision || previous.checked_add(1) == Some(revision))
                && key.is_some_and(|(_, _, r)| r == revision)
            {
                return if new_plan {
                    Observation::Clear
                } else {
                    Observation::Preserve
                };
            }
            self.clear();
            return Observation::Clear;
        }
        // Even rejected/disabled receipts are consumed: re-enabling cannot replay.
        let prior_seen = self.seen;
        self.seen = key;
        let valid = key.is_some_and(|(p, s, r)| {
            Some(p) == plan
                && r == revision
                && receipt["before_revision"].as_u64() == Some(previous)
                && previous.checked_add(1) == Some(revision)
                && snapshot["completed_steps"].as_u64() == s.checked_add(1)
                && snapshot["last_revision"].as_u64() == Some(revision)
                && snapshot["current_revision"].as_u64() == Some(revision)
                && prior_seen
                    .filter(|(old_plan, _, _)| *old_plan == p)
                    .is_none_or(|(_, old_step, old_revision)| {
                        old_step.checked_add(1) == Some(s) && old_revision.checked_add(1) == Some(r)
                    })
        });
        if !valid || self.mode == MotionMode::Off {
            self.clear();
            return Observation::Clear;
        }
        let Some(modules) = receipt["new_modules"].as_array() else {
            self.clear();
            return Observation::Clear;
        };
        let Some(cables) = receipt["new_cables"].as_array() else {
            self.clear();
            return Observation::Clear;
        };
        let Some(parameters) = receipt["parameter_changes"].as_array() else {
            self.clear();
            return Observation::Clear;
        };
        if modules.len() + cables.len() + parameters.len() > MAX_TARGETS {
            self.clear();
            return Observation::Clear;
        }
        // Timestamp from the authoritative commit, not from first GUI observation.
        // Large ages are consumed without creating a delayed presentation backlog.
        let Some(started) = now.checked_sub(Duration::from_millis(age_ms.min(420))) else {
            self.clear();
            return Observation::Clear;
        };
        let reduced = self.mode == MotionMode::Reduced;
        let mut arrivals = Vec::new();
        let mut changes = Vec::new();
        let mut cable_ids = Vec::new();
        let mut affected = BTreeSet::new();
        let mut identities = BTreeSet::new();
        let validate = (|| -> Option<()> {
            for m in modules {
                let id = m["id"].as_str()?;
                let kind = m["kind"].as_str()?;
                if !identities.insert(format!("module:{id}"))
                    || after.nodes.iter().filter(|n| n.id == id).count() != 1
                    || after
                        .nodes
                        .iter()
                        .find(|n| n.id == id)
                        .is_none_or(|n| n.kind != kind)
                {
                    return None;
                }
                arrivals.push(ModuleCue {
                    id: id.into(),
                    kind: kind.into(),
                    started,
                });
                affected.insert((id.to_owned(), kind.to_owned()));
            }
            for c in cables {
                let id = c["id"].as_str()?;
                if !identities.insert(format!("cable:{id}"))
                    || before.cables.iter().any(|c| c.id.as_deref() == Some(id))
                {
                    return None;
                }
                let mut matching = after.cables.iter().filter(|c| c.id.as_deref() == Some(id));
                let actual = matching.next()?;
                if matching.next().is_some()
                    || c["from"] != actual.from
                    || c["output"] != actual.output
                    || c["to"] != actual.to
                    || c["input"] != actual.input
                {
                    return None;
                }
                for node in [&actual.from, &actual.to] {
                    let n = after.nodes.iter().find(|n| &n.id == node)?;
                    affected.insert((n.id.clone(), n.kind.clone()));
                }
                cable_ids.push(id.into());
            }
            for p in parameters {
                let id = p["id"].as_str()?;
                let kind = p["kind"].as_str()?;
                let parameter = p["parameter"].as_str()?;
                let old = p["old_effective"].as_f64()? as f32;
                let new = p["new_effective"].as_f64()? as f32;
                if !identities.insert(format!("parameter:{id}:{parameter}"))
                    || arrivals.iter().any(|m| m.id == id)
                    || old == new
                    || Some(old) != effective(before, id, kind, parameter, catalog)
                    || Some(new) != effective(after, id, kind, parameter, catalog)
                {
                    return None;
                }
                changes.push(ParameterCue {
                    id: id.into(),
                    kind: kind.into(),
                    parameter: parameter.into(),
                    old,
                    new,
                    started,
                });
                affected.insert((id.into(), kind.into()));
            }
            Some(())
        })();
        if validate.is_none() {
            self.clear();
            return Observation::Clear;
        }
        if invalidate_cables {
            self.cables.clear();
        }
        // Keep overlap bounded. New effects take priority; overflow is ordinary UI.
        let incoming = arrivals.len() + changes.len() + cable_ids.len();
        if self.modules.len() + self.parameters.len() + self.cables.len() + incoming > MAX_TARGETS {
            self.clear();
        }
        for m in arrivals {
            self.modules.retain(|old| old.id != m.id);
            self.parameters.retain(|p| p.id != m.id);
            if age_ms < if reduced { 300 } else { 320 } {
                self.modules.push(m);
            }
        }
        for p in changes {
            self.parameters
                .retain(|old| !(old.id == p.id && old.parameter == p.parameter));
            if age_ms < if reduced { 300 } else { 360 } {
                self.parameters.push(p);
            }
        }
        for (id, kind) in affected.into_iter().take(MAX_TARGETS) {
            self.focus.retain(|m| m.id != id);
            if age_ms < if reduced { 300 } else { 420 } {
                self.focus.push(ModuleCue { id, kind, started });
            }
        }
        if self.focus.len() > MAX_TARGETS {
            self.focus.drain(..self.focus.len() - MAX_TARGETS);
        }
        if age_ms < if reduced { 300 } else { 420 } {
            self.cables.observe_ids(before, after, &cable_ids, started);
        }
        Observation::Commit
    }
}
#[cfg(test)]
#[path = "construction_motion_tests.rs"]
mod tests;

#[derive(Clone, Debug)]
pub(crate) enum Marker {
    Knob {
        center: eframe::egui::Pos2,
        radius: f32,
        old: f32,
        new: f32,
    },
    Slider {
        rect: eframe::egui::Rect,
        old: f32,
        new: f32,
    },
}
#[derive(Clone, Debug)]
pub(crate) struct Anchor {
    pub rect: eframe::egui::Rect,
    pub old_text: String,
    pub new_text: String,
    pub marker: Option<Marker>,
}
