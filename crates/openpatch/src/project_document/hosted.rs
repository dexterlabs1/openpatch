//! Portable authored identity, ports and full persistent state for hosted modules.
//! Structural validity preserves unknown plugins; execution support is explicit.

use super::PortDirection;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::io::{self, Write};
use std::sync::Arc;

const STATE_LIMIT: usize = 1024 * 1024;
const MAX_RACK_ID: f64 = 9_007_199_254_740_991.0;

/// Immutable full authored snapshot with a cached canonical content identity.
#[derive(Clone, Debug, PartialEq)]
pub struct HostedState(Arc<HostedStateInner>);

#[derive(Debug, PartialEq)]
struct HostedStateInner {
    value: Value,
    sha256: String,
    encoded_len: usize,
}

impl HostedState {
    pub fn new(mut value: Value) -> Result<Self, String> {
        // Keep the identity stable even when serde_json preserves object order.
        value.sort_all_objects();
        struct FingerprintWriter {
            hash: Sha256,
            length: usize,
        }
        impl Write for FingerprintWriter {
            fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                if bytes.len() > STATE_LIMIT - self.length {
                    return Err(io::Error::other("hosted state exceeds 1 MiB"));
                }
                self.hash.update(bytes);
                self.length += bytes.len();
                Ok(bytes.len())
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let mut encoded = FingerprintWriter {
            hash: Sha256::new(),
            length: 0,
        };
        serde_json::to_writer(&mut encoded, &value)
            .map_err(|_| "hosted state exceeds 1 MiB or cannot serialize".to_string())?;
        Ok(Self(Arc::new(HostedStateInner {
            value,
            sha256: format!("{:x}", encoded.hash.finalize()),
            encoded_len: encoded.length,
        })))
    }

    pub fn id(&self) -> &str {
        &self.0.sha256
    }
    pub fn encoded_len(&self) -> usize {
        self.0.encoded_len
    }
}

impl AsRef<Value> for HostedState {
    fn as_ref(&self) -> &Value {
        &self.0.value
    }
}

impl Serialize for HostedState {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.as_ref().serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for HostedState {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::new(Value::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostedIdentity {
    pub plugin: String,
    pub model: String,
    pub version: String,
    pub rack_version: String,
    pub plugin_sha256: String,
    pub runtime_sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostedPort {
    pub direction: PortDirection,
    pub index: u32,
    pub key: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostedModule {
    pub schema_version: u32,
    pub identity: HostedIdentity,
    pub ports: Vec<HostedPort>,
    /// The sole authored parameter/custom-state authority; history shares it.
    pub state: HostedState,
}

impl HostedModule {
    /// Validate preservation limits without declaring any execution capability.
    pub fn validate_structure(&self) -> Result<(), String> {
        if self.schema_version != 1 {
            return Err("unsupported hosted schema version".into());
        }
        for text in [
            &self.identity.plugin,
            &self.identity.model,
            &self.identity.version,
            &self.identity.rack_version,
        ] {
            if text.is_empty() || text.len() > 256 || text.chars().any(char::is_control) {
                return Err("hosted identity requires bounded nonempty text".into());
            }
        }
        for hash in [&self.identity.plugin_sha256, &self.identity.runtime_sha256] {
            if hash.len() != 64
                || !hash
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            {
                return Err("hosted fingerprints must be lowercase SHA-256".into());
            }
        }
        if self.ports.len() > 128 {
            return Err("hosted module exceeds 128 ports".into());
        }
        let mut indices = BTreeSet::new();
        let mut keys = BTreeSet::new();
        for port in &self.ports {
            super::validate_id(&port.key).map_err(|_| "invalid hosted port key".to_string())?;
            if port.index >= 128
                || !indices.insert((port.direction, port.index))
                || !keys.insert((port.direction, &port.key))
            {
                return Err(
                    "hosted ports require bounded unique direction/index and direction/key".into(),
                );
            }
        }
        if !self.state.as_ref().is_object()
            || self.state.as_ref()["plugin"].as_str() != Some(self.identity.plugin.as_str())
            || self.state.as_ref()["model"].as_str() != Some(self.identity.model.as_str())
            || self.state.as_ref()["version"].as_str() != Some(self.identity.version.as_str())
        {
            return Err("hosted state identity differs from authored identity".into());
        }
        if !self.state.as_ref()["id"].as_f64().is_some_and(|id| {
            id.is_finite() && (0.0..=MAX_RACK_ID).contains(&id) && id.fract() == 0.0
        }) {
            return Err("hosted state requires a nonnegative integral Rack id <= 2^53-1".into());
        }
        if self.state.encoded_len() > STATE_LIMIT {
            return Err("hosted state exceeds 1 MiB".into());
        }
        Ok(())
    }

    /// Qualify only the explicitly pinned first capability, without loading it.
    pub fn validate_supported_offset(&self) -> Result<(), String> {
        self.validate_structure()?;
        if self.identity.plugin != "Bogaudio"
            || self.identity.model != "Bogaudio-Offset"
            || self.identity.version != "2.6.47"
            || self.identity.rack_version != "2.6.6"
        {
            return Err("hosted identity is not the supported Offset capability".into());
        }
        let expected = offset_ports();
        if self.ports.len() != expected.len()
            || !expected.iter().all(|expected| {
                self.ports.iter().any(|port| {
                    port.direction == expected.direction
                        && port.index == expected.index
                        && port.key == expected.key
                })
            })
        {
            return Err("hosted Offset port manifest differs from supported capability".into());
        }
        validate_offset_snapshot(self.state.as_ref())
    }

    pub fn offset(
        identity: HostedIdentity,
        offset: f32,
        scale: f32,
        offset_first: bool,
        disable_output_limit: bool,
    ) -> Result<Self, String> {
        validate_parameter(offset)?;
        validate_parameter(scale)?;
        let module = Self {
            schema_version: 1,
            identity,
            ports: offset_ports(),
            state: HostedState::new(json!({
                "plugin": "Bogaudio", "model": "Bogaudio-Offset", "version": "2.6.47", "id": 2,
                "params": [
                    {"id": 0, "value": f64::from(offset)},
                    {"id": 1, "value": f64::from(scale)}
                ],
                "data": {"offset_first": offset_first, "disableOutputLimit": disable_output_limit}
            }))?,
        };
        module.validate_supported_offset()?;
        Ok(module)
    }

    pub fn parameter(&self, key: &str) -> Result<f32, String> {
        self.validate_supported_offset()?;
        let id = parameter_id(key)?;
        self.state.as_ref()["params"]
            .as_array()
            .and_then(|params| params.iter().find(|param| param["id"].as_u64() == Some(id)))
            .and_then(|param| param["value"].as_f64())
            .map(|value| value as f32)
            .ok_or_else(|| "hosted Offset parameter missing".into())
    }

    /// Return a validated new snapshot; existing history/state stays immutable.
    pub fn with_parameter(&self, key: &str, value: f32) -> Result<Self, String> {
        self.validate_supported_offset()?;
        validate_parameter(value)?;
        let id = parameter_id(key)?;
        let mut updated = self.clone();
        let mut state = self.state.as_ref().clone();
        let param = state["params"]
            .as_array_mut()
            .and_then(|params| {
                params
                    .iter_mut()
                    .find(|param| param["id"].as_u64() == Some(id))
            })
            .ok_or_else(|| "hosted Offset parameter missing".to_string())?;
        param["value"] = json!(f64::from(value));
        updated.state = HostedState::new(state)?;
        updated.validate_supported_offset()?;
        Ok(updated)
    }

    pub fn option(&self, key: &str) -> Result<bool, String> {
        self.validate_supported_offset()?;
        let key = match key {
            "offset_first" => "offset_first",
            "disable_output_limit" => "disableOutputLimit",
            _ => return Err("unknown hosted Offset option".into()),
        };
        self.state.as_ref()["data"][key]
            .as_bool()
            .ok_or_else(|| "hosted Offset option missing".into())
    }

    pub fn with_option(&self, key: &str, value: bool) -> Result<Self, String> {
        self.validate_supported_offset()?;
        let source_key = match key {
            "offset_first" => "offset_first",
            "disable_output_limit" => "disableOutputLimit",
            _ => return Err("unknown hosted Offset option".into()),
        };
        let mut updated = self.clone();
        let mut state = self.state.as_ref().clone();
        state["data"][source_key] = json!(value);
        updated.state = HostedState::new(state)?;
        updated.validate_supported_offset()?;
        Ok(updated)
    }
}

/// Validate the complete snapshot accepted by the pinned companion. Extra fields
/// remain preservable by `HostedModule`, but cannot silently enter execution.
pub fn validate_offset_snapshot(state: &Value) -> Result<(), String> {
    exact_keys(
        state,
        &["data", "id", "model", "params", "plugin", "version"],
    )?;
    if state["plugin"] != "Bogaudio"
        || state["model"] != "Bogaudio-Offset"
        || state["version"] != "2.6.47"
        || state["id"].as_i64() != Some(2)
    {
        return Err("Offset snapshot identity/version/id mismatch".into());
    }
    let params = state["params"]
        .as_array()
        .filter(|params| params.len() == 2)
        .ok_or_else(|| "Offset snapshot requires both parameters".to_string())?;
    let mut seen = 0u8;
    for param in params {
        exact_keys(param, &["id", "value"])?;
        let id = param["id"]
            .as_u64()
            .filter(|id| *id < 2)
            .ok_or_else(|| "invalid Offset snapshot parameter id".to_string())?;
        let value = param["value"]
            .as_f64()
            .ok_or_else(|| "invalid Offset snapshot parameter value".to_string())?;
        validate_parameter(value as f32)?;
        if !value.is_finite() || f64::from(value as f32) != value {
            return Err("Offset snapshot parameter must roundtrip exactly through f32".into());
        }
        if seen & (1 << id) != 0 {
            return Err("duplicate Offset snapshot parameter id".into());
        }
        seen |= 1 << id;
    }
    exact_keys(&state["data"], &["offset_first", "disableOutputLimit"])?;
    if !state["data"]["offset_first"].is_boolean()
        || !state["data"]["disableOutputLimit"].is_boolean()
    {
        return Err("Offset snapshot requires explicit boolean custom flags".into());
    }
    validate_state_size(state)
}

fn exact_keys(value: &Value, expected: &[&str]) -> Result<(), String> {
    let object = value
        .as_object()
        .ok_or_else(|| "Offset snapshot field must be an object".to_string())?;
    if object.len() != expected.len() || !expected.iter().all(|key| object.contains_key(*key)) {
        return Err("Offset snapshot contains missing or unsupported fields".into());
    }
    Ok(())
}

fn validate_parameter(value: f32) -> Result<(), String> {
    if !value.is_finite() || !(-1.0..=1.0).contains(&value) {
        return Err("Offset parameter must be finite and within [-1, 1]".into());
    }
    Ok(())
}

fn parameter_id(key: &str) -> Result<u64, String> {
    match key {
        "offset" => Ok(0),
        "scale" => Ok(1),
        _ => Err("unknown hosted Offset parameter".into()),
    }
}

fn offset_ports() -> Vec<HostedPort> {
    [
        (PortDirection::Input, 0, "offset_cv"),
        (PortDirection::Input, 1, "scale_cv"),
        (PortDirection::Input, 2, "in"),
        (PortDirection::Output, 0, "out"),
    ]
    .into_iter()
    .map(|(direction, index, key)| HostedPort {
        direction,
        index,
        key: key.into(),
    })
    .collect()
}

fn validate_state_size(state: &Value) -> Result<(), String> {
    struct BoundedSize(usize);
    impl Write for BoundedSize {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            if bytes.len() > STATE_LIMIT - self.0 {
                return Err(io::Error::other("hosted state exceeds 1 MiB"));
            }
            self.0 += bytes.len();
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    serde_json::to_writer(&mut BoundedSize(0), state)
        .map_err(|_| "hosted state exceeds 1 MiB or cannot serialize".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity() -> HostedIdentity {
        HostedIdentity {
            plugin: "Bogaudio".into(),
            model: "Bogaudio-Offset".into(),
            version: "2.6.47".into(),
            rack_version: "2.6.6".into(),
            plugin_sha256: "a".repeat(64),
            runtime_sha256: "1".repeat(64),
        }
    }
    fn offset() -> HostedModule {
        HostedModule::offset(identity(), 0.25, -0.5, true, false).unwrap()
    }

    #[test]
    fn unknown_module_state_survives_roundtrip_without_execution_support() {
        let mut module = offset();
        module.identity.plugin = "FutureVendor".into();
        module.identity.model = "FutureModule".into();
        module.identity.version = "8.7".into();
        module.identity.rack_version = "9.0".into();
        module.ports = vec![HostedPort {
            direction: PortDirection::Input,
            index: 12,
            key: "future.input".into(),
        }];
        module.state = HostedState::new(
            json!({"plugin":"FutureVendor", "model":"FutureModule", "version":"8.7", "id":42,
            "unknown": {"nested":[1,"unchanged",{"future":true}]}, "data":{"custom":"keep"}}),
        )
        .unwrap();
        module.validate_structure().unwrap();
        assert!(module.validate_supported_offset().is_err());
        let before = serde_json::to_value(&module).unwrap();
        let restored: HostedModule = serde_json::from_value(before.clone()).unwrap();
        restored.validate_structure().unwrap();
        assert_eq!(serde_json::to_value(&restored).unwrap(), before);
        let canonical = serde_json::to_vec(module.state.as_ref()).unwrap();
        assert_eq!(module.state.encoded_len(), canonical.len());
        assert_eq!(
            module.state.id(),
            format!("{:x}", Sha256::digest(&canonical))
        );
        assert_eq!(restored.state.id(), module.state.id());
        assert_eq!(restored.state.encoded_len(), module.state.encoded_len());
        assert_eq!(
            serde_json::to_value(&module.state).unwrap(),
            *module.state.as_ref()
        );
        let oversized = json!({"opaque": "x".repeat(STATE_LIMIT)});
        assert!(HostedState::new(oversized.clone()).is_err());
        assert!(serde_json::from_value::<HostedState>(oversized).is_err());
    }

    #[test]
    fn edits_preserve_history_and_use_exact_f32_snapshot_values() {
        let original = offset();
        let history = original.clone();
        assert!(Arc::ptr_eq(&original.state.0, &history.state.0));
        let updated = original
            .with_parameter("offset", 0.1)
            .unwrap()
            .with_parameter("scale", 0.75)
            .unwrap()
            .with_option("offset_first", false)
            .unwrap()
            .with_option("disable_output_limit", true)
            .unwrap();
        assert_eq!(updated.parameter("offset").unwrap(), 0.1f32);
        assert_eq!(
            updated.state.as_ref()["params"][0]["value"].as_f64(),
            Some(f64::from(0.1f32))
        );
        assert_eq!(updated.parameter("scale").unwrap(), 0.75);
        assert_eq!(
            updated.state.as_ref()["data"],
            json!({"offset_first":false,"disableOutputLimit":true})
        );
        assert_eq!(original.parameter("offset").unwrap(), 0.25);
        assert_eq!(history.state, original.state);
        assert_eq!(history.state.id(), original.state.id());
        assert_ne!(updated.state.id(), original.state.id());
        assert!(!Arc::ptr_eq(&original.state.0, &updated.state.0));
        let restored: HostedModule =
            serde_json::from_slice(&serde_json::to_vec(&updated).unwrap()).unwrap();
        restored.validate_supported_offset().unwrap();
        assert_eq!(restored.state, updated.state);
    }

    #[test]
    fn execution_refuses_wrong_fingerprints_ports_values_and_unknown_fields() {
        let mut module = offset();
        module.identity.plugin_sha256 = "A".repeat(64);
        assert!(module.validate_structure().is_err());
        let mut module = offset();
        module.ports[0].key = "unknown".into();
        module.validate_structure().unwrap();
        assert!(module.validate_supported_offset().is_err());
        assert!(HostedModule::offset(identity(), f32::NAN, 0.0, false, false).is_err());
        assert!(offset().with_parameter("scale", f32::INFINITY).is_err());
        assert!(offset().with_parameter("offset", 1.01).is_err());
        for (location, key) in [("", "future"), ("data", "future")] {
            let mut module = offset();
            let mut state = module.state.as_ref().clone();
            if location.is_empty() {
                state[key] = json!(true);
            } else {
                state[location][key] = json!(true);
            }
            module.state = HostedState::new(state).unwrap();
            module.validate_structure().unwrap();
            assert!(module.validate_supported_offset().is_err());
        }
        let mut module = offset();
        let mut state = module.state.as_ref().clone();
        state["params"][0]["value"] = json!(0.1f64);
        module.state = HostedState::new(state).unwrap();
        module.validate_structure().unwrap();
        assert!(module.validate_supported_offset().is_err());
    }
}
