//! Explicitly qualified plugin profiles, not automatic admission by port shape.
use super::{HostedIdentity, HostedPort, PortDirection};
use serde::Serialize;

pub const MAX_PARAMETERS: usize = 3;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
#[repr(u32)]
pub enum HostedCapability {
    Offset = 1,
    XFade = 2,
}

#[derive(Clone, Copy, Debug)]
pub struct HostedParameter {
    pub key: &'static str,
    pub minimum: f32,
    pub maximum: f32,
    pub default: f32,
    pub discrete: bool,
}
impl HostedParameter {
    pub fn accepts(self, value: f32) -> bool {
        value.is_finite()
            && (self.minimum..=self.maximum).contains(&value)
            && (!self.discrete || value.fract() == 0.0)
    }
}
const OFFSET: [HostedParameter; 2] = [
    HostedParameter {
        key: "offset",
        minimum: -1.,
        maximum: 1.,
        default: 0.,
        discrete: false,
    },
    HostedParameter {
        key: "scale",
        minimum: -1.,
        maximum: 1.,
        default: 0.,
        discrete: false,
    },
];
const XFADE: [HostedParameter; 3] = [
    HostedParameter {
        key: "mix",
        minimum: -1.,
        maximum: 1.,
        default: 0.,
        discrete: false,
    },
    HostedParameter {
        key: "curve",
        minimum: 0.,
        maximum: 1.,
        default: 0.5,
        discrete: false,
    },
    HostedParameter {
        key: "linear",
        minimum: 0.,
        maximum: 1.,
        default: 0.,
        discrete: true,
    },
];
impl HostedCapability {
    pub fn from_identity(identity: &HostedIdentity) -> Result<Self, String> {
        if identity.plugin != "Bogaudio"
            || identity.version != "2.6.47"
            || identity.rack_version != "2.6.6"
        {
            return Err("hosted identity is not a supported pinned capability".into());
        }
        Self::from_model(&identity.model)
    }
    pub fn from_model(model: &str) -> Result<Self, String> {
        match model {
            "Bogaudio-Offset" => Ok(Self::Offset),
            "Bogaudio-XFade" => Ok(Self::XFade),
            _ => Err("unsupported hosted model".into()),
        }
    }
    pub fn from_wire(id: u32) -> Result<Self, String> {
        match id {
            1 => Ok(Self::Offset),
            2 => Ok(Self::XFade),
            _ => Err("unsupported hosted wire capability".into()),
        }
    }
    pub const fn model(self) -> &'static str {
        match self {
            Self::Offset => "Bogaudio-Offset",
            Self::XFade => "Bogaudio-XFade",
        }
    }
    pub const fn parameters(self) -> &'static [HostedParameter] {
        match self {
            Self::Offset => &OFFSET,
            Self::XFade => &XFADE,
        }
    }
    pub fn parameter_id(self, key: &str) -> Result<u16, String> {
        self.parameters()
            .iter()
            .position(|p| p.key == key)
            .map(|i| i as u16)
            .ok_or_else(|| "unknown hosted parameter".into())
    }
    pub fn parameter_key(self, id: u16) -> Result<&'static str, String> {
        self.parameters()
            .get(id as usize)
            .map(|p| p.key)
            .ok_or_else(|| "unknown hosted parameter ID".into())
    }
    pub fn accepts_parameter(self, id: u16, value: f32) -> bool {
        self.parameters()
            .get(id as usize)
            .is_some_and(|p| p.accepts(value))
    }
    pub const fn input_keys(self) -> [&'static str; 3] {
        match self {
            Self::Offset => ["offset_cv", "scale_cv", "in"],
            Self::XFade => ["mix_cv", "a", "b"],
        }
    }
    pub const fn required_input_mask(self) -> u32 {
        match self {
            Self::Offset => 4,
            Self::XFade => 2,
        }
    }
    pub fn accepts_input_mask(self, mask: u32) -> bool {
        mask & !7 == 0 && mask & self.required_input_mask() != 0
    }
    pub fn ports(self) -> Vec<HostedPort> {
        self.input_keys()
            .into_iter()
            .enumerate()
            .map(|(index, key)| HostedPort {
                direction: PortDirection::Input,
                index: index as u32,
                key: key.into(),
            })
            .chain(std::iter::once(HostedPort {
                direction: PortDirection::Output,
                index: 0,
                key: "out".into(),
            }))
            .collect()
    }
}
