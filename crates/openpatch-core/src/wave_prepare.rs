//! Control-thread preparation and explicit shared admission. Banks are immutable;
//! reservation lifetimes follow the last bank owner, including retired graphs.
use crate::{
    wavetable_bank::{estimate_memory, PreparedWaveBank, WavetableMode},
    ModuleContent, Patch, WaveAsset,
};
use std::{
    collections::{BTreeMap, HashMap, HashSet, VecDeque},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex, OnceLock, Weak,
    },
};

const MEMORY_LIMIT: usize = 1024 * 1024 * 1024;
const RECENT_LIMIT: usize = 128 * 1024 * 1024;
const BANK_LIMIT: usize = 4096;

#[derive(Debug, Default)]
struct Budget {
    bytes: AtomicUsize,
    banks: AtomicUsize,
}
#[derive(Debug)]
pub(crate) struct BankLease {
    budget: Arc<Budget>,
    bytes: usize,
}
impl Drop for BankLease {
    fn drop(&mut self) {
        self.budget.bytes.fetch_sub(self.bytes, Ordering::AcqRel);
        self.budget.banks.fetch_sub(1, Ordering::AcqRel);
    }
}
struct ScratchLease {
    budget: Arc<Budget>,
    bytes: usize,
}
impl Drop for ScratchLease {
    fn drop(&mut self) {
        self.budget.bytes.fetch_sub(self.bytes, Ordering::AcqRel);
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct Key {
    digest: String,
    points: usize,
    mode: WavetableMode,
}
#[derive(Debug, Default)]
struct Registry {
    banks: HashMap<Key, Weak<PreparedWaveBank>>,
    building: HashSet<Key>,
    recent: VecDeque<Arc<PreparedWaveBank>>,
    recent_bytes: usize,
}
#[derive(Debug)]
struct Shared {
    registry: Mutex<Registry>,
    budget: Arc<Budget>,
    basic: OnceLock<Arc<WaveAsset>>,
    memory_limit: usize,
    recent_limit: usize,
}
/// Clone and share this control-only cache across sessions and graph builders.
/// No preparation, mutex or cache lookup is performed by the audio callback.
#[derive(Clone, Debug)]
pub struct WavePreparationCache(Arc<Shared>);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WavePreparationStats {
    pub admitted_bytes: usize,
    pub live_banks: usize,
}

/// Mathematical default, generated locally: sine, triangle, saw and square.
/// No wave asset from a third-party module is distributed.
pub(crate) fn basic_asset() -> Arc<WaveAsset> {
    let mut samples = Vec::with_capacity(4096);
    for wave in 0..4 {
        for point in 0..1024 {
            let phase = point as f32 / 1024.0;
            samples.push(match wave {
                0 => (2.0 * std::f32::consts::PI * phase).sin(),
                1 => {
                    if phase < 0.25 {
                        phase * 4.0
                    } else if phase < 0.75 {
                        2.0 - phase * 4.0
                    } else {
                        phase * 4.0 - 4.0
                    }
                }
                2 => {
                    if phase < 0.5 {
                        2.0 * phase
                    } else {
                        2.0 * phase - 2.0
                    }
                }
                _ => {
                    if phase < 0.5 {
                        1.0
                    } else {
                        -1.0
                    }
                }
            });
        }
    }
    Arc::new(WaveAsset::new(&samples).expect("finite mathematical default"))
}
impl Default for WavePreparationCache {
    fn default() -> Self {
        Self::with_limits(MEMORY_LIMIT, RECENT_LIMIT)
    }
}
impl WavePreparationCache {
    /// A host may choose a smaller combined retained/preparation payload budget.
    /// Pinned banks remain charged even after their cache is cleared or dropped.
    pub fn with_memory_limit(bytes: usize) -> Self {
        Self::with_limits(bytes, RECENT_LIMIT.min(bytes))
    }

    fn with_limits(memory_limit: usize, recent_limit: usize) -> Self {
        Self(Arc::new(Shared {
            registry: Mutex::new(Registry::default()),
            budget: Arc::new(Budget::default()),
            basic: OnceLock::new(),
            memory_limit,
            recent_limit,
        }))
    }
    pub fn stats(&self) -> WavePreparationStats {
        WavePreparationStats {
            admitted_bytes: self.0.budget.bytes.load(Ordering::Acquire),
            live_banks: self.0.budget.banks.load(Ordering::Acquire),
        }
    }
    /// Drop only cache-owned strong references. Pinned engines remain charged.
    pub fn clear_unused(&self) {
        let mut registry = self.0.registry.lock().unwrap_or_else(|e| e.into_inner());
        registry.recent.clear();
        registry.recent_bytes = 0;
        registry.banks.retain(|_, weak| weak.strong_count() > 0);
    }
    #[allow(deprecated)] // fetch_update is retained for the workspace minimum Rust version.
    fn reserve(
        &self,
        retained: usize,
        scratch: usize,
    ) -> Result<(BankLease, ScratchLease), String> {
        let total = retained
            .checked_add(scratch)
            .ok_or("Wavetable admission size overflow")?;
        let budget = &self.0.budget;
        budget
            .banks
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                (n < BANK_LIMIT).then_some(n + 1)
            })
            .map_err(|_| "Too many pinned wavetable banks")?;
        if budget
            .bytes
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                n.checked_add(total).filter(|&n| n <= self.0.memory_limit)
            })
            .is_err()
        {
            budget.banks.fetch_sub(1, Ordering::AcqRel);
            return Err("Wavetable preparation exceeds shared memory budget; release unused graphs or choose a smaller bank".into());
        }
        Ok((
            BankLease {
                budget: budget.clone(),
                bytes: retained,
            },
            ScratchLease {
                budget: budget.clone(),
                bytes: scratch,
            },
        ))
    }
    fn resolve(
        &self,
        asset: &Arc<WaveAsset>,
        points: usize,
        mode: WavetableMode,
    ) -> Result<Arc<PreparedWaveBank>, String> {
        let key = Key {
            digest: asset.id().to_owned(),
            points,
            mode,
        };
        {
            let mut registry = self.0.registry.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(bank) = registry.banks.get(&key).and_then(Weak::upgrade) {
                return Ok(bank);
            }
            if registry.building.contains(&key) {
                return Err("This wavetable bank is already being prepared; retry after preparation completes".into());
            }
            // Metadata is bounded by live/admitted banks, not lifetime imports.
            registry.banks.retain(|_, weak| weak.strong_count() > 0);
            registry.building.insert(key.clone());
        }
        let result = (|| {
            let estimate = estimate_memory(asset.sample_count(), points, mode)?;
            {
                let mut registry = self.0.registry.lock().unwrap_or_else(|e| e.into_inner());
                while self
                    .stats()
                    .admitted_bytes
                    .saturating_add(estimate.retained_bytes)
                    .saturating_add(estimate.scratch_bytes)
                    > self.0.memory_limit
                    || self.stats().live_banks >= BANK_LIMIT
                {
                    let Some(old) = registry.recent.pop_front() else {
                        break;
                    };
                    registry.recent_bytes -= old.prepared_bytes();
                    drop(old);
                }
            }
            let (lease, scratch) = self.reserve(estimate.retained_bytes, estimate.scratch_bytes)?;
            let mut bank = PreparedWaveBank::prepare(asset.decoded_samples(), points, mode)?;
            if bank.prepared_bytes() > estimate.retained_bytes {
                return Err("Allocator exceeded admitted wavetable capacity".into());
            }
            bank.set_lease(lease);
            drop(scratch);
            Ok(Arc::new(bank))
        })();
        let mut registry = self.0.registry.lock().unwrap_or_else(|e| e.into_inner());
        registry.building.remove(&key);
        if let Ok(bank) = &result {
            registry.banks.insert(key, Arc::downgrade(bank));
            if bank.prepared_bytes() <= self.0.recent_limit {
                registry.recent_bytes += bank.prepared_bytes();
                registry.recent.push_back(bank.clone());
                while registry.recent_bytes > self.0.recent_limit || registry.recent.len() > 256 {
                    if let Some(old) = registry.recent.pop_front() {
                        registry.recent_bytes -= old.prepared_bytes();
                    }
                }
            }
        }
        result
    }
}
#[derive(Debug)]
struct Entry {
    key: Key,
    is_default: bool,
    bank: Arc<PreparedWaveBank>,
}
/// Immutable preparation ticket. Sample-rate/parameter edits reuse its banks;
/// geometry, model, or sample changes require a new ticket before publication.
#[derive(Debug, Default)]
pub struct PreparedWaveBanks {
    entries: BTreeMap<String, Entry>,
}
impl PreparedWaveBanks {
    pub(crate) fn get(&self, id: &str) -> Option<Arc<PreparedWaveBank>> {
        self.entries.get(id).map(|e| e.bank.clone())
    }
    pub(crate) fn validate(&self, patch: &Patch) -> Result<(), String> {
        let mut count = 0;
        for node in &patch.nodes {
            if let Some(mode) = mode(&node.kind) {
                count += 1;
                let entry = self
                    .entries
                    .get(&node.id)
                    .ok_or("Missing prepared wavetable bank")?;
                let (digest, points) = match &node.content {
                    Some(ModuleContent::Wavetable {
                        asset_id,
                        wave_points,
                        ..
                    }) => {
                        let asset = patch
                            .assets
                            .get(asset_id)
                            .ok_or("Prepared wavetable asset is missing from patch")?;
                        estimate_memory(asset.sample_count(), *wave_points as usize, mode)?;
                        (asset_id.as_str(), *wave_points as usize)
                    }
                    None => (entry.key.digest.as_str(), 1024),
                    _ => return Err("Invalid wavetable content".into()),
                };
                if entry.key.mode != mode
                    || entry.key.points != points
                    || entry.key.digest != digest
                    || (node.content.is_none() != entry.is_default)
                {
                    return Err(format!("node '{}': stale prepared wavetable bank", node.id));
                }
            }
        }
        if count != self.entries.len() {
            return Err("Prepared wavetable ticket does not match graph".into());
        }
        Ok(())
    }
}
fn mode(kind: &str) -> Option<WavetableMode> {
    match kind {
        "wavetable_vco" => Some(WavetableMode::Audio),
        "wavetable_lfo" => Some(WavetableMode::Modulation),
        _ => None,
    }
}
impl Patch {
    pub fn prepare_wave_banks(
        &self,
        cache: &WavePreparationCache,
    ) -> Result<Arc<PreparedWaveBanks>, String> {
        crate::wave_assets::validate_assets(&self.assets)?;
        if self.nodes.len() > crate::MAX_NODES {
            return Err("Too many nodes for wavetable preparation".into());
        }
        let mut result = PreparedWaveBanks::default();
        for node in &self.nodes {
            crate::validate_content(node)?;
            if let Some(mode) = mode(&node.kind) {
                if self.version != 2 {
                    return Err("Wavetable modules require patch version 2".into());
                }
                let (asset, points) = match &node.content {
                    Some(ModuleContent::Wavetable {
                        asset_id,
                        wave_points,
                        ..
                    }) => (
                        self.assets.get(asset_id).ok_or_else(|| {
                            format!("node '{}': missing wavetable asset", node.id)
                        })?,
                        *wave_points as usize,
                    ),
                    None => (cache.0.basic.get_or_init(basic_asset), 1024),
                    _ => return Err("Invalid wavetable content".into()),
                };
                if result.entries.contains_key(&node.id) {
                    return Err("Duplicate wavetable node ID".into());
                }
                let bank = cache.resolve(asset, points, mode)?;
                result.entries.insert(
                    node.id.clone(),
                    Entry {
                        is_default: node.content.is_none(),
                        key: Key {
                            digest: asset.id().to_owned(),
                            points,
                            mode,
                        },
                        bank,
                    },
                );
            }
        }
        Ok(Arc::new(result))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Node;
    fn patch() -> Patch {
        let mut p = Patch::new_v2();
        p.nodes.push(Node {
            id: "wt".into(),
            kind: "wavetable_lfo".into(),
            params: Default::default(),
            bypassed: false,
            score: None,
            content: None,
        });
        p.output = Some("wt".into());
        p
    }
    #[test]
    fn banks_share_across_parameters_and_sample_rates_but_reject_stale_geometry() {
        let cache = WavePreparationCache::default();
        let p = patch();
        let a = p.prepare_wave_banks(&cache).unwrap();
        let mut edited = p.clone();
        edited.sample_rate = 96000;
        edited.nodes[0].params.insert("rate".into(), 3.0);
        let b = edited.prepare_wave_banks(&cache).unwrap();
        assert!(Arc::ptr_eq(&a.get("wt").unwrap(), &b.get("wt").unwrap()));
        assert!(a.validate(&edited).is_ok());
        edited.nodes[0].content = Some(ModuleContent::Wavetable {
            asset_id: cache.0.basic.get_or_init(basic_asset).id().into(),
            wave_points: 512,
            name: "New geometry".into(),
        });
        edited.assets.insert(
            cache.0.basic.get_or_init(basic_asset).id().into(),
            cache.0.basic.get_or_init(basic_asset).clone(),
        );
        assert!(a.validate(&edited).is_err());
        let c = edited.prepare_wave_banks(&cache).unwrap();
        assert!(!Arc::ptr_eq(&a.get("wt").unwrap(), &c.get("wt").unwrap()));
    }
    #[test]
    fn admission_counts_pinned_banks_until_the_last_owner_drops() {
        let cache = WavePreparationCache::with_limits(65536, 0);
        let p = patch();
        let ticket = p.prepare_wave_banks(&cache).unwrap();
        assert_eq!(cache.stats().admitted_bytes, 16384);
        let mut changed = p.clone();
        let asset = Arc::new(WaveAsset::new(&vec![0.25; 8192]).unwrap());
        changed.assets.insert(asset.id().into(), asset.clone());
        changed.nodes[0].content = Some(ModuleContent::Wavetable {
            asset_id: asset.id().into(),
            wave_points: 1024,
            name: "Larger".into(),
        });
        assert!(changed.prepare_wave_banks(&cache).is_err());
        assert_eq!(cache.stats().live_banks, 1);
        drop(ticket);
        assert_eq!(cache.stats().admitted_bytes, 0);
        let ticket = changed.prepare_wave_banks(&cache).unwrap();
        assert_eq!(cache.stats().admitted_bytes, 32768);
        drop(ticket);
        assert_eq!(cache.stats().live_banks, 0);
    }
    #[test]
    fn invalid_preparations_release_budget_and_missing_assets_never_enter_cache() {
        let cache = WavePreparationCache::with_limits(32768, 0);
        let mut p = patch();
        p.nodes[0].kind = "wavetable_vco".into();
        assert!(p.prepare_wave_banks(&cache).is_err());
        assert_eq!(cache.stats().admitted_bytes, 0);
        assert_eq!(cache.stats().live_banks, 0);
        p.nodes[0].content = Some(ModuleContent::Wavetable {
            asset_id: "0".repeat(64),
            wave_points: 1024,
            name: "Missing".into(),
        });
        assert!(p.prepare_wave_banks(&cache).is_err());
        assert_eq!(cache.stats().live_banks, 0);
    }
    #[test]
    fn thousands_of_tiny_imports_do_not_pin_expired_bank_metadata() {
        let cache = WavePreparationCache::with_limits(1024 * 1024, 1024 * 1024);
        for value in 0..4100 {
            let mut p = patch();
            let asset = Arc::new(WaveAsset::new(&[value as f32, -(value as f32)]).unwrap());
            p.nodes[0].content = Some(ModuleContent::Wavetable {
                asset_id: asset.id().into(),
                wave_points: 2,
                name: "Small".into(),
            });
            p.assets.insert(asset.id().into(), asset);
            let prepared = p.prepare_wave_banks(&cache).unwrap();
            assert!(cache.stats().live_banks <= 257);
            drop(prepared);
        }
        cache.clear_unused();
        assert_eq!(cache.stats().admitted_bytes, 0);
        assert_eq!(cache.stats().live_banks, 0);
    }
}
