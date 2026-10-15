//! Host-only presentation; these values never enter a patch or DSP parameters.
use serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct CablePresentation {
    pub opacity: f32,
    pub tension: f32,
    pub plug_brightness: f32,
}

impl Default for CablePresentation {
    fn default() -> Self {
        Self {
            opacity: 1.0,
            tension: 0.5,
            plug_brightness: 1.0,
        }
    }
}

impl CablePresentation {
    pub(crate) fn sanitized(self) -> Self {
        let defaults = Self::default();
        let finite = |value: f32, default: f32| {
            if value.is_finite() {
                value.clamp(0.0, 1.0)
            } else {
                default
            }
        };
        Self {
            opacity: finite(self.opacity, defaults.opacity),
            tension: finite(self.tension, defaults.tension),
            plug_brightness: finite(self.plug_brightness, defaults.plug_brightness),
        }
    }

    pub(crate) fn from_settings(settings: &Value) -> Self {
        let defaults = Self::default();
        // Clamp before conversion so a finite JSON f64 cannot overflow f32.
        let read = |key: &str, default| {
            settings[key]
                .as_f64()
                .filter(|value| value.is_finite())
                .map_or(default, |value| value.clamp(0.0, 1.0) as f32)
        };
        Self {
            opacity: read("cable_opacity", defaults.opacity),
            tension: read("cable_tension", defaults.tension),
            plug_brightness: read("plug_light_brightness", defaults.plug_brightness),
        }
    }
}
