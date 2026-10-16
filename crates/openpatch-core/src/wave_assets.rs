//! Immutable, self-contained wavetable samples. All construction, hashing and
//! serialization is control-thread work. Cloning an asset shares its samples.
use base64::{engine::general_purpose::STANDARD, Engine as _};
use serde::{de, ser::SerializeStruct, Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, fmt, sync::Arc};

pub use crate::wavetable_file::MAX_WAVETABLE_SAMPLES;
pub const MAX_PATCH_ASSETS: usize = 256;
pub const MAX_PATCH_ASSET_BYTES: usize = 32 * 1024 * 1024;
pub const MAX_PATCH_FILE_BYTES: usize = 64 * 1024 * 1024;
pub const MAX_HISTORY_ASSET_BYTES: usize = 128 * 1024 * 1024;
const MAX_ENCODED_BYTES: usize = (MAX_WAVETABLE_SAMPLES * 4).div_ceil(3) * 4;
const ENCODING: &str = "f32le-base64-v1";
const HASH_DOMAIN: &[u8] = b"openpatch:wavetable:f32le:v1\0";

pub type AssetId = String;
pub type WaveAssets = BTreeMap<AssetId, Arc<WaveAsset>>;

/// Finite IEEE-754 sample bits, including signed zero. Names, channel metadata
/// and wave interpretation deliberately do not participate in the identity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WaveAsset {
    bits: Arc<[u32]>,
    id: AssetId,
}

impl WaveAsset {
    pub fn new(samples: &[f32]) -> Result<Self, String> {
        if samples.is_empty() || samples.len() > MAX_WAVETABLE_SAMPLES {
            return Err(format!(
                "wavetable asset requires 1..={MAX_WAVETABLE_SAMPLES} samples"
            ));
        }
        Self::from_bits(samples.iter().map(|sample| sample.to_bits()).collect())
    }

    fn from_bits(bits: Vec<u32>) -> Result<Self, String> {
        if bits.is_empty() || bits.len() > MAX_WAVETABLE_SAMPLES {
            return Err(format!(
                "wavetable asset requires 1..={MAX_WAVETABLE_SAMPLES} samples"
            ));
        }
        let mut digest = Sha256::new();
        digest.update(HASH_DOMAIN);
        for bit in &bits {
            if !f32::from_bits(*bit).is_finite() {
                return Err("wavetable asset contains a nonfinite sample".into());
            }
            digest.update(bit.to_le_bytes());
        }
        let id = format!("{:x}", digest.finalize());
        Ok(Self {
            bits: bits.into(),
            id,
        })
    }

    pub fn id(&self) -> &str {
        &self.id
    }
    pub fn sample_count(&self) -> usize {
        self.bits.len()
    }
    pub fn decoded_bytes(&self) -> usize {
        self.bits.len() * 4
    }
    pub fn samples(&self) -> impl ExactSizeIterator<Item = f32> + '_ {
        self.bits.iter().map(|bits| f32::from_bits(*bits))
    }
    pub fn sample(&self, index: usize) -> Option<f32> {
        self.bits.get(index).map(|bits| f32::from_bits(*bits))
    }
    /// Allocates a decoded view for a preparation worker, never for DSP.
    pub fn decoded_samples(&self) -> Arc<[f32]> {
        self.samples().collect::<Vec<_>>().into()
    }
    /// Compact inspection metadata, explicitly distinct from portable payloads.
    pub fn descriptor(&self) -> Value {
        json!({"sha256":self.id,"sample_count":self.sample_count(),
            "decoded_bytes":self.decoded_bytes(),"encoding":ENCODING,"payload_omitted":true})
    }
}

impl Serialize for WaveAsset {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut bytes = Vec::with_capacity(self.decoded_bytes());
        for bits in self.bits.iter() {
            bytes.extend_from_slice(&bits.to_le_bytes());
        }
        let data = STANDARD.encode(bytes);
        let mut object = serializer.serialize_struct("WaveAsset", 2)?;
        object.serialize_field("encoding", ENCODING)?;
        object.serialize_field("data", &data)?;
        object.end()
    }
}

struct EncodedSamples(Vec<u32>);
impl<'de> Deserialize<'de> for EncodedSamples {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Visitor;
        impl de::Visitor<'_> for Visitor {
            type Value = EncodedSamples;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("bounded canonical base64 little-endian f32 sample bytes")
            }
            fn visit_str<E: de::Error>(self, text: &str) -> Result<Self::Value, E> {
                if text.is_empty() || text.len() > MAX_ENCODED_BYTES {
                    return Err(E::custom(
                        "wavetable encoded payload exceeds its sample bound",
                    ));
                }
                let bytes = STANDARD.decode(text).map_err(E::custom)?;
                if bytes.len() % 4 != 0 || bytes.len() > MAX_WAVETABLE_SAMPLES * 4 {
                    return Err(E::custom("invalid wavetable sample byte length"));
                }
                let bits = bytes
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
                    .collect();
                Ok(EncodedSamples(bits))
            }
        }
        deserializer.deserialize_str(Visitor)
    }
}

impl<'de> Deserialize<'de> for WaveAsset {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Payload {
            encoding: String,
            data: EncodedSamples,
        }
        let payload = Payload::deserialize(deserializer)?;
        if payload.encoding != ENCODING {
            return Err(de::Error::custom("unsupported wavetable asset encoding"));
        }
        Self::from_bits(payload.data.0).map_err(de::Error::custom)
    }
}

pub(crate) fn deserialize_assets<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<WaveAssets, D::Error> {
    struct Visitor;
    impl<'de> de::Visitor<'de> for Visitor {
        type Value = WaveAssets;
        fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
            f.write_str("a bounded map of digest-addressed wavetable assets")
        }
        fn visit_map<A: de::MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
            let mut assets = WaveAssets::new();
            let mut bytes = 0;
            while let Some(id) = map.next_key::<String>()? {
                if id.len() != 64
                    || !id
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                {
                    return Err(de::Error::custom(
                        "wavetable asset ID must be 64 lowercase hexadecimal characters",
                    ));
                }
                if assets.len() >= MAX_PATCH_ASSETS || assets.contains_key(&id) {
                    return Err(de::Error::custom("too many or duplicate wavetable assets"));
                }
                let asset: Arc<WaveAsset> = map.next_value()?;
                if id != asset.id() {
                    return Err(de::Error::custom(
                        "wavetable asset digest does not match sample bytes",
                    ));
                }
                bytes += asset.decoded_bytes();
                if bytes > MAX_PATCH_ASSET_BYTES {
                    return Err(de::Error::custom("patch wavetable assets exceed 32 MiB"));
                }
                assets.insert(id, asset);
            }
            Ok(assets)
        }
    }
    deserializer.deserialize_map(Visitor)
}

pub(crate) fn validate_assets(assets: &WaveAssets) -> Result<(), String> {
    if assets.len() > MAX_PATCH_ASSETS {
        return Err("too many wavetable assets".into());
    }
    let mut bytes = 0;
    for (id, asset) in assets {
        if id != asset.id() {
            return Err("wavetable asset digest does not match sample bytes".into());
        }
        bytes += asset.decoded_bytes();
        if bytes > MAX_PATCH_ASSET_BYTES {
            return Err("patch wavetable assets exceed 32 MiB".into());
        }
    }
    Ok(())
}

impl crate::Patch {
    /// A compact graph inspection view. Assets contain descriptors instead of
    /// sample payloads. Serialize Patch itself for a self-contained document.
    /// With no assets this is identical to the ordinary persisted representation.
    pub fn reference_view(&self) -> Value {
        let mut view = json!({"version":self.version,"sample_rate":self.sample_rate,
            "nodes":self.nodes,"cables":self.cables,"output":self.output});
        if self.output_port != "out" {
            view["output_port"] = json!(self.output_port);
        }
        if !self.assets.is_empty() {
            view["assets"] = Value::Object(
                self.assets
                    .iter()
                    .map(|(id, asset)| (id.clone(), asset.descriptor()))
                    .collect(),
            );
        }
        view
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_bits_survive_serde_and_history_shares_samples() {
        let bits = [
            0_u32,
            0x8000_0000,
            0x3f80_0000,
            0xbf80_0000,
            0x3f00_0000,
            1,
            0x7f7f_ffff,
        ];
        let asset = WaveAsset::from_bits(bits.to_vec()).unwrap();
        let round: WaveAsset =
            serde_json::from_str(&serde_json::to_string(&asset).unwrap()).unwrap();
        assert_eq!(round, asset);
        assert_eq!(&*round.bits, &bits);
        let clone = asset.clone();
        assert!(Arc::ptr_eq(&clone.bits, &asset.bits));
        let mut changed = bits;
        changed[0] = 0x8000_0000;
        assert_ne!(
            asset.id(),
            WaveAsset::from_bits(changed.to_vec()).unwrap().id()
        );
    }
    #[test]
    fn representation_is_strict_and_finite() {
        for samples in [
            vec![],
            vec![f32::NAN],
            vec![f32::INFINITY],
            vec![f32::NEG_INFINITY],
        ] {
            assert!(WaveAsset::new(&samples).is_err());
        }
        for input in [
            json!({"encoding":"other","data":"AAAAAA=="}),
            json!({"encoding":ENCODING,"data":"AAAA"}),
            json!({"encoding":ENCODING,"data":"A!AA"}),
            json!({"encoding":ENCODING,"data":""}),
            json!({"encoding":ENCODING,"data":"AAAAAA==","extra":0}),
            json!({"encoding":ENCODING,"data":"AACAfw=="}),
        ] {
            assert!(serde_json::from_value::<WaveAsset>(input).is_err());
        }
    }
    #[test]
    fn legacy_json_is_unchanged_and_asset_views_omit_payloads() {
        let mut patch = crate::Patch::new_v2();
        assert_eq!(
            patch.reference_view(),
            serde_json::to_value(&patch).unwrap()
        );
        let asset = Arc::new(WaveAsset::new(&[0.0, 0.5, -0.5]).unwrap());
        patch.assets.insert(asset.id().into(), asset.clone());
        let full = serde_json::to_value(&patch).unwrap();
        let view = patch.reference_view();
        assert_eq!(view["assets"][asset.id()]["sample_count"], 3);
        assert!(view["assets"][asset.id()].get("data").is_none());
        assert!(full["assets"][asset.id()]["data"].is_string());
        let decoded: crate::Patch = serde_json::from_value(full).unwrap();
        assert_eq!(decoded.assets, patch.assets);
        assert!(serde_json::from_value::<crate::Patch>(view).is_err());
    }
    #[test]
    fn digest_mismatch_duplicate_keys_and_version_one_assets_reject() {
        let asset = Arc::new(WaveAsset::new(&[0.0, 1.0]).unwrap());
        let payload = serde_json::to_string(&asset).unwrap();
        let duplicate = format!(
            "{{\"version\":2,\"assets\":{{\"{}\":{},\"{}\":{}}}}}",
            asset.id(),
            payload,
            asset.id(),
            payload
        );
        assert!(serde_json::from_str::<crate::Patch>(&duplicate).is_err());
        assert!(serde_json::from_value::<crate::Patch>(
            json!({"version":2,"assets":{"bad":asset}})
        )
        .is_err());
        let mut patch = crate::Patch::default();
        patch.assets.insert(asset.id().into(), asset.clone());
        assert!(patch.compile().is_err());
        patch.version = 2;
        assert!(patch.compile().is_ok());
        patch.assets.insert("wrong".into(), asset);
        assert!(patch.compile().is_err());
    }
    #[test]
    fn maximum_asset_roundtrips_and_oversize_rejects() {
        let asset = WaveAsset::new(&vec![0.25; MAX_WAVETABLE_SAMPLES]).unwrap();
        let text = serde_json::to_string(&asset).unwrap();
        assert!(text.len() > 2 * 1024 * 1024 && text.len() < 4 * 1024 * 1024);
        assert_eq!(serde_json::from_str::<WaveAsset>(&text).unwrap(), asset);
        assert!(WaveAsset::new(&vec![0.0; MAX_WAVETABLE_SAMPLES + 1]).is_err());
        assert!(serde_json::from_value::<WaveAsset>(
            json!({"encoding":ENCODING,"data":"A".repeat(MAX_ENCODED_BYTES+4)})
        )
        .is_err());
    }
}
