// Deterministic graph-aware layout proposal. Application history owns applying
// the proposal; this planner never changes the graph or current rack positions.
impl Rack {
    pub(crate) fn organize(
        &self,
        patch: &Patch,
        _catalog: &Value,
        rows: Option<usize>,
    ) -> Result<ModuleMove, String> {
        if self.positions_locked() {
            return Err("Unlock module positions before organizing the rack.".into());
        }
        if !self.navigation_idle() {
            return Err("Finish the current rack gesture before organizing.".into());
        }
        if rows.is_some_and(|rows| !(1..=8).contains(&rows)) {
            return Err("Choose Auto or between 1 and 8 rack rows.".into());
        }
        let mut widths: BTreeMap<String, f32> = patch
            .nodes
            .iter()
            .map(|node| (node.id.clone(), module_width(node)))
            .collect();
        widths.extend(
            self.unresolved_panels
                .iter()
                .map(|panel| (panel.id.clone(), UNRESOLVED_WIDTH)),
        );
        if widths
            .values()
            .any(|width| !width.is_finite() || *width <= 0.0)
        {
            return Err("A module has an invalid panel width.".into());
        }
        let mut before = BTreeMap::new();
        for id in widths.keys() {
            let point = self
                .positions
                .get(id)
                .ok_or_else(|| format!("Module {id} has no rack position yet."))?;
            if !point.is_finite() {
                return Err(format!("Module {id} has an invalid rack position."));
            }
            before.insert(id.clone(), *point);
        }
        let mut edges = BTreeSet::new();
        for cable in patch.cables.iter().chain(&self.source_routes) {
            if cable.from != cable.to
                && widths.contains_key(&cable.from)
                && widths.contains_key(&cable.to)
            {
                edges.insert((cable.from.clone(), cable.to.clone()));
            }
        }
        let order = organize_signal_order(&widths, &edges);
        let count = order.len();
        let row_count = rows
            .unwrap_or_else(|| {
                let viewport = self
                    .viewport
                    .map(|rect| rect.size())
                    .unwrap_or(vec2(1440.0, 800.0));
                let zoom = if self.zoom.is_finite() && self.zoom > 0.0 {
                    self.zoom
                } else {
                    1.0
                };
                let viewport = viewport / zoom;
                (1..=count.clamp(1, 8))
                    .min_by(|a, b| {
                        let score = |rows| {
                            let layout = organize_pack_rows(&order, &widths, rows);
                            let width = layout
                                .values()
                                .map(|row: &Vec<String>| {
                                    row.iter()
                                        .map(|id| organize_grid_width(widths[id]))
                                        .sum::<f32>()
                                })
                                .fold(0.0, f32::max);
                            (width / viewport.x.max(1.0))
                                .max(rows as f32 * ROW / viewport.y.max(1.0))
                        };
                        score(*a).total_cmp(&score(*b)).then(a.cmp(b))
                    })
                    .unwrap_or(1)
            })
            .min(count.max(1));
        let packed = organize_pack_rows(&order, &widths, row_count);
        let mut after = BTreeMap::new();
        for (row, ids) in packed {
            let mut x = 0.0;
            // The cable leaving one row meets the start of the next row nearby.
            let ordered: Vec<_> = if row % 2 == 0 {
                ids
            } else {
                ids.into_iter().rev().collect()
            };
            for id in ordered {
                after.insert(id.clone(), pos2(x, row as f32 * ROW));
                x += organize_grid_width(widths[&id]);
            }
        }
        if after.iter().any(|(id, point)| {
            !point.is_finite()
                || point.x + widths[id] > 100_000.0
                || point.y + PANEL_HEIGHT > 100_000.0
        }) {
            return Err("The organized rack exceeds the supported layout area.".into());
        }
        Ok(ModuleMove {
            revision: self.observed_revision,
            before,
            after,
        })
    }
}

fn organize_grid_width(width: f32) -> f32 {
    (width / HP).ceil() * HP
}

fn organize_signal_order(
    widths: &BTreeMap<String, f32>,
    edges: &BTreeSet<(String, String)>,
) -> Vec<String> {
    let mut neighbors: BTreeMap<String, BTreeSet<String>> = widths
        .keys()
        .map(|id| (id.clone(), BTreeSet::new()))
        .collect();
    let mut parents = neighbors.clone();
    for (from, to) in edges {
        neighbors.get_mut(from).unwrap().insert(to.clone());
        neighbors.get_mut(to).unwrap().insert(from.clone());
        parents.get_mut(to).unwrap().insert(from.clone());
    }
    let mut remaining: BTreeSet<_> = widths.keys().cloned().collect();
    let mut result = Vec::with_capacity(remaining.len());
    while let Some(seed) = remaining.first().cloned() {
        // Keep disconnected voices together rather than interleaving their stages.
        let mut component = BTreeSet::new();
        let mut queue = vec![seed];
        while let Some(id) = queue.pop() {
            if component.insert(id.clone()) {
                queue.extend(
                    neighbors[&id]
                        .iter()
                        .filter(|id| !component.contains(*id))
                        .cloned(),
                );
            }
        }
        let mut placed = BTreeMap::<String, usize>::new();
        while !component.is_empty() {
            // Prefer ready sources, then nearby already placed parents. For a
            // feedback loop, break at the fewest remaining inputs with ID ties.
            // This local cable-length/crossing heuristic is not a global optimum.
            let next = component
                .iter()
                .min_by_key(|id| {
                    let incoming = parents[*id].intersection(&component).count();
                    let parent_slots: Vec<_> = parents[*id]
                        .iter()
                        .filter_map(|id| placed.get(id))
                        .collect();
                    let distance = if parent_slots.is_empty() {
                        usize::MAX
                    } else {
                        placed.len()
                            - parent_slots.iter().map(|slot| **slot).sum::<usize>()
                                / parent_slots.len()
                    };
                    (incoming, distance, (*id).clone())
                })
                .unwrap()
                .clone();
            component.remove(&next);
            remaining.remove(&next);
            placed.insert(next.clone(), placed.len());
            result.push(next);
        }
    }
    result
}

fn organize_pack_rows(
    order: &[String],
    widths: &BTreeMap<String, f32>,
    rows: usize,
) -> BTreeMap<usize, Vec<String>> {
    let mut result = BTreeMap::new();
    let mut cursor = 0;
    let mut remaining_width: f32 = order.iter().map(|id| organize_grid_width(widths[id])).sum();
    for row in 0..rows.min(order.len()) {
        let remaining_rows = rows - row;
        let target = remaining_width / remaining_rows as f32;
        let mut row_width = 0.0;
        let start = cursor;
        while cursor < order.len() - (remaining_rows - 1) {
            let width = organize_grid_width(widths[&order[cursor]]);
            if cursor > start && (row_width + width - target).abs() > (row_width - target).abs() {
                break;
            }
            row_width += width;
            cursor += 1;
        }
        result.insert(row, order[start..cursor].to_vec());
        remaining_width -= row_width;
    }
    result
}

#[cfg(test)]
#[path = "rack_organize_tests.rs"]
mod rack_organize_tests;
