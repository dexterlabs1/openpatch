//! Workspace wavetable operations. Decoding, hashing, preparation and file I/O
//! run on the control thread; a successful import uses the normal commit path.
use super::*;
use openpatch_core::wavetable_file::{decode_wavetable, encode_wavetable_wav};
use openpatch_core::{ModuleContent, WaveAsset};
use std::sync::atomic::{AtomicU64, Ordering};

const MAX_SOURCE_BYTES: usize = 16 * 1024 * 1024;

fn wave_node<'a>(patch: &'a Patch, id: &str) -> Result<&'a Node, String> {
    let node = patch
        .nodes
        .iter()
        .find(|node| node.id == id)
        .ok_or("module not found")?;
    if !matches!(node.kind.as_str(), "wavetable_vco" | "wavetable_lfo") {
        return Err("operation requires a wavetable module".into());
    }
    Ok(node)
}

fn points(args: &Value) -> Result<u32, String> {
    args.get("wave_points")
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
        .ok_or_else(|| "wave_points must be a positive integer fitting u32".into())
}

fn install(
    patch: &mut Patch,
    id: &str,
    asset_id: &str,
    wave_points: u32,
    name: &str,
) -> Result<(), String> {
    if !patch.assets.contains_key(asset_id) {
        let basic = openpatch_core::default_wavetable_asset();
        if basic.id() == asset_id {
            patch.assets.insert(asset_id.into(), basic);
        }
    }
    let asset = patch
        .assets
        .get(asset_id)
        .ok_or("wavetable asset not found")?;
    if asset.id() != asset_id {
        return Err("wavetable asset digest mismatch".into());
    }
    if wave_points < 2 || wave_points as usize > asset.sample_count() {
        return Err("wave_points must fit at least one complete wave".into());
    }
    let mut node = wave_node(patch, id)?.clone();
    node.content = Some(ModuleContent::Wavetable {
        asset_id: asset_id.into(),
        wave_points,
        name: name.into(),
    });
    openpatch_core::validate_content(&node)?;
    patch
        .nodes
        .iter_mut()
        .find(|node| node.id == id)
        .unwrap()
        .content = node.content;
    Ok(())
}

/// Graph edit for ordinary direct, preview and atomic batch workflows.
pub(super) fn set(candidate: &mut Patch, args: &Value) -> Result<(), String> {
    let id = string(args, "id")?;
    let asset_id = string(args, "asset_id")?;
    let name = string(args, "name")?;
    install(candidate, id, asset_id, points(args)?, name)
}

fn read_source(path: &Path) -> Result<Vec<u8>, String> {
    let metadata = fs::symlink_metadata(path).map_err(|error| error.to_string())?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err("wavetable must be a regular file, not a symlink".into());
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(0x0020_0000); // FILE_FLAG_OPEN_REPARSE_POINT
    }
    let file = options.open(path).map_err(|error| error.to_string())?;
    let metadata = file.metadata().map_err(|error| error.to_string())?;
    if !metadata.is_file() {
        return Err("wavetable must be a regular file".into());
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x0400 != 0 {
            return Err("wavetable must not be a reparse point".into());
        }
    }
    if metadata.len() > MAX_SOURCE_BYTES as u64 {
        return Err("Wavetable source exceeds 16 MiB".into());
    }
    let mut bytes = Vec::new();
    file.take(MAX_SOURCE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() > MAX_SOURCE_BYTES {
        return Err("Wavetable source exceeds 16 MiB".into());
    }
    Ok(bytes)
}

struct Staged(PathBuf);
impl Drop for Staged {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

fn save_new(path: &Path, bytes: &[u8]) -> Result<(), String> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let directory = path.parent().ok_or("wavetable path has no parent")?;
    for _ in 0..32 {
        let temporary = directory.join(format!(
            ".openpatch-wave-{}-{}.tmp",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let mut file = match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
        {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.to_string()),
        };
        let staged = Staged(temporary);
        file.write_all(bytes)
            .and_then(|_| file.sync_all())
            .map_err(|error| error.to_string())?;
        drop(file);
        fs::hard_link(&staged.0, path).map_err(|error| error.to_string())?;
        return Ok(());
    }
    Err("could not create an exclusive wavetable staging file".into())
}

fn selected<'a>(patch: &'a Patch, id: &str) -> Result<(Arc<WaveAsset>, u32, &'a str), String> {
    let node = wave_node(patch, id)?;
    match &node.content {
        Some(ModuleContent::Wavetable {
            asset_id,
            wave_points,
            name,
        }) => {
            let asset = patch
                .assets
                .get(asset_id)
                .ok_or("wavetable asset not found")?;
            Ok((asset.clone(), *wave_points, name))
        }
        None => Ok((openpatch_core::default_wavetable_asset(), 1024, "Basic")),
        _ => Err("wavetable module has incompatible content".into()),
    }
}

impl Session {
    pub(super) fn import_wavetable(&mut self, args: &Value) -> Result<Value, String> {
        self.check_open()?;
        let expected = expected_revision(args)?;
        self.check_revision(expected)?;
        let id = string(args, "id")?;
        let node = wave_node(&self.patch, id)?;
        let previous_points = match &node.content {
            Some(ModuleContent::Wavetable { wave_points, .. }) => *wave_points,
            _ => 1024,
        };
        let filename = string(args, "filename")?;
        let extension = Path::new(filename)
            .extension()
            .and_then(|extension| extension.to_str())
            .ok_or("wavetable filename requires a supported extension")?;
        let path = self.file(filename, extension)?;
        let decoded = decode_wavetable(&read_source(&path)?, extension)?;
        let wave_points = if args.get("wave_points").is_some() {
            points(args)?
        } else {
            decoded
                .suggested_wave_length
                .and_then(|points| u32::try_from(points).ok())
                .unwrap_or(previous_points)
        };
        let asset = Arc::new(WaveAsset::new(&decoded.samples)?);
        let asset_id = asset.id().to_owned();
        let mut candidate = self.patch.clone();
        candidate
            .assets
            .entry(asset_id.clone())
            .or_insert_with(|| asset.clone());
        install(&mut candidate, id, &asset_id, wave_points, filename)?;
        self.prune_displaced_assets(&mut candidate);
        let changed = candidate.reference_view() != self.patch.reference_view()
            || candidate.assets != self.patch.assets;
        if changed {
            self.commit(candidate, expected, None, &EditEffects::default())?;
        }
        Ok(json!({"revision":self.revision,"id":id,"changed":changed,
            "content":{"type":"wavetable","asset_id":asset_id,"wave_points":wave_points,"name":filename},
            "asset":asset.descriptor(),"source_channels":decoded.source_channels,
            "source_sample_rate":decoded.source_sample_rate,"source_samples_interleaved":true}))
    }

    pub(super) fn export_wavetable(&self, args: &Value) -> Result<Value, String> {
        self.check_revision(expected_revision(args)?)?;
        let id = string(args, "id")?;
        let (asset, wave_points, name) = selected(&self.patch, id)?;
        let filename = string(args, "filename")?;
        let extension = Path::new(filename)
            .extension()
            .and_then(|extension| extension.to_str())
            .ok_or("export filename must end with .wav or .f32")?;
        let path = self.file(filename, extension)?;
        let samples = asset.samples().collect::<Vec<_>>();
        let bytes = match extension.to_ascii_lowercase().as_str() {
            "wav" => encode_wavetable_wav(&samples, wave_points as usize)?,
            "f32" => samples
                .iter()
                .flat_map(|sample| sample.to_le_bytes())
                .collect(),
            _ => return Err("export filename must end with .wav or .f32".into()),
        };
        save_new(&path, &bytes)?;
        Ok(json!({"revision":self.revision,"id":id,"filename":filename,
            "name":name,"wave_points":wave_points,"bytes":bytes.len(),"asset":asset.descriptor()}))
    }

    pub(super) fn inspect_wavetable(&self, args: &Value) -> Result<Value, String> {
        self.check_revision(expected_revision(args)?)?;
        let id = string(args, "id")?;
        let (asset, wave_points, name) = selected(&self.patch, id)?;
        let offset = args
            .get("offset")
            .map(|value| {
                value
                    .as_u64()
                    .and_then(|value| usize::try_from(value).ok())
                    .ok_or("offset must be a nonnegative integer")
            })
            .transpose()?
            .unwrap_or(0);
        if offset > asset.sample_count() {
            return Err("offset exceeds wavetable sample count".into());
        }
        let mut result = json!({"revision":self.revision,"id":id,"name":name,
            "wave_points":wave_points,"wave_count":asset.sample_count()/wave_points as usize,
            "asset":asset.descriptor()});
        if let Some(limit) = args.get("limit") {
            let limit = limit
                .as_u64()
                .filter(|limit| (1..=256).contains(limit))
                .ok_or("limit must be an integer in 1..256")? as usize;
            let samples = asset.samples().skip(offset).take(limit).collect::<Vec<_>>();
            result["offset"] = json!(offset);
            result["samples"] = json!(samples);
        } else if args.get("offset").is_some() {
            return Err("offset requires an explicit sample limit".into());
        }
        if serde_json::to_vec(&result)
            .map_err(|error| error.to_string())?
            .len()
            > 8192
        {
            return Err("wavetable inspection exceeds 8192 bytes; request fewer samples".into());
        }
        Ok(result)
    }
}
