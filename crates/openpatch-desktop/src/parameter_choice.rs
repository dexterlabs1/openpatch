// Compact categorical controls use explicit catalog values and labels. A
// stepped numeric range alone is never evidence that a control is a choice.
#[derive(Clone, Copy)]
struct ParameterChoices<'a> {
    values: &'a [Value],
    labels: &'a [Value],
}
impl<'a> ParameterChoices<'a> {
    fn from_spec(spec: &'a Value) -> Option<Self> {
        let values = spec["values"].as_array()?;
        let labels = spec["labels"].as_array()?;
        // Two or three states fit the shared compact parameter cell. Larger
        // lists belong to dedicated list controls, as on range/MIDI panels.
        if !(2..=3).contains(&values.len()) || values.len() != labels.len() {
            return None;
        }
        let min = spec["min"].as_f64()? as f32;
        let max = spec["max"].as_f64()? as f32;
        let step = spec["step"].as_f64()? as f32;
        if !min.is_finite() || !max.is_finite() || !step.is_finite() || step <= 0.0 {
            return None;
        }
        for (index, (value, label)) in values.iter().zip(labels).enumerate() {
            let value = value.as_f64()? as f32;
            if !value.is_finite()
                || value != min + index as f32 * step
                || label.as_str()?.trim().is_empty()
            {
                return None;
            }
        }
        (values.last()?.as_f64()? as f32 == max).then_some(Self { values, labels })
    }
    fn value(self, index: usize) -> f32 {
        self.values[index].as_f64().unwrap() as f32
    }
    fn label(self, index: usize) -> &'a str {
        self.labels[index].as_str().unwrap()
    }
    fn index(self, value: f32) -> usize {
        self.values
            .iter()
            .position(|candidate| candidate.as_f64().unwrap() as f32 == value)
            .unwrap_or(0)
    }
    fn segment(self, rect: Rect, index: usize) -> Rect {
        let width = rect.width() / self.values.len() as f32;
        Rect::from_min_size(
            rect.min + vec2(index as f32 * width, 0.0),
            vec2(width, rect.height()),
        )
    }
}

fn paint_parameter_choices(
    painter: &egui::Painter,
    rect: Rect,
    choices: ParameterChoices<'_>,
    value: f32,
    active: bool,
    accent: Color32,
    zoom: f32,
) {
    painter.rect_filled(rect, 2.0 * zoom, Color32::from_rgb(237, 233, 222));
    let selected = choices.index(value);
    for index in 0..choices.values.len() {
        let segment = choices.segment(rect, index);
        let color = if index == selected {
            painter.rect_filled(segment.shrink(1.0 * zoom), 1.0 * zoom, accent);
            Color32::from_rgb(249, 246, 235)
        } else {
            INK
        };
        if index > 0 {
            painter.line_segment(
                [segment.left_top(), segment.left_bottom()],
                Stroke::new(0.7 * zoom, Color32::from_rgb(143, 148, 135)),
            );
        }
        let font = crate::panel_skin::print_font(painter.ctx(), 10.0 * zoom);
        let mut galley = painter.layout_no_wrap(choices.label(index).into(), font, color);
        let available = (segment.width() - 6.0 * zoom).max(1.0);
        if galley.size().x > available {
            galley = painter.layout_no_wrap(
                choices.label(index).into(),
                crate::panel_skin::print_font(
                    painter.ctx(),
                    10.0 * zoom * available / galley.size().x,
                ),
                color,
            );
        }
        painter
            .with_clip_rect(painter.clip_rect().intersect(segment))
            .galley(segment.center() - galley.size() * 0.5, galley, color);
    }
    painter.rect_stroke(
        rect,
        2.0 * zoom,
        Stroke::new(
            if active { 1.3 } else { 0.8 } * zoom,
            if active {
                accent
            } else {
                Color32::from_rgb(143, 148, 135)
            },
        ),
        egui::StrokeKind::Inside,
    );
}

impl Rack {
    fn parameter_choice_input(
        &mut self,
        ui: &egui::Ui,
        response: &egui::Response,
        choices: ParameterChoices<'_>,
        value: f32,
        enabled: bool,
    ) -> Option<f32> {
        if !enabled
            || !ui.is_enabled()
            || !ui.input(|i| i.focused)
            || ui.input(|i| i.key_pressed(egui::Key::Escape))
        {
            return None;
        }
        if response.is_pointer_button_down_on()
            && ui.input(|i| i.pointer.button_pressed(PointerButton::Primary))
        {
            response.request_focus();
        }
        if response.clicked_by(PointerButton::Primary) {
            let pointer = response.interact_pointer_pos()?;
            let index = choices
                .values
                .iter()
                .enumerate()
                .find(|(index, _)| choices.segment(response.rect, *index).contains(pointer))?
                .0;
            return Some(choices.value(index));
        }
        if !response.has_focus() {
            return None;
        }
        ui.ctx().memory_mut(|memory| {
            memory.set_focus_lock_filter(
                response.id,
                egui::EventFilter {
                    horizontal_arrows: true,
                    vertical_arrows: true,
                    ..Default::default()
                },
            );
        });
        let current = choices.index(value);
        ui.input_mut(|input| {
            let modifiers = input.modifiers;
            // Radio-like choices are idempotent. Activation does not inspect
            // a global pointer position or cycle to an unrelated state.
            input.consume_key(modifiers, egui::Key::Space);
            input.consume_key(modifiers, egui::Key::Enter);
            let next = if input.consume_key(modifiers, egui::Key::Home) {
                0
            } else if input.consume_key(modifiers, egui::Key::End) {
                choices.values.len() - 1
            } else if input.consume_key(modifiers, egui::Key::ArrowLeft)
                || input.consume_key(modifiers, egui::Key::ArrowDown)
            {
                current.saturating_sub(1)
            } else if input.consume_key(modifiers, egui::Key::ArrowRight)
                || input.consume_key(modifiers, egui::Key::ArrowUp)
            {
                (current + 1).min(choices.values.len() - 1)
            } else {
                return None;
            };
            Some(choices.value(next))
        })
    }
}
