// Recoverable source-only panels. They are never injected into a Core Patch.
#[derive(Clone)]
struct UnresolvedPanel {
    id: String,
    plugin: String,
    model: String,
    version: String,
    ports: Vec<openpatch::project_document::ObservedPort>,
}
const UNRESOLVED_WIDTH: f32 = 180.0;
impl Rack {
    pub(crate) fn reconcile_document(
        &mut self,
        patch: &Patch,
        document: &openpatch::project_document::ProjectDocument,
        catalog: &Value,
    ) {
        use openpatch::project_document::DocumentNode;
        self.unresolved_panels = document
            .graph
            .nodes
            .iter()
            .filter_map(|node| {
                let DocumentNode::Unresolved { id, source, ports } = node else {
                    return None;
                };
                let record = document.source_module(source).ok()?;
                Some(UnresolvedPanel {
                    id: id.clone(),
                    plugin: record["plugin"].as_str().unwrap_or("Unknown plugin").into(),
                    model: record["model"].as_str().unwrap_or("Unknown module").into(),
                    version: record["version"].as_str().unwrap_or("unspecified").into(),
                    ports: ports.clone(),
                })
            })
            .collect();
        let unresolved: BTreeSet<_> = self
            .unresolved_panels
            .iter()
            .map(|panel| panel.id.as_str())
            .collect();
        self.source_routes = document
            .graph
            .cables
            .iter()
            .filter(|cable| {
                unresolved.contains(cable.route.from.as_str())
                    || unresolved.contains(cable.route.to.as_str())
            })
            .map(|cable| cable.route.clone())
            .collect();
        for (id, position) in &document.layout.positions {
            self.positions
                .entry(id.clone())
                .or_insert_with(|| pos2(position[0], position[1]));
        }
        self.reconcile(patch, catalog);
        let mut x = self
            .positions
            .values()
            .map(|p| p.x + UNRESOLVED_WIDTH)
            .fold(0., f32::max);
        for panel in &self.unresolved_panels {
            if !self.positions.contains_key(&panel.id) {
                self.positions.insert(panel.id.clone(), pos2(x, 0.));
                x += UNRESOLVED_WIDTH;
                self.layout_revision = self.layout_revision.wrapping_add(1);
            }
        }
    }
    /// Source coordinates use Rack panel widths. Fit the imported row to the
    /// native panels once, retaining each row and its source left-to-right order.
    /// Saved project layouts deliberately never call this adapter.
    pub(crate) fn reflow_imported_source_layout(
        &mut self,
        document: &openpatch::project_document::ProjectDocument,
    ) {
        use openpatch::project_document::DocumentNode;
        const GAP: f32 = 15.0;
        let mut panels: Vec<_> = document
            .graph
            .nodes
            .iter()
            .enumerate()
            .filter_map(|(order, node)| {
                let position = *self.positions.get(node.id())?;
                let width = match node {
                    DocumentNode::Native { node, .. } => module_width(node),
                    DocumentNode::Unresolved { .. } => UNRESOLVED_WIDTH,
                };
                Some((node.id().to_owned(), position, width, order))
            })
            .collect();
        panels.sort_by(|a, b| {
            a.1.y
                .total_cmp(&b.1.y)
                .then_with(|| a.1.x.total_cmp(&b.1.x))
                .then_with(|| a.3.cmp(&b.3))
        });
        let mut row_y = None;
        let mut next_x = 0.0_f32;
        let mut changed = false;
        for (id, mut position, width, _) in panels {
            if row_y == Some(position.y) {
                if position.x < next_x {
                    position.x = next_x;
                    self.positions.insert(id, position);
                    changed = true;
                }
            } else {
                row_y = Some(position.y);
            }
            next_x = position.x + width + GAP;
        }
        if changed {
            self.layout_revision = self.layout_revision.wrapping_add(1);
        }
    }

    pub(crate) fn take_dependency_focus(&mut self) -> Option<String> {
        self.dependency_focus.take()
    }

    fn paint_unresolved_panels(
        &mut self,
        ui: &mut egui::Ui,
        painter: &egui::Painter,
        canvas: Rect,
        inputs: &mut BTreeMap<(String, String), Pos2>,
        outputs: &mut BTreeMap<(String, String), Pos2>,
    ) -> bool {
        use openpatch::project_document::PortDirection;
        let mut pointer_on_panel = false;
        for panel in &self.unresolved_panels {
            let Some(position) = self.positions.get(&panel.id) else {
                continue;
            };
            let z = self.zoom;
            let rect = Rect::from_min_size(
                canvas.min + self.pan + position.to_vec2() * z,
                vec2(UNRESOLVED_WIDTH, PANEL_HEIGHT) * z,
            );
            // Populate all retained endpoint coordinates even when the panel is off-screen.
            for direction in [PortDirection::Input, PortDirection::Output] {
                let ports: Vec<_> = panel
                    .ports
                    .iter()
                    .filter(|port| port.direction == direction)
                    .collect();
                let spacing = (210. / ports.len().max(1) as f32).min(32.);
                for (row, port) in ports.iter().enumerate() {
                    let center = rect.min
                        + vec2(
                            if direction == PortDirection::Input {
                                27.
                            } else {
                                UNRESOLVED_WIDTH - 27.
                            },
                            145. + row as f32 * spacing,
                        ) * z;
                    let map = if direction == PortDirection::Input {
                        &mut *inputs
                    } else {
                        &mut *outputs
                    };
                    map.insert((panel.id.clone(), port.key()), center);
                }
            }
            if !painter.clip_rect().intersects(rect) {
                continue;
            }
            painter.rect_filled(rect, 2. * z, Color32::from_rgb(214, 211, 196));
            painter.rect_stroke(
                rect,
                2. * z,
                Stroke::new(z, Color32::from_rgb(120, 116, 101)),
                egui::StrokeKind::Inside,
            );
            let ink = Color32::from_rgb(52, 56, 49);
            let amber = Color32::from_rgb(123, 84, 37);
            let clipped =
                painter.with_clip_rect(rect.shrink(7. * z).intersect(painter.clip_rect()));
            clipped.text(
                rect.min + vec2(12., 16.) * z,
                Align2::LEFT_TOP,
                &panel.model,
                FontId::proportional(18. * z),
                ink,
            );
            clipped.text(
                rect.min + vec2(12., 43.) * z,
                Align2::LEFT_TOP,
                &panel.plugin,
                FontId::proportional(11. * z),
                ink,
            );
            clipped.line_segment(
                [
                    rect.min + vec2(12., 66.) * z,
                    rect.min + vec2(168., 66.) * z,
                ],
                Stroke::new(z, ink),
            );
            clipped.text(
                rect.min + vec2(12., 81.) * z,
                Align2::LEFT_TOP,
                "UNAVAILABLE",
                FontId::proportional(12. * z),
                amber,
            );
            clipped.text(
                rect.min + vec2(12., 103.) * z,
                Align2::LEFT_TOP,
                "Source + routing retained",
                FontId::proportional(10. * z),
                ink,
            );
            for port in &panel.ports {
                let map = if port.direction == PortDirection::Input {
                    &*inputs
                } else {
                    &*outputs
                };
                let center = map[&(panel.id.clone(), port.key())];
                clipped.circle_filled(center, 8. * z, Color32::from_rgb(61, 65, 58));
                clipped.circle_stroke(
                    center,
                    8. * z,
                    Stroke::new(2. * z, Color32::from_rgb(164, 153, 111)),
                );
                clipped.text(
                    center + vec2(0., 11.) * z,
                    Align2::CENTER_TOP,
                    format!(
                        "{} #{}",
                        if port.direction == PortDirection::Input {
                            "IN"
                        } else {
                            "OUT"
                        },
                        port.index
                    ),
                    FontId::proportional(8. * z),
                    ink,
                );
            }
            clipped.text(
                rect.min + vec2(12., 360.) * z,
                Align2::LEFT_TOP,
                "Click to inspect / repair",
                FontId::proportional(10. * z),
                ink,
            );
            for x in [8., 172.] {
                for y in [8., PANEL_HEIGHT - 8.] {
                    let center = rect.min + vec2(x, y) * z;
                    crate::panel_skin::mounting_screw(painter, center, z);
                }
            }
            let response = ui.interact(
                rect,
                ui.id().with(("unresolved-module", &panel.id)),
                Sense::click(),
            );
            pointer_on_panel |= response.hovered();
            if response.clicked() {
                self.dependency_focus = Some(panel.id.clone());
            }
            response.on_hover_text(format!("{} / {} ({})\nUnavailable dependency. Its source data and observed port indices are retained. Click to review; this module does not run.",panel.plugin,panel.model,panel.version));
        }
        // Dashed source routes have no signal indication and cannot imply a
        // running module. Their original identities/colors remain in the document.
        for cable in &self.source_routes {
            let Some(start) = outputs.get(&(cable.from.clone(), cable.output.clone())) else {
                continue;
            };
            let Some(end) = inputs.get(&(cable.to.clone(), cable.input.clone())) else {
                continue;
            };
            let color = cable
                .color
                .map(|c| Color32::from_rgba_unmultiplied(c[0], c[1], c[2], 150))
                .unwrap_or(Color32::from_rgba_unmultiplied(152, 132, 86, 150));
            let sag = (end.x - start.x).abs().mul_add(0.1, 35. * self.zoom);
            let point = |t: f32| {
                let u = 1. - t;
                start.to_vec2() * (u * u * u)
                    + (start.to_vec2() + vec2(0., sag)) * (3. * u * u * t)
                    + (end.to_vec2() + vec2(0., sag)) * (3. * u * t * t)
                    + end.to_vec2() * (t * t * t)
            };
            for segment in (0..40).step_by(2) {
                painter.line_segment(
                    [
                        point(segment as f32 / 40.).to_pos2(),
                        point((segment + 1) as f32 / 40.).to_pos2(),
                    ],
                    Stroke::new((1.6 * self.zoom).max(0.4), color),
                );
            }
        }
        pointer_on_panel
    }
}
