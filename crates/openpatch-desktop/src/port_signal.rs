// Actual endpoint emission is independent of cable color and edit decorations.
impl Rack {
    pub(crate) fn set_port_lights(&mut self, frame: Option<Arc<crate::audio::PortLightFrame>>) {
        self.port_lights = frame.filter(|frame| {
            frame.token != 0
                && frame.control_epoch & 1 == 1
                && frame.sequence != 0
                && matches!(frame.backend, 1 | 2)
                && frame.last_observed_frame < frame.frame
        });
    }

    fn port_signal_state(
        &self,
        id: &str,
        direction: openpatch_core::PortDirection,
        port: &str,
    ) -> Option<openpatch_core::PortLightState> {
        let frame = self.port_lights.as_ref()?;
        if frame.revision != self.observed_revision {
            return None;
        }
        frame
            .ports
            .get(&crate::audio::PortLightKey::new(id, direction, port))
            .copied()
    }

    fn paint_port_signal(
        &self,
        painter: &egui::Painter,
        point: Pos2,
        id: &str,
        direction: openpatch_core::PortDirection,
        port: &str,
    ) {
        let brightness = self.cable_presentation.plug_brightness;
        if brightness <= 0.0 {
            return;
        }
        let Some(color) = self
            .port_signal_state(id, direction, port)
            .and_then(port_signal_color)
        else {
            return;
        };
        let color = color.gamma_multiply(brightness);
        self.lighting.port(painter, point, self.zoom, color);
    }

    fn port_signal_description(
        &self,
        id: &str,
        direction: openpatch_core::PortDirection,
        port: &str,
    ) -> String {
        if self.cable_presentation.plug_brightness <= 0.0 {
            return "\nPlug signal lights are off.".into();
        }
        let Some(state) = self.port_signal_state(id, direction, port) else {
            return "\nSignal lights: no current measurement.".into();
        };
        if !state.connected {
            return "\nSignal lights: unpatched.".into();
        }
        if state.invalid {
            return "\nSignal lights: invalid signal; display suppressed.".into();
        }
        format!(
            "\nMeasured: {} channel{}. Plug light: green + / red − mono, blue poly.",
            state.channels,
            if state.channels == 1 { "" } else { "s" }
        )
    }
}

fn port_signal_color(state: openpatch_core::PortLightState) -> Option<Color32> {
    if !state.connected
        || state.invalid
        || state.channels == 0
        || state.raw_rgb.iter().any(|value| !value.is_finite())
    {
        return None;
    }
    // Screen-combine our red/green/blue palette without rectifying stored
    // detector history. This deliberately does not infer signal from voice count.
    let palette = [[1.0, 0.22, 0.18], [0.23, 1.0, 0.40], [0.26, 0.49, 1.0]];
    let mut inverse = [1.0; 3];
    for (rgb, level) in palette.iter().zip(state.raw_rgb) {
        // A perceptual display curve reveals small real voltages. Apply it
        // before the user's linear brightness setting, keeping zero dark and
        // retaining the measured positive/negative/poly color assignment.
        let visible = level.clamp(0.0, 1.0).sqrt();
        for (result, component) in inverse.iter_mut().zip(rgb) {
            *result *= 1.0 - component * visible;
        }
    }
    let rgb = inverse.map(|value| ((1.0 - value) * 255.0).round() as u8);
    (rgb != [0; 3]).then(|| Color32::from_rgb_additive(rgb[0], rgb[1], rgb[2]))
}

#[cfg(test)]
mod port_signal_tests {
    use super::*;
    use openpatch_core::{PortDirection, PortLightState};

    #[test]
    fn port_emission_uses_signed_rgb_and_actual_activity_not_poly_capacity() {
        let mut state = PortLightState {
            connected: true,
            channels: 1,
            raw_rgb: [-0.5, 0.5, 0.0],
            invalid: false,
        };
        let positive = port_signal_color(state).unwrap();
        assert!(positive.g() > positive.r());
        state.raw_rgb = [0.5, -0.5, 0.0];
        let negative = port_signal_color(state).unwrap();
        assert!(negative.r() > negative.g());
        state.channels = 16;
        state.raw_rgb = [0.0; 3];
        assert!(port_signal_color(state).is_none());
        state.raw_rgb = [0.0, 0.0, 0.5];
        let poly = port_signal_color(state).unwrap();
        assert!(poly.b() > poly.g());
        state.raw_rgb[2] = 5.0;
        let saturated = port_signal_color(state).unwrap();
        state.raw_rgb[2] = 1.0;
        assert_eq!(port_signal_color(state), Some(saturated));
        for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            state.raw_rgb[2] = value;
            assert!(port_signal_color(state).is_none());
        }
    }

    #[test]
    fn port_measurement_never_survives_revision_or_offline_clear() {
        let patch: Patch = serde_json::from_value(json!({"version":2,"nodes":[
            {"id":"source","kind":"constant","params":{"value":5}},
            {"id":"sink","kind":"gain"}],"cables":[{"from":"source","to":"sink","input":"signal"}],"output":"sink"})).unwrap();
        let mut engine = patch.compile().unwrap();
        engine.set_port_lights_enabled(true);
        engine.process_audio_block(&mut [[0.0; 2]; 64]);
        let frame = crate::audio::offline_frame(&patch, &engine, 3).unwrap();
        let mut rack = Rack {
            observed_revision: 3,
            ..Rack::default()
        };
        rack.set_port_lights(Some(frame));
        assert!(rack
            .port_signal_state("source", PortDirection::Output, "out")
            .is_some());
        assert!(rack
            .port_signal_state("source", PortDirection::Input, "out")
            .is_none());
        assert!(rack
            .port_signal_state("missing", PortDirection::Output, "out")
            .is_none());
        rack.observed_revision = 4;
        assert!(rack
            .port_signal_state("source", PortDirection::Output, "out")
            .is_none());
        rack.observed_revision = 3;
        rack.set_port_lights(None);
        assert!(rack
            .port_signal_state("source", PortDirection::Output, "out")
            .is_none());
    }
}
