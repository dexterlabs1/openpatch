//! Native controls for actual, revision-checked construction work.
use eframe::egui::{self, RichText};
use serde_json::{json, Value};

#[cfg(test)]
#[path = "construction_ui_tests.rs"]
mod tests;

pub struct Panel {
    snapshot: Value,
    dismissed: Option<u64>,
    document_revision: u64,
    motion_mode: crate::construction_motion::MotionMode,
}

impl Default for Panel {
    fn default() -> Self {
        Self {
            snapshot: Value::Null,
            dismissed: None,
            document_revision: 0,
            motion_mode: crate::construction_motion::MotionMode::Full,
        }
    }
}

pub struct Action {
    pub name: &'static str,
    pub arguments: Value,
}

impl Panel {
    pub(crate) fn set_motion_mode(&mut self, mode: crate::construction_motion::MotionMode) {
        self.motion_mode = mode;
    }
    pub(crate) fn motion_mode(&self) -> crate::construction_motion::MotionMode {
        self.motion_mode
    }

    pub fn observe(&mut self, snapshot: Value) {
        self.snapshot = snapshot;
    }

    /// Scope presentation to the accepted document replacement. Session keeps
    /// its retained plan and receipts available to inspect_construction; they
    /// are not progress for the newly installed document. Revisions remain
    /// monotonic across New, so plans started at this boundary stay visible.
    pub(crate) fn begin_document(&mut self, revision: u64) {
        self.document_revision = revision;
    }

    /// Compact construction status for the always-visible bottom bar (replaces the old full-width top strip).
    pub fn footer(&mut self, ui: &mut egui::Ui) -> Option<Action> {
        let plan_id = self.snapshot["plan_id"].as_u64()?;
        if self.snapshot["base_revision"].as_u64().unwrap_or(0) < self.document_revision
            || self.dismissed == Some(plan_id)
        {
            return None;
        }
        let status = self.snapshot["status"].as_str().unwrap_or("failed");
        let (running, paused) = (status == "running", status == "paused");
        let active = running || paused;
        let completed = self.snapshot["completed_steps"].as_u64().unwrap_or(0);
        let total = self.snapshot["total_steps"].as_u64().unwrap_or(0);
        let title = self.snapshot["title"]
            .as_str()
            .unwrap_or("Patch construction");
        let heading = match status {
            "running" => "Building",
            "paused" => "Paused",
            "completed" => "Built",
            "cancelled" => "Cancelled",
            "conflicted" => "Stopped",
            _ => "Stopped",
        };
        let detail = if let Some(error) = self.snapshot["error"].as_str() {
            error.to_string()
        } else {
            self.snapshot[if active { "next_label" } else { "last_label" }]
                .as_str()
                .map(|l| format!("{}: {l}", if active { "Next" } else { "Last step" }))
                .unwrap_or_default()
        };
        let help = format!("{title}\n{detail}\nEach completed step is one real patch edit and one shared Undo entry. Completed steps stay in the rack. Plan {plan_id}.");
        let mut action = None;
        let warn = status == "conflicted" || self.snapshot["error"].is_string();
        ui.label(RichText::new(heading).size(10.0).strong().color(if warn {
            ui.visuals().warn_fg_color
        } else {
            ui.visuals().text_color()
        }))
        .on_hover_text(&help);
        ui.add(
            egui::ProgressBar::new(if total == 0 {
                0.0
            } else {
                completed as f32 / total as f32
            })
            .desired_width(70.0)
            .desired_height(10.0)
            .text(RichText::new(format!("{completed}/{total}")).size(9.0)),
        )
        .on_hover_text(&help);
        if running && ui.small_button("Pause").clicked() {
            action = Some(Action {
                name: "pause_construction",
                arguments: json!({"plan_id":plan_id}),
            });
        }
        if paused {
            let revision = self.snapshot["current_revision"].as_u64();
            let unchanged =
                revision.is_some() && revision == self.snapshot["expected_revision"].as_u64();
            if ui
                .add_enabled(
                    unchanged,
                    egui::Button::new(RichText::new("Resume").size(10.0)).small(),
                )
                .clicked()
            {
                action = Some(Action {
                    name: "resume_construction",
                    arguments: json!({"plan_id":plan_id,"expected_revision":revision.unwrap()}),
                });
            }
        }
        if active
            && ui
                .small_button("Cancel")
                .on_hover_text("Stop future steps and keep completed work.")
                .clicked()
        {
            action = Some(Action {
                name: "cancel_construction",
                arguments: json!({"plan_id":plan_id}),
            });
        }
        if !active && ui.small_button("Dismiss").clicked() {
            self.dismissed = Some(plan_id);
        }
        let mut selected_mode = self.motion_mode;
        egui::ComboBox::from_id_salt("construction-motion")
            .selected_text(RichText::new(format!("Motion: {}", selected_mode.label())).size(10.0))
            .width(92.0)
            .show_ui(ui, |ui| {
                for mode in [crate::construction_motion::MotionMode::Full, crate::construction_motion::MotionMode::Reduced, crate::construction_motion::MotionMode::Off] {
                    ui.selectable_value(&mut selected_mode, mode, mode.label());
                }
            })
            .response
            .on_hover_text("How committed edits animate: Full adds short movement accents; Reduced uses static accents; Off settles immediately.");
        self.set_motion_mode(selected_mode);
        ui.separator();
        action
    }

    #[allow(dead_code)] // the full-width strip is kept for tests and fixtures; the app uses `footer`
    pub fn show(&mut self, ctx: &egui::Context) -> Option<Action> {
        let plan_id = self.snapshot["plan_id"].as_u64()?;
        if self.snapshot["base_revision"].as_u64().unwrap_or(0) < self.document_revision {
            return None;
        }
        if self.dismissed == Some(plan_id) {
            return None;
        }
        let status = self.snapshot["status"].as_str().unwrap_or("failed");
        let running = status == "running";
        let paused = status == "paused";
        let active = running || paused;
        let completed = self.snapshot["completed_steps"].as_u64().unwrap_or(0);
        let total = self.snapshot["total_steps"].as_u64().unwrap_or(0);
        let title = self.snapshot["title"]
            .as_str()
            .unwrap_or("Patch construction");
        let heading = match status {
            "running" => "Building",
            "paused" => "Paused",
            "completed" => "Construction complete",
            "cancelled" => "Construction cancelled",
            "conflicted" => "Stopped — rack changed",
            _ => "Construction stopped",
        };
        let mut action = None;
        let mut selected_mode = self.motion_mode;
        egui::TopBottomPanel::top("construction_progress")
            .show(ctx, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new(heading).strong());
                    ui.label(title);
                    ui.add(
                        egui::ProgressBar::new(if total == 0 {
                            0.0
                        } else {
                            completed as f32 / total as f32
                        })
                        .desired_width(150.0)
                        .text(format!("{completed} of {total} steps")),
                    )
                    .on_hover_text("Each completed step is one real patch edit and one shared Undo entry.");
                    if running && ui.button("Pause").clicked() {
                        action = Some(Action {
                            name: "pause_construction",
                            arguments: json!({"plan_id":plan_id}),
                        });
                    }
                    if paused {
                        let revision = self.snapshot["current_revision"].as_u64();
                        let unchanged = revision.is_some()
                            && revision == self.snapshot["expected_revision"].as_u64();
                        if ui.add_enabled(unchanged, egui::Button::new("Resume"))
                            .on_disabled_hover_text("The rack changed while paused. Review it and start a new plan.")
                            .clicked()
                        {
                            action = Some(Action {
                                name: "resume_construction",
                                arguments: json!({"plan_id":plan_id,"expected_revision":revision.unwrap()}),
                            });
                        }
                    }
                    if active && ui.button("Cancel")
                        .on_hover_text("Stop future steps and keep completed work. Use Undo to reverse individual edits.")
                        .clicked()
                    {
                        action = Some(Action {
                            name: "cancel_construction",
                            arguments: json!({"plan_id":plan_id}),
                        });
                    }
                    if !active && ui.small_button("Dismiss").clicked() {
                        self.dismissed = Some(plan_id);
                    }
                    ui.label(RichText::new("Motion").small().weak());
                    for mode in [crate::construction_motion::MotionMode::Full,crate::construction_motion::MotionMode::Reduced,crate::construction_motion::MotionMode::Off] {
                        ui.selectable_value(&mut selected_mode,mode,mode.label())
                            .on_hover_text("Presentation of committed edits only. Full adds short movement accents; Reduced uses static accents; Off settles everything immediately. Changing modes affects future steps. Playback stays under your control. Enable Follow agent edits in View → Agent motion to reveal future edits. Remembered for this window.");
                    }
                });
                if let Some(error) = self.snapshot["error"].as_str() {
                    ui.label(RichText::new(error).color(ui.visuals().warn_fg_color));
                } else if let Some(label) = self.snapshot[if active { "next_label" } else { "last_label" }].as_str() {
                    ui.label(RichText::new(format!("{}: {label}", if active { "Next" } else { "Last step" })).small());
                }
                ui.label(RichText::new("Completed steps stay in the rack. Playback remains under your control.").small().weak())
                    .on_hover_text(format!(
                        "Plan {plan_id} · last committed revision {} · current revision {}. A pause or cancel takes effect between atomic steps. Progress records committed edits, not confirmation from the audio callback.",
                        self.snapshot["last_revision"], self.snapshot["current_revision"]
                    ));
            });
        self.set_motion_mode(selected_mode);
        action
    }
}
