//! Native project persistence on control/UI threads. A project publishes graph
//! and rack layout together. Cooperative writers serialize through a stable
//! companion lock; an unrelated external editor can still race the final rename.
use crate::rack::Rack;
use openpatch_core::{catalog, Patch};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions, TryLockError},
    io::{Read, Write},
    path::{Path, PathBuf},
};
use tempfile::{Builder, NamedTempFile};

const MAX_DOCUMENT_BYTES: usize = 2 * 1_048_576;
const MAX_LEGACY_BYTES: usize = 1_048_576;
const RECOVERY_PREFIX: &str = ".openpatch-recovery-";
const EXTENSION: &str = ".openpatch";

#[derive(Clone, Debug)]
pub struct Document {
    pub patch: Patch,
    pub layout: Value,
}
impl Document {
    pub fn new(mut patch: Patch, layout: Value) -> Result<Self, String> {
        patch.assign_missing_cable_ids()?;
        let document = Self { patch, layout };
        document.encode()?;
        Ok(document)
    }
    pub fn encode(&self) -> Result<Vec<u8>, String> {
        self.patch.validate()?;
        let object = self.layout.as_object().ok_or("layout must be an object")?;
        if object.len() != 4
            || object
                .keys()
                .any(|key| !["version", "zoom", "pan", "positions"].contains(&key.as_str()))
        {
            return Err("layout accepts exactly version, zoom, pan, and positions".into());
        }
        Rack::default().restore_layout(&self.layout)?;
        let bytes = serde_json::to_vec_pretty(&json!({"format":"openpatch-project","version":1,"patch":self.patch,"layout":self.layout})).map_err(|e| e.to_string())?;
        if bytes.len() > MAX_DOCUMENT_BYTES {
            return Err("project exceeds 2 MiB".into());
        }
        Ok(bytes)
    }
    pub fn equivalent(&self, other: &Self) -> bool {
        match (
            serde_json::to_value(&self.patch),
            serde_json::to_value(&other.patch),
        ) {
            (Ok(a), Ok(b)) => a == b && self.layout == other.layout,
            _ => false,
        }
    }
}
#[derive(Clone, Debug)]
pub struct FileToken {
    name: String,
    bytes: Vec<u8>,
    /// Publication succeeded, but durability of the directory entry could not
    /// be confirmed. The new bytes/token are authoritative despite this warning.
    pub durability_warning: Option<String>,
}
#[derive(Clone, Debug)]
pub struct Loaded {
    pub document: Document,
    pub token: Option<FileToken>,
    pub imported: bool,
}
#[derive(Debug)]
pub struct Store {
    workspace: PathBuf,
    recovery_name: String,
    // Keep companion inodes stable and locked; never unlink or replace them.
    _recovery_lock: File,
    claimed: BTreeMap<String, File>,
}
impl Store {
    pub fn new(workspace: &Path) -> Result<Self, String> {
        fs::create_dir_all(workspace).map_err(|e| e.to_string())?;
        let workspace = workspace.canonicalize().map_err(|e| e.to_string())?;
        let mut claimed = BTreeMap::new();
        let entries = fs::read_dir(&workspace).map_err(|e| e.to_string())?;
        for entry in entries {
            let Ok(entry) = entry else {
                continue;
            };
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            if valid_recovery_name(&name).is_err() || regular(&entry.path()).is_err() {
                continue;
            }
            if let Ok(Some(lock)) = try_companion_lock(&workspace, &name) {
                claimed.insert(name, lock);
            }
        }
        // Reserve a random companion before any corresponding recovery document
        // becomes discoverable. Keeping the file leaves a stable lock inode.
        let reservation = Builder::new()
            .prefix(RECOVERY_PREFIX)
            .suffix(".openpatch.lock")
            .tempfile_in(&workspace)
            .map_err(|e| e.to_string())?;
        let (recovery_lock, lock_path) = reservation.keep().map_err(|e| e.error.to_string())?;
        recovery_lock.try_lock().map_err(|e| e.to_string())?;
        let lock_name = lock_path
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or("invalid recovery lock filename")?;
        let recovery_name = lock_name
            .strip_suffix(".lock")
            .ok_or("invalid recovery lock suffix")?
            .to_owned();
        Ok(Self {
            workspace,
            recovery_name,
            _recovery_lock: recovery_lock,
            claimed,
        })
    }
    pub fn load(&self, name: &str) -> Result<Loaded, String> {
        validate_name(name)?;
        if name.ends_with(EXTENSION) {
            let bytes = read_regular(&self.workspace.join(name), MAX_DOCUMENT_BYTES)?;
            let document = decode(&bytes)?;
            Ok(Loaded {
                document,
                token: Some(FileToken {
                    name: name.into(),
                    bytes,
                    durability_warning: None,
                }),
                imported: false,
            })
        } else if name.ends_with(".json") && !name.ends_with(".layout.json") {
            let bytes = read_regular(&self.workspace.join(name), MAX_LEGACY_BYTES)?;
            let patch: Patch =
                serde_json::from_slice(&bytes).map_err(|e| format!("invalid legacy patch: {e}"))?;
            patch.validate()?;
            let sidecar = self.workspace.join(format!("{name}.layout.json"));
            let layout = match fs::symlink_metadata(&sidecar) {
                Ok(_) => serde_json::from_slice(&read_regular(&sidecar, MAX_LEGACY_BYTES)?)
                    .map_err(|e| format!("invalid legacy layout: {e}"))?,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    let mut rack = Rack::default();
                    rack.reconcile(&patch, &catalog());
                    rack.layout()
                }
                Err(e) => return Err(e.to_string()),
            };
            Ok(Loaded {
                document: Document::new(patch, layout)?,
                token: None,
                imported: true,
            })
        } else {
            Err("project filename must end with .openpatch or .json".into())
        }
    }
    pub fn save_new(&self, name: &str, document: &Document) -> Result<FileToken, String> {
        native_name(name)?;
        let bytes = document.encode()?;
        let _lock = companion_lock(&self.workspace, name)?;
        publish(&self.workspace, name, bytes, false)
    }
    pub fn save_existing(
        &self,
        name: &str,
        document: &Document,
        token: &FileToken,
    ) -> Result<FileToken, String> {
        native_name(name)?;
        if token.name != name {
            return Err("save token belongs to a different project".into());
        }
        let bytes = document.encode()?;
        let _lock = companion_lock(&self.workspace, name)?;
        let current = read_regular(&self.workspace.join(name), MAX_DOCUMENT_BYTES)?;
        if current != token.bytes {
            return Err("project changed on disk; reopen it or save a new copy".into());
        }
        publish(&self.workspace, name, bytes, true)
    }
    pub fn save_recovery(&self, document: &Document) -> Result<(), String> {
        let bytes = document.encode()?;
        let path = self.workspace.join(&self.recovery_name);
        let replace = match fs::symlink_metadata(&path) {
            Ok(_) => {
                regular(&path)?;
                true
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
            Err(e) => return Err(e.to_string()),
        };
        let token = publish(&self.workspace, &self.recovery_name, bytes, replace)?;
        if let Some(warning) = token.durability_warning {
            eprintln!("Recovery published: {warning}");
        }
        Ok(())
    }
    pub fn clear_recovery(&self) -> Result<(), String> {
        remove_regular(&self.workspace.join(&self.recovery_name))
    }
    /// Only orphan documents claimed at startup, never live instances or files
    /// appearing after this Store was constructed. Corrupt candidates stay listed.
    pub fn recoveries(&self) -> Vec<String> {
        self.claimed
            .keys()
            .filter(|name| regular(&self.workspace.join(name)).is_ok())
            .cloned()
            .collect()
    }
    pub fn load_recovery(&self, name: &str) -> Result<Loaded, String> {
        self.check_claim(name)?;
        let document = decode(&read_regular(
            &self.workspace.join(name),
            MAX_DOCUMENT_BYTES,
        )?)?;
        Ok(Loaded {
            document,
            token: None,
            imported: false,
        })
    }
    pub fn discard_recovery(&self, name: &str) -> Result<(), String> {
        self.check_claim(name)?;
        remove_regular(&self.workspace.join(name))
    }
    fn check_claim(&self, name: &str) -> Result<(), String> {
        valid_recovery_name(name)?;
        if !self.claimed.contains_key(name) {
            return Err("recovery is not owned by this session".into());
        }
        Ok(())
    }
}
fn decode(bytes: &[u8]) -> Result<Document, String> {
    let value: Value =
        serde_json::from_slice(bytes).map_err(|e| format!("invalid project: {e}"))?;
    let object = value.as_object().ok_or("project must be an object")?;
    if object.len() != 4
        || object
            .keys()
            .any(|key| !["format", "version", "patch", "layout"].contains(&key.as_str()))
    {
        return Err("project accepts exactly format, version, patch, and layout".into());
    }
    if value["format"] != "openpatch-project" || value["version"] != 1 {
        return Err("unsupported project format or version".into());
    }
    let patch = serde_json::from_value(value["patch"].clone())
        .map_err(|e| format!("invalid project patch: {e}"))?;
    Document::new(patch, value["layout"].clone())
}
fn validate_name(name: &str) -> Result<(), String> {
    if name.is_empty()
        || name.len() > 120
        || name.starts_with('.')
        || !name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
    {
        return Err("filename must be a simple ASCII name of at most 120 bytes; no directories or hidden files".into());
    }
    Ok(())
}
fn native_name(name: &str) -> Result<(), String> {
    validate_name(name)?;
    if !name.ends_with(EXTENSION) {
        return Err("native project filename must end with .openpatch".into());
    }
    Ok(())
}
fn valid_recovery_name(name: &str) -> Result<(), String> {
    if !name.starts_with(RECOVERY_PREFIX)
        || !name.ends_with(EXTENSION)
        || name.len() > 120
        || !name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
        || name.len() <= RECOVERY_PREFIX.len() + EXTENSION.len()
    {
        return Err("invalid recovery filename".into());
    }
    Ok(())
}
fn regular(path: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err("file must be a regular file, not a symlink".into());
    }
    Ok(())
}
fn read_regular(path: &Path, limit: usize) -> Result<Vec<u8>, String> {
    regular(path)?;
    let file = File::open(path).map_err(|e| e.to_string())?;
    if !file.metadata().map_err(|e| e.to_string())?.is_file() {
        return Err("file must be regular".into());
    }
    let mut bytes = Vec::new();
    file.take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > limit {
        return Err(format!("file exceeds {} MiB", limit / 1_048_576));
    }
    Ok(bytes)
}
fn remove_regular(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(_) => {
            regular(path)?;
            fs::remove_file(path).map_err(|e| e.to_string())
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}
fn open_companion(workspace: &Path, name: &str) -> Result<File, String> {
    let path = workspace.join(format!("{name}.lock"));
    match fs::symlink_metadata(&path) {
        Ok(_) => regular(&path)?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
        Err(e) => return Err(e.to_string()),
    }
    OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
        .map_err(|e| e.to_string())
}
fn try_companion_lock(workspace: &Path, name: &str) -> Result<Option<File>, String> {
    let file = open_companion(workspace, name)?;
    match file.try_lock() {
        Ok(()) => Ok(Some(file)),
        Err(TryLockError::WouldBlock) => Ok(None),
        Err(TryLockError::Error(e)) => Err(e.to_string()),
    }
}
fn companion_lock(workspace: &Path, name: &str) -> Result<File, String> {
    try_companion_lock(workspace, name)?
        .ok_or_else(|| "project is being written by another session; try again".into())
}
fn publish(
    workspace: &Path,
    name: &str,
    bytes: Vec<u8>,
    replace: bool,
) -> Result<FileToken, String> {
    let target = workspace.join(name);
    if replace {
        regular(&target)?;
    }
    let mut temporary: NamedTempFile = Builder::new()
        .prefix(".openpatch-write-")
        .suffix(".tmp")
        .tempfile_in(workspace)
        .map_err(|e| e.to_string())?;
    temporary.write_all(&bytes).map_err(|e| e.to_string())?;
    temporary.as_file().sync_all().map_err(|e| e.to_string())?;
    before_publish()?;
    if replace {
        temporary
            .persist(&target)
            .map_err(|e| e.error.to_string())?;
    } else {
        temporary
            .persist_noclobber(&target)
            .map_err(|e| e.error.to_string())?;
    }
    let durability_warning = sync_directory(workspace).err().map(|e| {
        format!("saved file is published, but directory durability could not be confirmed: {e}")
    });
    Ok(FileToken {
        name: name.into(),
        bytes,
        durability_warning,
    })
}
fn before_publish() -> Result<(), String> {
    #[cfg(test)]
    {
        let callback = BEFORE_NEXT_PUBLISH.with(|slot| slot.borrow_mut().take());
        if let Some(callback) = callback {
            callback();
        }
    }
    #[cfg(test)]
    if FAIL_BEFORE_PUBLISH.with(|flag| flag.replace(false)) {
        return Err("injected failure before project publication".into());
    }
    Ok(())
}
fn sync_directory(path: &Path) -> std::io::Result<()> {
    #[cfg(test)]
    if FAIL_DIRECTORY_SYNC.with(|flag| flag.replace(false)) {
        return Err(std::io::Error::other("injected directory sync failure"));
    }
    #[cfg(unix)]
    {
        File::open(path)?.sync_all()
    }
    // Directory handles require platform-specific flags on Windows. Publication
    // is still atomic; this explicitly reports the weaker durability guarantee.
    #[cfg(not(unix))]
    {
        let _ = path;
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "directory sync unavailable on this platform",
        ))
    }
}
/// Run once after staging and syncing bytes, immediately before publication.
/// This deterministic test seam never runs in production builds.
#[cfg(test)]
pub(crate) fn before_next_publish(callback: impl FnOnce() + 'static) {
    BEFORE_NEXT_PUBLISH.with(|slot| {
        *slot.borrow_mut() = Some(Box::new(callback));
    });
}

#[cfg(test)]
thread_local! {
    static BEFORE_NEXT_PUBLISH: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const { std::cell::RefCell::new(None) };
    static FAIL_BEFORE_PUBLISH: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static FAIL_DIRECTORY_SYNC: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[cfg(test)]
mod tests {
    use super::*;
    fn document(value: f32) -> Document {
        let mut patch = Patch::new_v2();
        patch.nodes.push(openpatch_core::Node {
            bypassed: false,
            id: "source".into(),
            kind: "constant".into(),
            params: BTreeMap::from([("value".into(), value)]),
        });
        patch.output = Some("source".into());
        let mut rack = Rack::default();
        rack.reconcile(&patch, &catalog());
        Document::new(patch, rack.layout()).unwrap()
    }
    #[test]
    fn failed_prepublication_save_and_recovery_leave_previous_complete_bytes() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::new(directory.path()).unwrap();
        let original = document(1.0);
        let replacement = document(2.0);
        let token = store.save_new("patch.openpatch", &original).unwrap();
        FAIL_BEFORE_PUBLISH.with(|flag| flag.set(true));
        assert!(store
            .save_existing("patch.openpatch", &replacement, &token)
            .is_err());
        assert_eq!(
            fs::read(directory.path().join("patch.openpatch")).unwrap(),
            token.bytes
        );
        assert!(store
            .load("patch.openpatch")
            .unwrap()
            .document
            .equivalent(&original));
        store.save_recovery(&original).unwrap();
        let recovery_path = directory.path().join(&store.recovery_name);
        let before = fs::read(&recovery_path).unwrap();
        FAIL_BEFORE_PUBLISH.with(|flag| flag.set(true));
        assert!(store.save_recovery(&replacement).is_err());
        assert_eq!(fs::read(&recovery_path).unwrap(), before);
        // A staged but unpublished file left by process death is never mistaken
        // for either a project or a recovery candidate.
        fs::write(
            directory.path().join(".openpatch-write-crash.tmp"),
            b"partial",
        )
        .unwrap();
        drop(store);
        let restarted = Store::new(directory.path()).unwrap();
        assert_eq!(restarted.recoveries().len(), 1);
        assert!(restarted
            .load_recovery(&restarted.recoveries()[0])
            .unwrap()
            .document
            .equivalent(&original));
    }
    #[test]
    fn directory_sync_failure_returns_published_token_and_warning() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::new(directory.path()).unwrap();
        let token = store.save_new("patch.openpatch", &document(1.0)).unwrap();
        FAIL_DIRECTORY_SYNC.with(|flag| flag.set(true));
        let saved = store
            .save_existing("patch.openpatch", &document(2.0), &token)
            .unwrap();
        assert!(saved.durability_warning.is_some());
        assert_eq!(
            fs::read(directory.path().join("patch.openpatch")).unwrap(),
            saved.bytes
        );
        assert!(store
            .save_existing("patch.openpatch", &document(3.0), &token)
            .is_err());
        assert!(store
            .save_existing("patch.openpatch", &document(3.0), &saved)
            .is_ok());
    }
    #[test]
    fn stable_companion_serializes_cooperating_writers_and_conflicts_preserve_first_save() {
        let directory = tempfile::tempdir().unwrap();
        let a = Store::new(directory.path()).unwrap();
        let b = Store::new(directory.path()).unwrap();
        let original = a.save_new("patch.openpatch", &document(1.0)).unwrap();
        let stale = b.load("patch.openpatch").unwrap().token.unwrap();
        let lock = companion_lock(directory.path(), "patch.openpatch").unwrap();
        assert!(b
            .save_existing("patch.openpatch", &document(2.0), &stale)
            .is_err());
        assert_eq!(
            fs::read(directory.path().join("patch.openpatch")).unwrap(),
            original.bytes
        );
        drop(lock);
        let first = a
            .save_existing("patch.openpatch", &document(3.0), &original)
            .unwrap();
        assert!(b
            .save_existing("patch.openpatch", &document(4.0), &stale)
            .is_err());
        assert_eq!(
            fs::read(directory.path().join("patch.openpatch")).unwrap(),
            first.bytes
        );
    }
    #[test]
    fn recovery_claim_is_exclusive_and_invalid_candidate_is_not_removed() {
        let directory = tempfile::tempdir().unwrap();
        let owner = Store::new(directory.path()).unwrap();
        owner.save_recovery(&document(1.0)).unwrap();
        let name = owner.recovery_name.clone();
        let observer = Store::new(directory.path()).unwrap();
        assert!(observer.recoveries().is_empty());
        assert!(observer.load_recovery(&name).is_err());
        assert!(observer.discard_recovery(&name).is_err());
        drop(owner);
        let first = Store::new(directory.path()).unwrap();
        assert_eq!(first.recoveries(), vec![name.clone()]);
        let second = Store::new(directory.path()).unwrap();
        assert!(second.recoveries().is_empty());
        assert!(second.discard_recovery(&name).is_err());
        fs::write(directory.path().join(&name), b"broken project").unwrap();
        assert!(first.load_recovery(&name).is_err());
        assert!(directory.path().join(&name).exists());
        first.discard_recovery(&name).unwrap();
        assert!(!directory.path().join(&name).exists());
        assert!(directory.path().join(format!("{name}.lock")).exists());
    }
    #[test]
    fn malformed_or_oversized_documents_never_publish() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::new(directory.path()).unwrap();
        let mut bad = document(1.0);
        bad.layout["extra"] = json!(true);
        assert!(store.save_new("bad.openpatch", &bad).is_err());
        assert!(!directory.path().join("bad.openpatch").exists());
        assert!(!directory.path().join("bad.openpatch.lock").exists());
        let oversized = vec![b' '; MAX_DOCUMENT_BYTES + 1];
        fs::write(directory.path().join("large.openpatch"), oversized).unwrap();
        assert!(store.load("large.openpatch").unwrap_err().contains("2 MiB"));
        for value in [
            json!({"format":"openpatch-project","version":2,"patch":{},"layout":{}}),
            json!({"format":"openpatch-project","version":1,"patch":{},"layout":{},"extra":1}),
        ] {
            fs::write(
                directory.path().join("bad.openpatch"),
                serde_json::to_vec(&value).unwrap(),
            )
            .unwrap();
            assert!(store.load("bad.openpatch").is_err());
        }
    }
    #[test]
    fn legacy_import_is_read_only_and_absent_layout_is_reconciled() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::new(directory.path()).unwrap();
        let original = document(1.0);
        let path = directory.path().join("legacy.json");
        let bytes = serde_json::to_vec(&original.patch).unwrap();
        fs::write(&path, &bytes).unwrap();
        let loaded = store.load("legacy.json").unwrap();
        assert!(loaded.imported);
        assert!(loaded.token.is_none());
        assert!(loaded.document.layout["positions"]["source"].is_array());
        assert_eq!(fs::read(&path).unwrap(), bytes);
        let sidecar = directory.path().join("legacy.json.layout.json");
        fs::write(&sidecar, b"not JSON").unwrap();
        assert!(store.load("legacy.json").is_err());
        assert_eq!(fs::read(&path).unwrap(), bytes);
        assert_eq!(fs::read(&sidecar).unwrap(), b"not JSON");
        fs::write(&sidecar, vec![b' '; MAX_LEGACY_BYTES + 1]).unwrap();
        assert!(store.load("legacy.json").is_err());
    }
    #[cfg(unix)]
    #[test]
    fn symlink_targets_sidecars_and_companion_locks_are_rejected() {
        use std::os::unix::fs::symlink;
        let directory = tempfile::tempdir().unwrap();
        let store = Store::new(directory.path()).unwrap();
        let original = document(1.0);
        let token = store.save_new("real.openpatch", &original).unwrap();
        symlink(
            directory.path().join("real.openpatch"),
            directory.path().join("link.openpatch"),
        )
        .unwrap();
        assert!(store.load("link.openpatch").is_err());
        assert!(store.save_new("link.openpatch", &document(2.0)).is_err());
        assert_eq!(
            fs::read(directory.path().join("real.openpatch")).unwrap(),
            token.bytes
        );
        let bytes = serde_json::to_vec(&original.patch).unwrap();
        fs::write(directory.path().join("legacy.json"), bytes).unwrap();
        symlink(
            directory.path().join("real.openpatch"),
            directory.path().join("legacy.json.layout.json"),
        )
        .unwrap();
        assert!(store.load("legacy.json").is_err());
        symlink(
            directory.path().join("real.openpatch"),
            directory.path().join("guarded.openpatch.lock"),
        )
        .unwrap();
        assert!(store.save_new("guarded.openpatch", &original).is_err());
        assert!(!directory.path().join("guarded.openpatch").exists());
    }
}
