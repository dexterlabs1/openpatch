// Presentation descriptors come from the controls that were actually rendered.
// Native inline DragValue fields retain their ordinary egui focus traversal.
struct PrecisionControl {
    edit: ParameterEdit,
}

struct PrecisionNext {
    id: String,
    kind: String,
    parameter: String,
    revision: u64,
}

impl Rack {
    fn precision_collects(&self, node: &Node) -> bool {
        self.precision_collect_id.as_deref() == Some(node.id.as_str())
    }

    fn register_precision_control(&mut self, edit: ParameterEdit, anchor: Rect) {
        if let Some(next) = &self.precision_next {
            if edit.revision > next.revision {
                self.precision_next = None;
                if let Some(current) = &mut self.editing {
                    current.stale = true;
                }
                self.precision_error = Some(
                    "Patch changed after the value was saved. Reopen the control to continue."
                        .into(),
                );
            } else if edit.revision == next.revision
                && edit.id == next.id
                && edit.kind == next.kind
                && edit.parameter == next.parameter
            {
                self.editing = Some(edit.clone());
                self.precision_next = None;
                self.precision_error = None;
            }
        }
        if self.editing.as_ref().is_some_and(|current| {
            current.id == edit.id
                && current.kind == edit.kind
                && current.parameter == edit.parameter
        }) {
            self.precision_anchor = Some(anchor);
        }
        self.precision_controls.push(PrecisionControl { edit });
    }

    fn open_precision_editor(&mut self, edit: ParameterEdit, anchor: Rect) {
        self.editing = Some(edit);
        self.precision_anchor = Some(anchor);
        self.precision_next = None;
        self.precision_error = None;
        // Opening midway through drawing must not create a partial traversal.
        self.precision_controls.clear();
        self.precision_collect_id = None;
    }

    fn precision_affordance(&self, ui: &egui::Ui, response: &egui::Response) {
        if !ui.is_enabled() || !(response.hovered() || response.has_focus()) {
            return;
        }
        let painter = ui.painter();
        let start = crate::lighting::next(painter);
        painter.rect_stroke(
            response.rect.expand(1.5 * self.zoom),
            2.0 * self.zoom,
            Stroke::new(1.0_f32, Color32::from_rgb(201, 137, 54)),
            egui::StrokeKind::Outside,
        );
        self.lighting.preserve(painter, start);
    }

    /// `accepted` is captured while the successful edit still holds the session
    /// lock. It is never inferred from a later GUI sync or revision arithmetic.
    pub(crate) fn complete_parameter_navigation(
        &mut self,
        accepted: Result<u64, String>,
        forward: bool,
    ) {
        let revision = match accepted {
            Ok(revision) => revision,
            Err(error) => {
                self.precision_error = Some(error);
                return;
            }
        };
        let Some(edit) = &mut self.editing else {
            return;
        };
        let Some(index) = self.precision_controls.iter().position(|control| {
            control.edit.id == edit.id
                && control.edit.kind == edit.kind
                && control.edit.parameter == edit.parameter
        }) else {
            edit.stale = true;
            self.precision_error =
                Some("Value saved. Reopen a visible control to continue editing.".into());
            return;
        };
        let count = self.precision_controls.len();
        let index = if forward {
            (index + 1) % count
        } else {
            (index + count - 1) % count
        };
        let next = &self.precision_controls[index];
        self.precision_next = Some(PrecisionNext {
            id: next.edit.id.clone(),
            kind: next.edit.kind.clone(),
            parameter: next.edit.parameter.clone(),
            revision,
        });
        edit.revision = revision;
        edit.stale = false;
        self.precision_error = None;
    }

    fn parameter_editor(&mut self, ctx: &egui::Context, action: &mut Option<RackAction>) {
        if self
            .editing
            .as_ref()
            .is_none_or(|edit| edit.surface != ParameterEditSurface::Window)
        {
            return;
        }
        let waiting = self
            .precision_next
            .as_ref()
            .is_some_and(|next| next.revision > self.observed_revision);
        if !waiting && self.precision_next.take().is_some() {
            self.precision_error = Some(
                "Value saved. The next control is no longer visible; your draft remains here."
                    .into(),
            );
        }
        let anchor = self.precision_anchor;
        let viewport = ctx.content_rect().shrink(8.0);
        let position = anchor.map_or(viewport.center() - vec2(155.0, 140.0), |rect| {
            rect.right_top() + vec2(12.0, 0.0)
        });
        let navigation = self.precision_controls.len() > 1;
        let edit = self.editing.as_mut().unwrap();
        let mut outcome = ParameterEditOutcome::Keep;
        let mut open = true;
        egui::Window::new(format!("{} / {}", edit.id, edit.parameter))
            .id(egui::Id::new("rack-parameter-editor"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(290.0)
            .fixed_pos(position)
            .constrain_to(viewport)
            .show(ctx, |ui| {
                if waiting {
                    ui.label("Value saved. Waiting for the committed patch…");
                    ui.spinner();
                    return;
                }
                if anchor.is_none() {
                    ui.small("Control is outside the rack view. Your draft stays here.");
                }
                if let Some(error) = &self.precision_error {
                    ui.colored_label(Color32::LIGHT_RED, error);
                }
                if let Some(index) = self
                    .precision_controls
                    .iter()
                    .position(|control| control.edit.parameter == edit.parameter)
                {
                    ui.small(format!(
                        "Type-entry control {} of {}",
                        index + 1,
                        self.precision_controls.len()
                    ));
                }
                outcome = parameter_editor_contents_with_navigation(ui, edit, navigation);
            });
        match outcome {
            ParameterEditOutcome::Keep if open => {}
            ParameterEditOutcome::Apply(raw) => {
                let edit = self.editing.take().unwrap();
                *action = Some(parameter_edit_action(&edit, raw));
            }
            ParameterEditOutcome::ApplyAndNavigate { raw, forward } => {
                let edit = self.editing.as_ref().unwrap();
                *action = Some(RackAction::ParameterEditAndNavigate {
                    arguments: json!({"id":edit.id,"parameter":edit.parameter,"value":raw,"expected_revision":edit.revision}),
                    forward,
                });
            }
            ParameterEditOutcome::Keep | ParameterEditOutcome::Cancel => {
                self.editing = None;
                self.precision_next = None;
                self.precision_error = None;
            }
        }
    }
}
