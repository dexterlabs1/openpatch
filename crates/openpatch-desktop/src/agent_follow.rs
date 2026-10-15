//! Transient camera presentation from already validated construction cues.
use super::*;
use crate::construction_motion::{FollowFocus, FollowTarget, MotionMode};
use std::time::{Duration, Instant};
const TRAVEL_MS: u64 = 220;

#[derive(Default)]
pub(super) struct State {
    pub enabled: bool,
    pub pending: Vec<FollowTarget>,
    revision: Option<u64>,
    flight: Option<Flight>,
    saved_pan: Option<Vec2>,
}
struct Flight {
    from: Vec2,
    to: Vec2,
    started: Instant,
    duration: Duration,
    expires: Instant,
    identities: Vec<(String, String)>,
}
impl State {
    pub fn suspend(&mut self) {
        self.pending.clear();
        self.flight = None;
        self.revision = None;
    }
    pub fn active(&self) -> bool {
        self.flight.is_some()
    }
    pub fn persisted_pan(&self, visible: Vec2) -> Vec2 {
        self.saved_pan.unwrap_or(visible)
    }
    pub fn promote_view(&mut self) {
        self.saved_pan = None;
    }
    pub fn queue(&mut self, targets: &[FollowTarget], revision: u64, now: Instant) {
        self.suspend();
        if self.enabled {
            self.pending.extend(
                targets
                    .iter()
                    .filter(|target| {
                        now.saturating_duration_since(target.started)
                            < Duration::from_millis(TRAVEL_MS)
                    })
                    .take(crate::construction_motion::MAX_TARGETS)
                    .cloned(),
            );
            self.revision = Some(revision);
        }
    }
}

impl Rack {
    pub(crate) fn agent_follow_enabled(&self) -> bool {
        self.agent_follow.enabled
    }
    pub(crate) fn set_agent_follow(&mut self, enabled: bool) {
        self.agent_follow.enabled = enabled;
        self.agent_follow.suspend();
    }
    pub(crate) fn suspend_agent_follow(&mut self) {
        self.agent_follow.suspend();
    }
    pub(crate) fn agent_follow_view_pan(&self) -> Vec2 {
        self.pan
    }

    pub(super) fn tick_agent_follow(&mut self, patch: &Patch, revision: u64, now: Instant) -> bool {
        if !self.agent_follow.enabled || self.agent_follow.revision != Some(revision) {
            self.agent_follow.suspend();
            return false;
        }
        let Some(flight) = self.agent_follow.flight.as_ref() else {
            return false;
        };
        if now >= flight.expires {
            self.agent_follow.suspend();
            return false;
        }
        if flight
            .identities
            .iter()
            .any(|(id, kind)| !patch.nodes.iter().any(|n| &n.id == id && &n.kind == kind))
        {
            self.agent_follow.suspend();
            return false;
        }
        let t = if self.construction_animation.mode == MotionMode::Reduced {
            1.0
        } else {
            (now.saturating_duration_since(flight.started).as_secs_f32()
                / flight.duration.as_secs_f32())
            .clamp(0.0, 1.0)
        };
        let eased = t * t * t * (t * (t * 6.0 - 15.0) + 10.0);
        let pan = flight.from + (flight.to - flight.from) * eased;
        if pan.is_finite() {
            self.agent_follow.saved_pan.get_or_insert(self.pan);
            self.pan = bounded_pan(pan);
        }
        if t >= 1.0 {
            self.agent_follow.flight = None;
        }
        t < 1.0
    }

    pub(super) fn resolve_agent_follow(
        &mut self,
        patch: &Patch,
        catalog: &Value,
        canvas: Rect,
        now: Instant,
    ) {
        let pending = std::mem::take(&mut self.agent_follow.pending);
        if pending.is_empty() || !self.agent_follow.enabled {
            return;
        }
        let inset = (canvas.width().min(canvas.height()) * 0.1).clamp(4.0, 24.0);
        let inner = canvas.shrink(inset);
        if !inner.is_finite() || inner.width() < 1.0 || inner.height() < 1.0 {
            return;
        }
        let mut targets = Vec::new();
        for target in pending {
            if now.saturating_duration_since(target.started) >= Duration::from_millis(TRAVEL_MS) {
                continue;
            }
            let Some(node) = patch
                .nodes
                .iter()
                .find(|n| n.id == target.id && n.kind == target.kind)
            else {
                continue;
            };
            let Some(position) = self.positions.get(&target.id) else {
                continue;
            };
            let origin = canvas.min + self.pan + position.to_vec2() * self.zoom;
            let module =
                Rect::from_min_size(origin, vec2(module_width(node), PANEL_HEIGHT) * self.zoom);
            let (priority, mut rect) = match &target.focus {
                FollowFocus::Parameter(name) => (
                    0,
                    self.construction_anchors
                        .get(&(target.id.clone(), name.clone()))
                        .map_or(module, |anchor| anchor.rect),
                ),
                FollowFocus::Port { input, name } => {
                    let Some(center) = self
                        .port_center_for_smoke(canvas, patch, catalog, &target.id, name, *input)
                    else {
                        continue;
                    };
                    (
                        1,
                        Rect::from_center_size(center, vec2(32.0, 48.0) * self.zoom),
                    )
                }
                FollowFocus::Module => (2, module),
            };
            if !rect.is_finite() {
                continue;
            }
            if rect.width() > inner.width() || rect.height() > inner.height() {
                // Oversized modules show their header; a known control/plug
                // instead keeps its real center without forcing a zoom change.
                rect = if matches!(target.focus, FollowFocus::Module) || rect == module {
                    Rect::from_min_size(
                        rect.min,
                        vec2(
                            rect.width().min(inner.width()),
                            (80.0 * self.zoom).min(inner.height()),
                        ),
                    )
                } else {
                    Rect::from_center_size(rect.center(), rect.size().min(inner.size()))
                };
            }
            let context = if module.width() <= inner.width() && module.height() <= inner.height() {
                module
            } else {
                rect
            };
            targets.push((priority, rect, target, context));
        }
        if targets.is_empty() {
            return;
        }
        // Fit nearby edits together. Far-apart edits choose one meaningful
        // destination: controls, then changed plugs, then nearest module.
        targets.sort_by(|a, b| {
            a.0.cmp(&b.0).then_with(|| {
                a.1.center()
                    .distance_sq(inner.center())
                    .total_cmp(&b.1.center().distance_sq(inner.center()))
            })
        });
        let union = targets
            .iter()
            .fold(Rect::NOTHING, |rect, target| rect.union(target.1));
        // An already visible actual focus does not authorize tidying its
        // surrounding panel. When reveal is needed, retain panel context where
        // it fits, falling back to the actual anchors for dense/oversized edits.
        if inner.contains_rect(union) {
            return;
        }
        let context = targets
            .iter()
            .fold(Rect::NOTHING, |rect, target| rect.union(target.3));
        let rect = if context.width() <= inner.width() && context.height() <= inner.height() {
            context
        } else if union.width() <= inner.width() && union.height() <= inner.height() {
            union
        } else {
            if inner.contains_rect(targets[0].1) {
                return;
            }
            targets[0].3
        };
        let axis = |lo: f32, hi: f32, min: f32, max: f32| {
            if lo < min {
                min - lo
            } else if hi > max {
                max - hi
            } else {
                0.0
            }
        };
        let delta = vec2(
            axis(rect.left(), rect.right(), inner.left(), inner.right()),
            axis(rect.top(), rect.bottom(), inner.top(), inner.bottom()),
        );
        if !delta.is_finite() || delta == Vec2::ZERO {
            return;
        }
        let remaining = Duration::from_millis(TRAVEL_MS)
            .saturating_sub(now.saturating_duration_since(targets[0].2.started));
        if self.construction_animation.mode == MotionMode::Full
            && remaining < Duration::from_millis(80)
        {
            return;
        }
        let to = bounded_pan(self.pan + delta);
        let expires = targets
            .iter()
            .filter_map(|(_, _, target, _)| {
                let millis = if self.construction_animation.mode == MotionMode::Reduced {
                    300
                } else {
                    match target.focus {
                        FollowFocus::Module => 320,
                        FollowFocus::Parameter(_) => 360,
                        FollowFocus::Port { .. } => 420,
                    }
                };
                target.started.checked_add(Duration::from_millis(millis))
            })
            .min();
        let Some(expires) = expires else {
            return;
        };
        let identities = targets
            .into_iter()
            .map(|(_, _, target, _)| (target.id, target.kind))
            .collect();
        self.agent_follow.flight = Some(Flight {
            from: self.pan,
            to,
            started: now,
            duration: remaining,
            expires,
            identities,
        });
    }
}
