//! Host-only presentation; these values never enter a patch or DSP parameters.
use serde_json::Value;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum CableFinish {
    #[default]
    Classic,
    Woven,
    Tracer,
    Translucent,
}

impl CableFinish {
    pub(crate) const ALL: [Self; 4] = [Self::Classic, Self::Woven, Self::Tracer, Self::Translucent];

    pub(crate) fn key(self) -> &'static str {
        match self {
            Self::Classic => "classic",
            Self::Woven => "woven",
            Self::Tracer => "tracer",
            Self::Translucent => "translucent",
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Classic => "Classic",
            Self::Woven => "Woven",
            Self::Tracer => "Tracer",
            Self::Translucent => "Translucent",
        }
    }

    pub(crate) fn from_key(key: &str) -> Option<Self> {
        match key {
            "classic" => Some(Self::Classic),
            "woven" => Some(Self::Woven),
            "tracer" => Some(Self::Tracer),
            "translucent" => Some(Self::Translucent),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct CablePresentation {
    pub opacity: f32,
    pub tension: f32,
    pub plug_brightness: f32,
    pub finish: CableFinish,
}

impl Default for CablePresentation {
    fn default() -> Self {
        Self {
            opacity: 1.0,
            tension: 0.5,
            plug_brightness: 1.0,
            finish: CableFinish::Classic,
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
            finish: self.finish,
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
            finish: settings["cable_finish"]
                .as_str()
                .and_then(CableFinish::from_key)
                .unwrap_or_default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn cable_finish_settings_preserve_legacy_defaults_and_saved_selection() {
        for settings in [
            json!({}),
            json!({"cable_finish":false}),
            json!({"cable_finish":"unknown"}),
        ] {
            assert_eq!(
                CablePresentation::from_settings(&settings).finish,
                CableFinish::Classic
            );
        }
        for finish in CableFinish::ALL {
            let presentation = CablePresentation::from_settings(&json!({
                "cable_finish":finish.key(), "cable_opacity":0.4,
                "cable_tension":0.8,"plug_light_brightness":0.2
            }));
            assert_eq!(presentation.finish, finish);
            assert_eq!(presentation.sanitized(), presentation);
            assert_eq!(presentation.opacity, 0.4);
            assert_eq!(presentation.tension, 0.8);
            assert_eq!(presentation.plug_brightness, 0.2);
        }
    }
}
