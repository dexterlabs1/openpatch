//! Native project persistence on control/UI threads. A project publishes graph
//! and rack layout together. Cooperative writers serialize through a stable
//! companion lock; an unrelated external editor can still race the final rename.
use crate::rack::Rack;
use openpatch::project_document::{ProjectDocument, ProjectLayout};
use openpatch_core::{catalog, Patch, MAX_PATCH_FILE_BYTES};
#[cfg(test)]
use serde_json::json;
use serde_json::Value;
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions, TryLockError},
    io::{Read, Write},
    path::{Path, PathBuf},
};
use tempfile::{Builder, NamedTempFile};

const MAX_DOCUMENT_BYTES: usize = MAX_PATCH_FILE_BYTES;
const MAX_LEGACY_LAYOUT_BYTES: usize = 1_048_576;
const MAX_VCV_INPUT_BYTES: usize = 16 * 1_048_576;
const RECOVERY_PREFIX: &str = ".openpatch-recovery-";
const EXTENSION: &str = ".openpatch";
const TEMPLATE_NAME: &str = ".openpatch-template.openpatch";

/// The portable document is authoritative. Cached native/layout views are
/// derived together after validation and cannot be edited independently.
#[derive(Clone, Debug)]
pub struct Document {
    project: ProjectDocument,
    patch: Patch,
    layout: Value,
}
impl Document {
    pub fn new(patch: Patch, layout: Value) -> Result<Self, String> {
        let layout: ProjectLayout =
            serde_json::from_value(layout).map_err(|e| format!("invalid project layout: {e}"))?;
        Self::from_project(ProjectDocument::from_patch(patch, layout)?)
    }
    pub fn from_project(project: ProjectDocument) -> Result<Self, String> {
        // A state carrier is editable native state, never implicit permission to
        // play the supported subset of an unresolved document.
        let patch = project.state_carrier()?.patch;
        let layout = serde_json::to_value(&project.layout).map_err(|e| e.to_string())?;
        Rack::default().restore_layout(&layout)?;
        project.encode()?;
        Ok(Self {
            project,
            patch,
            layout,
        })
    }
    pub fn project(&self) -> &ProjectDocument {
        &self.project
    }
    pub fn patch(&self) -> &Patch {
        &self.patch
    }
    pub fn layout(&self) -> &Value {
        &self.layout
    }
    pub fn with_layout(self, layout: Value) -> Result<Self, String> {
        let mut project = self.project;
        project.layout =
            serde_json::from_value(layout).map_err(|e| format!("invalid project layout: {e}"))?;
        Self::from_project(project)
    }
    pub fn encode(&self) -> Result<Vec<u8>, String> {
        self.project.encode()
    }
    pub fn equivalent(&self, other: &Self) -> bool {
        // Validated source package/blob keys and native asset IDs are content
        // digests, so this includes opaque payloads without expanding them.
        self.project.reference_view() == other.project.reference_view()
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
            let bytes = read_regular(&self.workspace.join(name), MAX_DOCUMENT_BYTES)?;
            let value: Value =
                serde_json::from_slice(&bytes).map_err(|e| format!("invalid legacy patch: {e}"))?;
            if value.get("format").is_some() {
                let document = decode(&bytes)?;
                let document = if !document.project().sources.is_empty()
                    || !document.project().source_blobs.is_empty()
                {
                    fit_imported_source_layout(document)?
                } else {
                    document
                };
                return Ok(Loaded {
                    document,
                    token: None,
                    imported: true,
                });
            }
            let patch: Patch =
                serde_json::from_value(value).map_err(|e| format!("invalid legacy patch: {e}"))?;
            patch.validate()?;
            let sidecar = self.workspace.join(format!("{name}.layout.json"));
            let layout = match fs::symlink_metadata(&sidecar) {
                Ok(_) => serde_json::from_slice(&read_regular(&sidecar, MAX_LEGACY_LAYOUT_BYTES)?)
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
        } else if name.ends_with(".vcv") {
            let bytes = read_regular(&self.workspace.join(name), MAX_VCV_INPUT_BYTES)?;
            let imported = openpatch::vcv_import::import(&bytes)?;
            Ok(Loaded {
                document: fit_imported_source_layout(Document::from_project(imported.document)?)?,
                token: None,
                imported: true,
            })
        } else {
            Err("project filename must end with .openpatch, .json, or .vcv".into())
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
    /// Templates use one dedicated hidden name and the same validated document
    /// format as projects. Missing is distinct from corrupt or unsafe files.
    pub fn load_template(&self) -> Result<Option<Document>, String> {
        self.template_token()?
            .map(|token| decode(&token.bytes))
            .transpose()
    }

    /// Capture bounded raw bytes for an explicit replacement review, including
    /// corrupt JSON. Decoding is deliberately not required to replace a template.
    pub fn template_token(&self) -> Result<Option<FileToken>, String> {
        let path = self.workspace.join(TEMPLATE_NAME);
        match fs::symlink_metadata(&path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error.to_string()),
            Ok(_) => Ok(Some(FileToken {
                name: TEMPLATE_NAME.into(),
                bytes: read_regular(&path, MAX_DOCUMENT_BYTES)?,
                durability_warning: None,
            })),
        }
    }

    /// Publish only the reviewed template version (or a reviewed absence).
    /// Cooperative writers share a stable companion lock; missing publication
    /// also uses no-clobber against a late creation by an external writer.
    pub fn save_template(
        &self,
        document: &Document,
        reviewed: Option<&FileToken>,
    ) -> Result<FileToken, String> {
        if reviewed.is_some_and(|token| token.name != TEMPLATE_NAME) {
            return Err("save token belongs to a different project, not the template".into());
        }
        let bytes = document.encode()?;
        let _lock = companion_lock(&self.workspace, TEMPLATE_NAME)?;
        let current = self.template_token()?;
        let replace = match (reviewed, current.as_ref()) {
            (None, None) => false,
            (Some(reviewed), Some(current)) if reviewed.bytes == current.bytes => true,
            _ => return Err("template changed on disk; review it again before saving".into()),
        };
        publish(&self.workspace, TEMPLATE_NAME, bytes, replace)
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
/// A one-time adapter at external import boundaries. Native project loads use
/// their authored layout verbatim, including deliberate overlap or close spacing.
fn fit_imported_source_layout(document: Document) -> Result<Document, String> {
    let mut rack = Rack::default();
    rack.restore_layout(document.layout())?;
    rack.reconcile_document(document.patch(), document.project(), &catalog());
    rack.reflow_imported_source_layout(document.project());
    document.with_layout(rack.layout())
}

fn decode(bytes: &[u8]) -> Result<Document, String> {
    #[derive(serde::Deserialize)]
    struct Header {
        format: Option<String>,
    }
    let header: Header =
        serde_json::from_slice(bytes).map_err(|e| format!("invalid project: {e}"))?;
    if header.format.as_deref() != Some("openpatch-project") {
        return Err("native project format marker is missing".into());
    }
    Document::from_project(ProjectDocument::decode(bytes)?)
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
    let metadata = file.metadata().map_err(|e| e.to_string())?;
    if !metadata.is_file() {
        return Err("file must be regular".into());
    }
    if metadata.len() > limit as u64 {
        return Err(format!("file exceeds {} MiB", limit / 1_048_576));
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
pub(crate) fn fail_next_publish() {
    FAIL_BEFORE_PUBLISH.with(|flag| flag.set(true));
}

#[cfg(test)]
thread_local! {
    static BEFORE_NEXT_PUBLISH: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const { std::cell::RefCell::new(None) };
    static FAIL_BEFORE_PUBLISH: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static FAIL_DIRECTORY_SYNC: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// A complete opaque module, route and asset fixture shared by storage and
/// editor acceptance tests. All records pass the public shared validation.
#[cfg(test)]
pub(crate) fn test_source_document(payload: Value, asset_bytes: &[u8]) -> Document {
    use openpatch::project_document::{
        DocumentCable, DocumentNode, ObservedPort, PortDirection, SourceBlob, SourceCable,
        SourceModule, SourcePackage,
    };
    use std::{collections::BTreeSet, sync::Arc};
    let patch: Patch = serde_json::from_value(json!({
        "version":2, "nodes":[{"id":"amp","kind":"gain","params":{"gain":0.5}}],
        "output":"amp"
    }))
    .unwrap();
    let mut project = ProjectDocument::from_patch(patch, ProjectLayout::default()).unwrap();
    let blob = SourceBlob::new(asset_bytes.to_vec()).unwrap();
    let blob_id = blob.id();
    let package = Arc::new(SourcePackage {
        patch: Arc::new(json!({
            "version":"2.6.4", "global_unknown":{"retained":true},
            "modules":[{"id":42,"plugin":"MissingVendor","model":"OpaqueVoice",
                "version":"1.2.3","pos":[0,0],"data":payload}],
            "cables":[{"id":9,"outputModuleId":42,"outputId":0,
                "inputModuleId":7,"inputId":0,"unknown_route_state":{"retained":true}}]
        })),
        assets: BTreeMap::from([("res/opaque.bin".into(), blob_id.clone())]),
        missing_assets: BTreeSet::from(["res/unavailable.wav".into()]),
    });
    let package_id = package.id().unwrap();
    project.sources.insert(package_id.clone(), package);
    project.source_blobs.insert(blob_id, blob);
    project.graph.nodes.push(DocumentNode::Unresolved {
        id: "unknown".into(),
        source: SourceModule {
            package: package_id.clone(),
            index: 0,
            mapping: None,
        },
        ports: vec![ObservedPort {
            direction: PortDirection::Output,
            index: 0,
        }],
    });
    project.graph.cables.push(DocumentCable {
        route: serde_json::from_value(json!({
            "id":"opaque-route", "from":"unknown", "output":"vcv_out_0",
            "to":"amp", "input":"signal", "color":[12,34,56], "input_order":8
        }))
        .unwrap(),
        source: Some(SourceCable {
            package: package_id,
            index: 0,
        }),
    });
    project.layout.positions =
        BTreeMap::from([("amp".into(), [180.0, 0.0]), ("unknown".into(), [0.0, 0.0])]);
    Document::from_project(project).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn document(value: f32) -> Document {
        let mut patch = Patch::new_v2();
        patch.nodes.push(openpatch_core::Node {
            content: None,
            rackmcp_sequence: None,
            score: None,
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
    fn raw_rack_import_fits_native_widths_and_keeps_original_source_rows_and_coordinates() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::new(directory.path()).unwrap();
        let source = json!({"version":"2.6.4","modules":[
            {"id":1,"plugin":"Fundamental","model":"VCMixer","version":"2.6.4","pos":[0,0]},
            {"id":2,"plugin":"MissingVendor","model":"Unknown","version":"1","pos":[1,0]},
            {"id":3,"plugin":"MissingVendor","model":"Unknown","version":"1","pos":[3,1]}
        ],"cables":[]});
        let bytes = serde_json::to_vec(&source).unwrap();
        fs::write(directory.path().join("overlap.vcv"), &bytes).unwrap();
        let loaded = store.load("overlap.vcv").unwrap();
        assert!(loaded.imported && loaded.token.is_none());
        let document = &loaded.document;
        let native = &document.patch().nodes[0];
        let node_at = |index| {
            document
                .project()
                .graph
                .nodes
                .iter()
                .find(|node| node.source().is_some_and(|source| source.index == index))
                .unwrap()
        };
        let positions = &document.project().layout.positions;
        assert_eq!(positions[&native.id], [0.0, 0.0]);
        assert_eq!(
            positions[node_at(1).id()],
            [crate::rack::module_width(native) + 15.0, 0.0]
        );
        assert_eq!(positions[node_at(2).id()], [45.0, 420.0]);
        assert_eq!(
            document
                .project()
                .sources
                .values()
                .next()
                .unwrap()
                .patch
                .as_ref(),
            &source
        );
        assert_eq!(
            fs::read(directory.path().join("overlap.vcv")).unwrap(),
            bytes
        );
        assert!(document.equivalent(&store.load("overlap.vcv").unwrap().document));
        store.save_new("fitted.openpatch", document).unwrap();
        assert_eq!(
            store
                .load("fitted.openpatch")
                .unwrap()
                .document
                .encode()
                .unwrap(),
            document.encode().unwrap()
        );
    }

    #[test]
    fn portable_json_import_fits_source_panels_but_native_project_layouts_remain_authored() {
        use openpatch::project_document::DocumentNode;
        let directory = tempfile::tempdir().unwrap();
        let store = Store::new(directory.path()).unwrap();
        let original = test_source_document(json!({"opaque":true}), &[1, 7, 9]);
        let mut project = original.project().clone();
        let wide: openpatch_core::Node = serde_json::from_value(json!({
            "id":"wide", "kind":"stock_scope"
        }))
        .unwrap();
        let width = crate::rack::module_width(&wide);
        project.graph.nodes.push(DocumentNode::Native {
            node: wide,
            source: None,
        });
        project.layout.positions.insert("wide".into(), [0.0, 0.0]);
        project
            .layout
            .positions
            .insert("unknown".into(), [15.0, 0.0]);
        project.layout.positions.insert("amp".into(), [30.0, 0.0]);
        let original = Document::from_project(project).unwrap();
        let bytes = original.encode().unwrap();
        fs::write(directory.path().join("portable.json"), &bytes).unwrap();
        fs::write(
            directory.path().join("portable.json.layout.json"),
            b"ignored native sidecar",
        )
        .unwrap();
        let loaded = store.load("portable.json").unwrap();
        assert!(loaded.imported && loaded.token.is_none());
        let positions = &loaded.document.project().layout.positions;
        assert_eq!(positions["wide"], [0.0, 0.0]);
        assert_eq!(positions["unknown"], [width + 15.0, 0.0]);
        assert_eq!(positions["amp"], [width + 15.0 + 180.0 + 15.0, 0.0]);
        assert_eq!(
            serde_json::to_value(&loaded.document.project().sources).unwrap(),
            serde_json::to_value(&original.project().sources).unwrap()
        );
        assert_eq!(
            serde_json::to_value(&loaded.document.project().source_blobs).unwrap(),
            serde_json::to_value(&original.project().source_blobs).unwrap()
        );
        assert_eq!(
            fs::read(directory.path().join("portable.json")).unwrap(),
            bytes
        );
        assert!(loaded
            .document
            .equivalent(&store.load("portable.json").unwrap().document));

        // The same authored overlap in a saved native project remains exact.
        store.save_new("authored.openpatch", &original).unwrap();
        assert_eq!(
            store
                .load("authored.openpatch")
                .unwrap()
                .document
                .encode()
                .unwrap(),
            bytes
        );
        store
            .save_new("fitted.openpatch", &loaded.document)
            .unwrap();
        assert!(loaded
            .document
            .equivalent(&store.load("fitted.openpatch").unwrap().document));

        // Portable native-only JSON is recognized without consulting a legacy
        // sidecar and keeps its layout, just like its .openpatch counterpart.
        let native = document(1.0);
        fs::write(
            directory.path().join("native.json"),
            native.encode().unwrap(),
        )
        .unwrap();
        fs::write(
            directory.path().join("native.json.layout.json"),
            b"ignored native sidecar",
        )
        .unwrap();
        let loaded = store.load("native.json").unwrap();
        assert!(loaded.imported && loaded.token.is_none());
        assert!(native.equivalent(&loaded.document));
    }

    #[test]
    fn opaque_payload_routes_and_assets_round_trip_save_template_and_recovery() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::new(directory.path()).unwrap();
        let original = test_source_document(json!({"opaque":[1,{"arbitrary":true}]}), &[0, 255, 7]);
        assert_eq!(original.patch().nodes.len(), 1);
        assert!(original.patch().cables.is_empty());
        assert!(original
            .project()
            .project(openpatch::project_document::PlaybackMode::Exact)
            .is_err());
        let token = store.save_new("opaque.openpatch", &original).unwrap();
        let loaded = store.load("opaque.openpatch").unwrap();
        assert!(original.equivalent(&loaded.document));
        assert_eq!(
            loaded.document.encode().unwrap(),
            original.encode().unwrap()
        );
        assert!(!loaded.imported && loaded.token.is_some());

        // Both opaque state and same-length asset changes leave the carrier
        // identical, but must still change saved content/equivalence.
        let revised = test_source_document(json!({"opaque":[2,{"arbitrary":true}]}), &[0, 254, 7]);
        assert_eq!(
            original.patch().reference_view(),
            revised.patch().reference_view()
        );
        assert!(!original.equivalent(&revised));
        store
            .save_existing("opaque.openpatch", &revised, &token)
            .unwrap();
        assert!(revised.equivalent(&store.load("opaque.openpatch").unwrap().document));
        store.save_template(&revised, None).unwrap();
        assert!(revised.equivalent(&store.load_template().unwrap().unwrap()));
        store.save_recovery(&revised).unwrap();
        let recovery = store.recovery_name.clone();
        drop(store);
        let restarted = Store::new(directory.path()).unwrap();
        assert_eq!(restarted.recoveries(), vec![recovery.clone()]);
        let recovered = restarted.load_recovery(&recovery).unwrap().document;
        assert_eq!(recovered.encode().unwrap(), revised.encode().unwrap());
        assert_eq!(
            recovered
                .project()
                .source_blobs
                .values()
                .next()
                .unwrap()
                .bytes(),
            &[0, 254, 7]
        );
        assert!(recovered.project().native_patch_for_legacy_api().is_err());
        assert!(revised.equivalent(&restarted.load_template().unwrap().unwrap()));
    }

    #[test]
    fn maximum_wave_asset_round_trips_native_overwrite_template_recovery_and_legacy() {
        use openpatch_core::{wavetable_file::MAX_WAVETABLE_SAMPLES, WaveAsset};
        use std::sync::Arc;

        assert_eq!(MAX_WAVETABLE_SAMPLES, 524_288);
        let mut samples = vec![0.25; MAX_WAVETABLE_SAMPLES];
        samples[0] = -0.0;
        samples[MAX_WAVETABLE_SAMPLES - 1] = -0.75;
        let asset = Arc::new(WaveAsset::new(&samples).unwrap());
        let id = asset.id().to_owned();
        let mut original = document(1.0);
        let mut patch = original.patch().clone();
        patch.assets.insert(id.clone(), asset.clone());
        original = Document::new(patch, original.layout().clone()).unwrap();
        let encoded = original.encode().unwrap();
        assert!(
            encoded.len() > 2 * 1_048_576,
            "The maximum asset must exceed the old native-project limit"
        );
        assert!(encoded.len() < MAX_DOCUMENT_BYTES);
        let envelope: Value = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(envelope["format"], "openpatch-project");
        assert_eq!(envelope["version"], 2);
        assert!(envelope["graph"]["assets"][&id]["data"].is_string());
        drop(envelope);
        drop(encoded);

        let verify = |loaded: &Document, expected: &Document| {
            assert!(loaded.equivalent(expected));
            let decoded = &loaded.patch().assets[&id];
            assert_eq!(decoded.id(), id);
            assert_eq!(decoded.sample_count(), MAX_WAVETABLE_SAMPLES);
            assert_eq!(decoded.decoded_bytes(), 2 * 1_048_576);
            assert_eq!(decoded.sample(0).unwrap().to_bits(), (-0.0_f32).to_bits());
            assert_eq!(decoded.sample(MAX_WAVETABLE_SAMPLES - 1), Some(-0.75));
            assert_eq!(decoded.as_ref(), asset.as_ref());
        };
        let directory = tempfile::tempdir().unwrap();
        let store = Store::new(directory.path()).unwrap();
        let token = store.save_new("wave.openpatch", &original).unwrap();
        verify(&store.load("wave.openpatch").unwrap().document, &original);

        let mut revised = original.clone();
        let mut patch = revised.patch().clone();
        patch.nodes[0].params.insert("value".into(), 2.0);
        revised = Document::new(patch, revised.layout().clone()).unwrap();
        assert!(Arc::ptr_eq(
            &original.patch().assets[&id],
            &revised.patch().assets[&id]
        ));
        store
            .save_existing("wave.openpatch", &revised, &token)
            .unwrap();
        verify(&store.load("wave.openpatch").unwrap().document, &revised);
        store.save_template(&revised, None).unwrap();
        verify(&store.load_template().unwrap().unwrap(), &revised);
        store.save_recovery(&revised).unwrap();
        let recovery = store.recovery_name.clone();

        let legacy = serde_json::to_vec(revised.patch()).unwrap();
        assert!(
            legacy.len() > 2 * 1_048_576,
            "Legacy patch JSON needs the expanded envelope too"
        );
        let legacy_path = directory.path().join("wave.json");
        fs::write(&legacy_path, &legacy).unwrap();
        fs::write(
            directory.path().join("wave.json.layout.json"),
            serde_json::to_vec(revised.layout()).unwrap(),
        )
        .unwrap();
        let imported = store.load("wave.json").unwrap();
        assert!(imported.imported);
        assert!(imported.token.is_none());
        verify(&imported.document, &revised);
        assert_eq!(fs::read(&legacy_path).unwrap(), legacy);

        drop(store);
        let restarted = Store::new(directory.path()).unwrap();
        assert_eq!(restarted.recoveries(), vec![recovery.clone()]);
        verify(
            &restarted.load_recovery(&recovery).unwrap().document,
            &revised,
        );
        verify(&restarted.load_template().unwrap().unwrap(), &revised);
    }

    #[test]
    fn document_equivalence_distinguishes_asset_content_graph_and_layout_without_payload_views() {
        use openpatch_core::WaveAsset;
        use std::sync::Arc;

        let asset = Arc::new(WaveAsset::new(&[0.0, 0.25, -0.5, 1.0]).unwrap());
        let mut original = document(1.0);
        let mut patch = original.patch().clone();
        patch.assets.insert(asset.id().into(), asset.clone());
        original = Document::new(patch, original.layout().clone()).unwrap();
        assert!(original.equivalent(&original.clone()));
        // Different immutable content with the same sample count must dirty
        // the document; a descriptor cannot be reduced to length alone.
        let replacement = Arc::new(WaveAsset::new(&[0.0, 0.25, -0.5, 0.75]).unwrap());
        let mut patch = original.patch().clone();
        patch.assets.clear();
        patch.assets.insert(replacement.id().into(), replacement);
        let changed = Document::new(patch, original.layout().clone()).unwrap();
        assert!(!original.equivalent(&changed));
        let mut patch = original.patch().clone();
        patch.nodes[0].params.insert("value".into(), 2.0);
        let changed = Document::new(patch, original.layout().clone()).unwrap();
        assert!(!original.equivalent(&changed));
        let mut layout = original.layout().clone();
        layout["pan"] = json!([100.0, 200.0]);
        let changed = original.clone().with_layout(layout).unwrap();
        assert!(!original.equivalent(&changed));
        let ordinary = document(1.0);
        assert!(ordinary.equivalent(&ordinary.clone()));
        assert!(serde_json::to_value(ordinary.patch())
            .unwrap()
            .get("assets")
            .is_none());
    }

    #[test]
    fn oversized_recovery_uses_the_64_mib_envelope_without_reading_sparse_payload() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::new(directory.path()).unwrap();
        store.save_recovery(&document(1.0)).unwrap();
        let recovery = store.recovery_name.clone();
        let path = directory.path().join(&recovery);
        File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_len((MAX_DOCUMENT_BYTES + 1) as u64)
            .unwrap();
        drop(store);
        let restarted = Store::new(directory.path()).unwrap();
        assert!(restarted
            .load_recovery(&recovery)
            .unwrap_err()
            .contains("64 MiB"));
        assert_eq!(
            fs::metadata(path).unwrap().len(),
            (MAX_DOCUMENT_BYTES + 1) as u64
        );
    }

    #[test]
    fn templates_round_trip_both_engines_graph_ids_layout_and_saved_run_hold_values() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::new(directory.path()).unwrap();
        assert!(store.load_template().unwrap().is_none());
        assert!(store.template_token().unwrap().is_none());
        let mut token = None;
        for version in [1, 2] {
            let mut patch: Patch =
                serde_json::from_value(json!({"version":version,"sample_rate":48000,
                "nodes":[{"id":"pitch","kind":"constant","params":{"value":1.25}},
                    {"id":"level","kind":"gain","params":{"gain":0.125}}],
                "cables":[{"id":"retained_cable","from":"pitch","to":"level","input":"signal"}],
                "output":"level"}))
                .unwrap();
            if version == 2 {
                patch.nodes.extend(serde_json::from_value::<Vec<openpatch_core::Node>>(json!([
                    {"id":"pattern","kind":"seq3","params":{"running":0.0,"tempo":123.5,"row1_1":0.75}},
                    {"id":"latch","kind":"push","params":{"holding":1.0}}])).unwrap());
            }
            let mut rack = Rack::default();
            rack.reconcile(&patch, &catalog());
            let mut layout = rack.layout();
            layout["zoom"] = json!(4.0);
            layout["pan"] = json!([-780.5, 431.25]);
            layout["positions"]["pitch"] = json!([15.0, 420.0]);
            let original = Document::new(patch, layout).unwrap();
            token = Some(store.save_template(&original, token.as_ref()).unwrap());
            let loaded = store.load_template().unwrap().unwrap();
            assert!(original.equivalent(&loaded));
            assert_eq!(
                loaded.patch().cables[0].id.as_deref(),
                Some("retained_cable")
            );
            if version == 2 {
                assert_eq!(
                    loaded
                        .patch()
                        .nodes
                        .iter()
                        .find(|n| n.id == "pattern")
                        .unwrap()
                        .params["running"],
                    0.0
                );
                assert_eq!(
                    loaded
                        .patch()
                        .nodes
                        .iter()
                        .find(|n| n.id == "latch")
                        .unwrap()
                        .params["holding"],
                    1.0
                );
            }
        }
    }

    #[test]
    fn corrupt_template_can_be_replaced_only_with_its_reviewed_raw_token() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::new(directory.path()).unwrap();
        let path = directory.path().join(TEMPLATE_NAME);
        fs::write(&path, b"broken template JSON").unwrap();
        assert!(store.load_template().is_err());
        let token = store.template_token().unwrap().unwrap();
        assert_eq!(token.bytes, b"broken template JSON");
        assert!(store.save_template(&document(2.0), None).is_err());
        let saved = store.save_template(&document(2.0), Some(&token)).unwrap();
        assert!(store
            .load_template()
            .unwrap()
            .unwrap()
            .equivalent(&document(2.0)));
        assert_eq!(fs::read(&path).unwrap(), saved.bytes);
        let project = store
            .save_new("ordinary.openpatch", &document(3.0))
            .unwrap();
        assert!(store.save_template(&document(4.0), Some(&project)).is_err());
        assert_eq!(fs::read(&path).unwrap(), saved.bytes);
    }

    #[test]
    fn template_review_conflicts_and_cooperative_lock_never_overwrite_another_writer() {
        let directory = tempfile::tempdir().unwrap();
        let a = Store::new(directory.path()).unwrap();
        let b = Store::new(directory.path()).unwrap();
        let token = a.save_template(&document(1.0), None).unwrap();
        let stale = b.template_token().unwrap().unwrap();
        let lock = companion_lock(directory.path(), TEMPLATE_NAME).unwrap();
        assert!(b.save_template(&document(2.0), Some(&stale)).is_err());
        assert_eq!(
            fs::read(directory.path().join(TEMPLATE_NAME)).unwrap(),
            token.bytes
        );
        drop(lock);
        let first = a.save_template(&document(3.0), Some(&token)).unwrap();
        assert!(b.save_template(&document(4.0), Some(&stale)).is_err());
        assert!(b.save_template(&document(4.0), None).is_err());
        assert_eq!(
            fs::read(directory.path().join(TEMPLATE_NAME)).unwrap(),
            first.bytes
        );
        fs::remove_file(directory.path().join(TEMPLATE_NAME)).unwrap();
        assert!(a.save_template(&document(5.0), Some(&first)).is_err());
        assert!(!directory.path().join(TEMPLATE_NAME).exists());
        // A late noncooperating create is still protected by no-clobber publish.
        let path = directory.path().join(TEMPLATE_NAME);
        before_next_publish(move || {
            fs::write(path, b"late external create").unwrap();
        });
        assert!(a.save_template(&document(6.0), None).is_err());
        assert_eq!(
            fs::read(directory.path().join(TEMPLATE_NAME)).unwrap(),
            b"late external create"
        );
    }

    #[test]
    fn template_invalid_oversized_and_directory_inputs_are_rejected_without_publication() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::new(directory.path()).unwrap();
        let path = directory.path().join(TEMPLATE_NAME);
        let original = document(1.0);
        let mut invalid = original.layout().clone();
        invalid["extra"] = json!(true);
        assert!(original.with_layout(invalid).is_err());
        assert!(!path.exists());
        assert!(!directory
            .path()
            .join(format!("{TEMPLATE_NAME}.lock"))
            .exists());
        File::create(&path)
            .unwrap()
            .set_len((MAX_DOCUMENT_BYTES + 1) as u64)
            .unwrap();
        assert!(store.load_template().unwrap_err().contains("64 MiB"));
        assert!(store.template_token().unwrap_err().contains("64 MiB"));
        assert!(store.save_template(&document(1.0), None).is_err());
        assert_eq!(
            fs::metadata(&path).unwrap().len(),
            (MAX_DOCUMENT_BYTES + 1) as u64
        );
        fs::remove_file(&path).unwrap();
        fs::create_dir(&path).unwrap();
        assert!(store.load_template().is_err());
        assert!(store.template_token().is_err());
        assert!(store.save_template(&document(1.0), None).is_err());
        assert!(path.is_dir());
    }

    #[test]
    fn template_failed_publication_and_durability_warning_follow_project_contract() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::new(directory.path()).unwrap();
        let path = directory.path().join(TEMPLATE_NAME);
        fail_next_publish();
        assert!(store.save_template(&document(1.0), None).is_err());
        assert!(!path.exists());
        let original = store.save_template(&document(1.0), None).unwrap();
        fail_next_publish();
        assert!(store
            .save_template(&document(2.0), Some(&original))
            .is_err());
        assert_eq!(fs::read(&path).unwrap(), original.bytes);
        FAIL_DIRECTORY_SYNC.with(|flag| flag.set(true));
        let saved = store
            .save_template(&document(2.0), Some(&original))
            .unwrap();
        assert!(saved.durability_warning.is_some());
        assert_eq!(fs::read(&path).unwrap(), saved.bytes);
        assert!(store
            .save_template(&document(3.0), Some(&original))
            .is_err());
        assert!(store.save_template(&document(3.0), Some(&saved)).is_ok());
    }

    #[test]
    fn template_is_never_a_recovery_or_accessible_through_ordinary_project_apis() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::new(directory.path()).unwrap();
        let token = store.save_template(&document(1.0), None).unwrap();
        assert!(store.load(TEMPLATE_NAME).is_err());
        assert!(store.save_new(TEMPLATE_NAME, &document(2.0)).is_err());
        assert!(store
            .save_existing(TEMPLATE_NAME, &document(2.0), &token)
            .is_err());
        assert!(store.load_recovery(TEMPLATE_NAME).is_err());
        assert!(store.discard_recovery(TEMPLATE_NAME).is_err());
        drop(store);
        let reopened = Store::new(directory.path()).unwrap();
        assert!(reopened.recoveries().is_empty());
        assert!(reopened.load_template().unwrap().is_some());
    }

    #[cfg(unix)]
    #[test]
    fn template_symlinks_dangling_symlinks_and_unsafe_companions_are_rejected() {
        use std::os::unix::fs::symlink;
        let directory = tempfile::tempdir().unwrap();
        let store = Store::new(directory.path()).unwrap();
        let outside = directory.path().join("ordinary.openpatch");
        let original = document(1.0).encode().unwrap();
        fs::write(&outside, &original).unwrap();
        let path = directory.path().join(TEMPLATE_NAME);
        let absent = directory.path().join("absent.openpatch");
        for target in [&outside, &absent] {
            symlink(target, &path).unwrap();
            assert!(store.load_template().is_err());
            assert!(store.template_token().is_err());
            assert!(store.save_template(&document(2.0), None).is_err());
            fs::remove_file(&path).unwrap();
        }
        assert_eq!(fs::read(&outside).unwrap(), original);
        let lock = directory.path().join(format!("{TEMPLATE_NAME}.lock"));
        // Earlier rejected writes may have reserved a stable safe companion.
        if lock.exists() {
            fs::remove_file(&lock).unwrap();
        }
        symlink(&outside, &lock).unwrap();
        assert!(store.save_template(&document(2.0), None).is_err());
        assert!(!path.exists());
        assert_eq!(fs::read(&outside).unwrap(), original);
        fs::remove_file(&lock).unwrap();
        fs::create_dir(&lock).unwrap();
        assert!(store.save_template(&document(2.0), None).is_err());
    }

    #[test]
    fn full_zoom_range_round_trips_through_projects_and_crash_recovery() {
        for zoom in [0.25, 0.85, 1.0, 4.0] {
            let directory = tempfile::tempdir().unwrap();
            let store = Store::new(directory.path()).unwrap();
            let original = document(1.0);
            let mut layout = original.layout().clone();
            layout["zoom"] = json!(zoom);
            layout["pan"] = json!([-452.5, 930.0]);
            let original = original.with_layout(layout).unwrap();
            store.save_new("zoom.openpatch", &original).unwrap();
            assert!(original.equivalent(&store.load("zoom.openpatch").unwrap().document));
            store.save_recovery(&original).unwrap();
            drop(store);
            let reopened = Store::new(directory.path()).unwrap();
            let names = reopened.recoveries();
            assert_eq!(names.len(), 1);
            assert!(original.equivalent(&reopened.load_recovery(&names[0]).unwrap().document));
            for invalid in [json!(0.249), json!(4.001), Value::Null, json!("1.0")] {
                let mut layout = original.layout().clone();
                layout["zoom"] = invalid;
                assert!(original.clone().with_layout(layout).is_err());
            }
        }
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
        let original = document(1.0);
        let mut layout = original.layout().clone();
        layout["extra"] = json!(true);
        assert!(original.with_layout(layout).is_err());
        assert!(!directory.path().join("bad.openpatch").exists());
        assert!(!directory.path().join("bad.openpatch.lock").exists());
        assert_eq!(MAX_DOCUMENT_BYTES, 64 * 1_048_576);
        for name in ["large.openpatch", "large.json"] {
            File::create(directory.path().join(name))
                .unwrap()
                .set_len((MAX_DOCUMENT_BYTES + 1) as u64)
                .unwrap();
            assert!(store.load(name).unwrap_err().contains("64 MiB"));
        }
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
        let bytes = serde_json::to_vec(original.patch()).unwrap();
        fs::write(&path, &bytes).unwrap();
        let loaded = store.load("legacy.json").unwrap();
        assert!(loaded.imported);
        assert!(loaded.token.is_none());
        assert!(loaded.document.layout()["positions"]["source"].is_array());
        assert_eq!(fs::read(&path).unwrap(), bytes);
        let sidecar = directory.path().join("legacy.json.layout.json");
        fs::write(&sidecar, b"not JSON").unwrap();
        assert!(store.load("legacy.json").is_err());
        assert_eq!(fs::read(&path).unwrap(), bytes);
        assert_eq!(fs::read(&sidecar).unwrap(), b"not JSON");
        File::create(&sidecar)
            .unwrap()
            .set_len((MAX_LEGACY_LAYOUT_BYTES + 1) as u64)
            .unwrap();
        assert!(store.load("legacy.json").unwrap_err().contains("1 MiB"));
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
        let bytes = serde_json::to_vec(original.patch()).unwrap();
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
