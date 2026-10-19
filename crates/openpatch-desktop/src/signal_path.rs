// Routing inspection is host-only. It follows directed module connections,
// including CV dependencies; it does not infer a module's internal DSP paths.
#[derive(Default)]
struct SignalPath {
    enabled: bool,
    selected: Option<Cable>,
    graph: Vec<Cable>,
    highlighted: Vec<bool>,
    modules: BTreeSet<String>,
    highlighted_ids: BTreeSet<String>,
    highlighted_legacy: Vec<Cable>,
}

impl SignalPath {
    fn refresh(&mut self, patch: &Patch, source: &[Cable]) {
        if !self.enabled {
            return;
        }
        let routes = || patch.cables.iter().chain(source);
        let unchanged = self.graph.len() == patch.cables.len() + source.len()
            && self
                .graph
                .iter()
                .zip(routes())
                .all(|(a, b)| same_cable(a, b));
        if unchanged {
            return;
        }
        self.graph = routes().cloned().collect();
        if let Some(selected) = self.selected.as_ref() {
            self.selected = self
                .graph
                .iter()
                .find(|cable| match (&selected.id, &cable.id) {
                    (Some(a), Some(b)) => a == b,
                    _ => same_cable(selected, cable),
                })
                .cloned();
        }
        self.trace();
    }

    fn trace(&mut self) {
        self.highlighted = vec![false; self.graph.len()];
        self.modules.clear();
        self.highlighted_ids.clear();
        self.highlighted_legacy.clear();
        let Some(selected) = self.selected.as_ref() else {
            return;
        };
        let Some(index) = self
            .graph
            .iter()
            .position(|cable| same_cable(cable, selected))
        else {
            self.selected = None;
            return;
        };
        self.highlighted[index] = true;
        // Two independent traversals avoid flooding unrelated side chains:
        // predecessors of the source, and successors of the destination.
        for upstream in [true, false] {
            let start = if upstream {
                &selected.from
            } else {
                &selected.to
            };
            let mut seen = BTreeSet::from([start.as_str()]);
            let mut pending = vec![start.as_str()];
            while let Some(node) = pending.pop() {
                self.modules.insert(node.into());
                for (i, cable) in self.graph.iter().enumerate() {
                    let (at, next) = if upstream {
                        (&cable.to, &cable.from)
                    } else {
                        (&cable.from, &cable.to)
                    };
                    if at == node {
                        self.highlighted[i] = true;
                        if seen.insert(next.as_str()) {
                            pending.push(next);
                        }
                    }
                }
            }
        }
        for (cable, highlighted) in self.graph.iter().zip(&self.highlighted) {
            if *highlighted {
                match &cable.id {
                    Some(id) => {
                        self.highlighted_ids.insert(id.clone());
                    }
                    None => self.highlighted_legacy.push(cable.clone()),
                }
            }
        }
    }

    fn contains(&self, cable: &Cable) -> bool {
        self.enabled
            && self.selected.is_some()
            && match &cable.id {
                Some(id) => self.highlighted_ids.contains(id),
                None => self
                    .highlighted_legacy
                    .iter()
                    .any(|candidate| same_cable(candidate, cable)),
            }
    }
    fn dim(&self, cable: &Cable) -> f32 {
        if self.enabled && self.selected.is_some() && !self.contains(cable) {
            0.14
        } else {
            1.
        }
    }
}

impl Rack {
    pub(crate) fn signal_path_mode(&self) -> bool {
        self.signal_path.enabled
    }

    pub(crate) fn set_signal_path_mode(&mut self, enabled: bool) {
        if enabled && !self.navigation_idle() {
            return;
        }
        self.signal_path.enabled = enabled;
        self.signal_path.selected = None;
        self.signal_path.highlighted.clear();
        self.signal_path.modules.clear();
        self.signal_path.highlighted_ids.clear();
        self.signal_path.highlighted_legacy.clear();
        self.cancel_module_drag();
    }

    fn signal_path_geometry(
        &self,
        outputs: &BTreeMap<(String, String), Pos2>,
        inputs: &BTreeMap<(String, String), Pos2>,
        now: std::time::Instant,
    ) -> Vec<(usize, [Pos2; 4])> {
        if !self.signal_path.enabled {
            return vec![];
        }
        let native_count = self.signal_path.graph.len() - self.source_routes.len();
        let mut counts = BTreeMap::new();
        let mut ordinals = BTreeMap::new();
        for cable in self.signal_path.graph.iter().take(native_count) {
            *counts.entry((&cable.to, &cable.input)).or_insert(0usize) += 1;
        }
        self.signal_path
            .graph
            .iter()
            .enumerate()
            .filter_map(|(index, cable)| {
                let from = *outputs.get(&(cable.from.clone(), cable.output.clone()))?;
                let to = *inputs.get(&(cable.to.clone(), cable.input.clone()))?;
                let points = if index >= native_count {
                    source_route_curve(from, to, self.zoom)
                } else {
                    let count = counts[&(&cable.to, &cable.input)];
                    let ordinal = ordinals.entry((&cable.to, &cable.input)).or_insert(0usize);
                    let fan = if count > 1 {
                        (*ordinal as f32 / (count - 1) as f32 - 0.5) * 40.
                    } else {
                        0.
                    };
                    *ordinal += 1;
                    let movement = self
                        .construction_animation
                        .cables
                        .movement(cable, now)
                        .and_then(|(old, old_fan, elapsed)| {
                            Some(cable_move_geometry(
                                *outputs.get(&(old.from.clone(), old.output.clone()))?,
                                *inputs.get(&(old.to.clone(), old.input.clone()))?,
                                from,
                                to,
                                old_fan,
                                fan,
                                self.zoom,
                                elapsed,
                                old.from != cable.from || old.output != cable.output,
                                old.to != cable.to || old.input != cable.input,
                            ))
                        });
                    let (from, to, fan) =
                        movement.map_or((from, to, fan), |m| (m.from, m.to, m.fan));
                    cable_curve_points(from, to, self.zoom, fan, self.cable_presentation.tension)
                };
                Some((index, points))
            })
            .collect()
    }

    fn pick_signal_path(
        &mut self,
        ui: &mut egui::Ui,
        canvas: Rect,
        geometry: &[(usize, [Pos2; 4])],
    ) {
        if !self.signal_path.enabled
            || !ui.is_enabled()
            || !self.shortcuts_enabled
            || egui::Popup::is_any_open(ui.ctx())
        {
            return;
        }
        let pointer = ui.input(|i| i.pointer.interact_pos().or(i.pointer.hover_pos()));
        let hovered = pointer.filter(|p| canvas.contains(*p)).and_then(|pointer| {
            // Last painted route wins at crossings, matching the visible stack.
            geometry
                .iter()
                .rev()
                .find(|(_, curve)| cable_curve_near(*curve, pointer, 6.))
                .map(|(index, _)| *index)
        });
        if hovered.is_some() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if ui.input(|i| i.pointer.primary_clicked())
            && pointer.is_some_and(|p| canvas.contains(p))
            && pointer.is_some_and(|p| ui.ctx().layer_id_at(p) == Some(ui.layer_id()))
        {
            self.signal_path.selected = hovered.map(|index| self.signal_path.graph[index].clone());
            self.signal_path.trace();
        }
    }

    fn paint_signal_path(
        &self,
        ui: &egui::Ui,
        painter: &egui::Painter,
        canvas: Rect,
        geometry: &[(usize, [Pos2; 4])],
        patch: &Patch,
    ) {
        if !self.signal_path.enabled {
            return;
        }
        for node in self.signal_path.modules.iter() {
            if let Some(position) = self.positions.get(node) {
                let width = patch
                    .nodes
                    .iter()
                    .find(|n| &n.id == node)
                    .map(module_width)
                    .unwrap_or(UNRESOLVED_WIDTH);
                let rect = Rect::from_min_size(
                    canvas.min + self.pan + position.to_vec2() * self.zoom,
                    vec2(width, PANEL_HEIGHT) * self.zoom,
                );
                painter.rect_stroke(
                    rect.shrink(self.zoom),
                    1.,
                    Stroke::new(1.5_f32, Color32::from_rgb(209, 175, 105)),
                    egui::StrokeKind::Inside,
                );
            }
        }
        for (index, curve) in geometry {
            if !self
                .signal_path
                .highlighted
                .get(*index)
                .copied()
                .unwrap_or(false)
            {
                continue;
            }
            let cable = &self.signal_path.graph[*index];
            let selected = self
                .signal_path
                .selected
                .as_ref()
                .is_some_and(|s| same_cable(s, cable));
            let ink = if selected {
                Color32::from_rgb(255, 224, 160)
            } else {
                Color32::from_rgb(218, 235, 225)
            };
            // This inspection overlay stays legible with room lights off.
            painter.add(egui::epaint::CubicBezierShape::from_points_stroke(
                *curve,
                false,
                Color32::TRANSPARENT,
                Stroke::new(
                    if selected { 1.4_f32 } else { 1.0_f32 },
                    if selected {
                        ink
                    } else {
                        cable_instance_color(cable).gamma_multiply(0.72)
                    },
                ),
            ));
            for t in [0.24, 0.5, 0.76] {
                let tip = cable_curve_prefix(*curve, t)[3];
                let before = cable_curve_prefix(*curve, t - 0.015)[3];
                let direction = tip - before;
                if direction.length_sq() < 0.0001 {
                    continue;
                }
                let direction = direction.normalized();
                let normal = vec2(-direction.y, direction.x);
                let size = (5. * self.zoom).clamp(4., 7.);
                let points = vec![
                    tip - direction * size + normal * size * 0.6,
                    tip,
                    tip - direction * size - normal * size * 0.6,
                ];
                painter.add(egui::Shape::line(
                    points.clone(),
                    Stroke::new(4.0_f32, Color32::from_rgb(20, 26, 24)),
                ));
                painter.add(egui::Shape::line(points, Stroke::new(1.6_f32, ink)));
            }
        }
        let label = match &self.signal_path.selected {
            Some(c) => format!("SIGNAL PATH   {}.{} → {}.{}   ·   {} connections   ·   Audio + CV routing   ·   Esc to exit",
                c.from,c.output,c.to,c.input,self.signal_path.highlighted.iter().filter(|v| **v).count()),
            None => "SIGNAL PATH   Click a cable to trace its routing   ·   Esc to exit".into(),
        };
        let font = FontId::proportional(12.);
        let galley = painter.layout(
            label,
            font,
            Color32::from_rgb(236, 225, 200),
            (canvas.width() - 40.).max(50.),
        );
        let badge = Rect::from_min_size(
            canvas.left_bottom() + vec2(12., -galley.size().y - 26.),
            galley.size() + vec2(20., 14.),
        );
        painter.rect_filled(badge, 6., Color32::from_rgba_unmultiplied(24, 30, 28, 242));
        painter.rect_stroke(
            badge,
            6.,
            Stroke::new(1.0_f32, Color32::from_rgb(102, 101, 77)),
            egui::StrokeKind::Inside,
        );
        painter.galley(
            badge.min + vec2(10., 7.),
            galley,
            Color32::from_rgb(236, 225, 200),
        );
        let _ = ui;
    }
}

fn source_route_curve(from: Pos2, to: Pos2, zoom: f32) -> [Pos2; 4] {
    let sag = (to.x - from.x).abs().mul_add(0.1, 35. * zoom);
    [from, from + vec2(0., sag), to + vec2(0., sag), to]
}
