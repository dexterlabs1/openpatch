//! Control-thread ownership of an explicitly launched, pinned v1 companion.
//!
//! This owner is deliberately !Send/!Sync. Transfer only `PreparedHost::endpoint`
//! to processing; return it at a quiescent barrier before persistent capture.

use super::{ControlView, Fence, OwnedEndpoint, Region, Status, SAMPLE_RATE};
use crate::project_document::hosted::{validate_supported_snapshot, HostedCapability};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::marker::PhantomData;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::rc::Rc;
use std::thread;
use std::time::{Duration, Instant};
use tempfile::TempDir;

const STATE_LIMIT: usize = 1024 * 1024;
const RECEIPT_LIMIT: usize = 16 * 1024;
const MANIFEST_LIMIT: usize = 1024 * 1024;
const FINGERPRINT_LIMIT: u64 = 256 * 1024 * 1024;
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(2);
const POLL_INTERVAL: Duration = Duration::from_millis(2);

/// All paths and fingerprints are explicit. Construction performs no execution.
#[derive(Debug)]
pub struct LaunchConfig {
    pub companion_executable: PathBuf,
    pub sdk_dir: PathBuf,
    pub plugin_dir: PathBuf,
    pub workspace_parent: PathBuf,
    pub expected_plugin_sha256: String,
    pub expected_runtime_sha256: String,
    pub fence: Fence,
    pub input_mask: u32,
    /// Full Rack module snapshot, including both parameters and custom flags.
    pub snapshot: Value,
    /// Must be nonzero and no greater than 30 seconds.
    pub startup_timeout: Duration,
}

pub struct PreparedHost {
    pub process: HostProcess,
    pub endpoint: OwnedEndpoint,
}

#[derive(Debug)]
pub struct CapturedState {
    pub fence: Fence,
    pub state: Value,
}

/// Owns child lifecycle and private staging; create/use/drop on a control thread.
/// Dropping stops/reaps the child but never publishes a persistent capture.
pub struct HostProcess {
    child: Option<Child>,
    control: ControlView,
    workspace: TempDir,
    fence: Fence,
    state_out: PathBuf,
    capability: HostedCapability,
    _control_thread: PhantomData<Rc<()>>,
}

impl LaunchConfig {
    /// Explicit control-thread launch. Verifies bytes before and after startup,
    /// then verifies the companion's actual loaded-image receipt.
    pub fn launch(self) -> io::Result<PreparedHost> {
        self.validate()?;
        let capability = validate_supported_snapshot(&self.snapshot).map_err(invalid)?;
        let executable = Fingerprint::read(&self.companion_executable)?;
        if fs::metadata(&executable.path)?.permissions().mode() & 0o111 == 0 {
            return Err(invalid("companion is not executable"));
        }
        let sdk = self.sdk_dir.canonicalize()?;
        let plugin = self.plugin_dir.canonicalize()?;
        let plugin_image = Fingerprint::read(&plugin.join("plugin.dylib"))?;
        let runtime = Fingerprint::read(&sdk.join("libRack.dylib"))?;
        let manifest = Fingerprint::read(&plugin.join("plugin.json"))?;
        if plugin_image.sha256 != self.expected_plugin_sha256
            || runtime.sha256 != self.expected_runtime_sha256
        {
            return Err(invalid(
                "plugin/runtime SHA-256 does not match authored identity",
            ));
        }
        validate_manifest(&read_json(&manifest.path, MANIFEST_LIMIT)?, capability)?;
        let snapshot = encode_snapshot(&self.snapshot)?;
        let workspace = tempfile::Builder::new()
            .prefix("openpatch-binary-host-")
            .tempdir_in(&self.workspace_parent)?;
        fs::set_permissions(workspace.path(), fs::Permissions::from_mode(0o700))?;
        let user = workspace.path().join("private-user");
        fs::create_dir(&user)?;
        fs::set_permissions(&user, fs::Permissions::from_mode(0o700))?;
        let state_in = workspace.path().join("state-in.json");
        let state_out = workspace.path().join("state-out.json");
        private_file(&state_in)?.write_all(&snapshot)?;
        // Precreate all child outputs so their permissions never depend on umask.
        private_file(&state_out)?;
        private_file(&user.join("identity.json"))?;
        let stdout = private_file(&workspace.path().join("child.stdout"))?;
        let stderr = private_file(&workspace.path().join("child.stderr"))?;
        let shared = workspace.path().join("transport");
        let region = Region::create_for_capability(
            &shared,
            self.fence,
            self.input_mask,
            SAMPLE_RATE,
            capability,
        )?;
        let control = region.control_view()?;
        let endpoint = region
            .into_endpoint()
            .ok_or_else(|| invalid("new transport endpoint unavailable"))?;
        let mut command = Command::new(&executable.path);
        command.args([&plugin, &user, &shared, &state_in, &state_out]);
        for (key, _) in std::env::vars_os() {
            if key.to_string_lossy().starts_with("DYLD_") {
                command.env_remove(key);
            }
        }
        command
            .env("DYLD_LIBRARY_PATH", &sdk)
            .stdin(Stdio::null())
            .stdout(Stdio::from(stdout))
            .stderr(Stdio::from(stderr));
        let child = command.spawn()?;
        // Every error after spawn passes through HostProcess::drop and reaps.
        let mut process = HostProcess {
            child: Some(child),
            control,
            workspace,
            fence: self.fence,
            state_out,
            capability,
            _control_thread: PhantomData,
        };
        let deadline = Instant::now() + self.startup_timeout;
        loop {
            match process.poll_health()? {
                Status::Ready => break,
                Status::Booting => {}
                status => return Err(invalid(format!("unexpected startup status {status:?}"))),
            }
            if Instant::now() >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "companion startup timed out",
                ));
            }
            thread::sleep(POLL_INTERVAL);
        }
        validate_receipt(
            &read_json(&user.join("identity.json"), RECEIPT_LIMIT)?,
            &plugin_image.path,
            &runtime.path,
            capability,
        )?;
        for fingerprint in [&executable, &plugin_image, &runtime, &manifest] {
            fingerprint.recheck()?;
        }
        // A child that failed during receipt verification is never prepared.
        if process.poll_health()? != Status::Ready {
            return Err(invalid("companion left READY during preparation"));
        }
        Ok(PreparedHost { process, endpoint })
    }

    fn validate(&self) -> io::Result<()> {
        if self.startup_timeout.is_zero() || self.startup_timeout > Duration::from_secs(30) {
            return Err(invalid("startup timeout must be in (0, 30 seconds]"));
        }
        if self.fence.incarnation == 0 || self.fence.generation == 0 || self.fence.revision == 0 {
            return Err(invalid("host fences must be nonzero"));
        }
        let capability = validate_supported_snapshot(&self.snapshot).map_err(invalid)?;
        if !capability.accepts_input_mask(self.input_mask) {
            return Err(invalid("unsupported fixed input connection mask"));
        }
        for hash in [&self.expected_plugin_sha256, &self.expected_runtime_sha256] {
            if hash.len() != 64
                || !hash
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            {
                return Err(invalid("fingerprints must be lowercase SHA-256"));
            }
        }
        validate_snapshot(&self.snapshot)
    }
}

impl HostProcess {
    pub fn fence(&self) -> Fence {
        self.fence
    }

    pub fn id(&self) -> u32 {
        self.child
            .as_ref()
            .expect("child retained until owner drops")
            .id()
    }

    /// Private diagnostics remain available only while this owner is alive.
    pub fn workspace_path(&self) -> &Path {
        self.workspace.path()
    }

    /// Nonblocking control-thread observation. Does not read child log contents.
    pub fn poll_health(&mut self) -> io::Result<Status> {
        let status = self.control.status();
        let exit = self.child.as_mut().expect("child retained").try_wait()?;
        if let Some(exit) = exit {
            return Err(io::Error::other(format!(
                "companion exited unexpectedly: {exit}; transport {status:?}"
            )));
        }
        match status {
            Status::Fault(code) => Err(io::Error::other(format!("companion fault {code}"))),
            Status::Stopped | Status::Invalid => {
                Err(invalid(format!("unexpected companion status {status:?}")))
            }
            _ => Ok(status),
        }
    }

    /// Consume the returned endpoint at a quiescent barrier. Only a successful,
    /// fully acknowledged stop can produce persistent state for this fence.
    pub fn finish(mut self, endpoint: OwnedEndpoint) -> io::Result<CapturedState> {
        if endpoint.fence() != self.fence || !self.control.owns_endpoint(&endpoint) {
            return Err(invalid("capture endpoint does not belong to this process"));
        }
        if endpoint.fault().is_some()
            || !endpoint.is_drained()
            || endpoint.submitted_blocks() != endpoint.received_blocks()
        {
            return Err(invalid(
                "capture requires a healthy, fully drained endpoint",
            ));
        }
        let submitted = endpoint.submitted_blocks();
        if submitted > u32::MAX as u64 {
            return Err(invalid("capture processed counter exhausted"));
        }
        let deadline = Instant::now() + SHUTDOWN_TIMEOUT;
        loop {
            if self.poll_health()? != Status::Ready {
                return Err(invalid("capture requires READY companion"));
            }
            let processed = u64::from(self.control.processed_blocks());
            if processed == submitted {
                break;
            }
            if processed > submitted {
                return Err(invalid("capture acknowledgement exceeds submitted blocks"));
            }
            if Instant::now() >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "capture acknowledgement timed out",
                ));
            }
            thread::sleep(POLL_INTERVAL);
        }
        self.control.request_stop();
        let deadline = Instant::now() + SHUTDOWN_TIMEOUT;
        let exit = loop {
            if let Some(exit) = self.child.as_mut().expect("child retained").try_wait()? {
                break exit;
            }
            if Instant::now() >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "companion state capture timed out",
                ));
            }
            thread::sleep(POLL_INTERVAL);
        };
        if !exit.success()
            || self.control.status() != Status::Stopped
            || u64::from(self.control.processed_blocks()) != submitted
        {
            return Err(io::Error::other(format!(
                "capture failed: exit {exit}; transport {:?}",
                self.control.status()
            )));
        }
        let state = read_json(&self.state_out, STATE_LIMIT)?;
        if validate_supported_snapshot(&state).map_err(invalid)? != self.capability {
            return Err(invalid(
                "captured state changed the requested hosted capability",
            ));
        }
        Ok(CapturedState {
            fence: self.fence,
            state,
        })
    }
}

impl Drop for HostProcess {
    fn drop(&mut self) {
        let Some(child) = self.child.as_mut() else {
            return;
        };
        self.control.request_stop();
        let deadline = Instant::now() + Duration::from_millis(100);
        loop {
            match child.try_wait() {
                Ok(Some(_)) => return,
                Ok(None) if Instant::now() < deadline => thread::sleep(POLL_INTERVAL),
                _ => break,
            }
        }
        let _ = child.kill();
        // Always reap before private workspace/mapping destruction, even when
        // the graceful stop failed. No authored state is published from Drop.
        let _ = child.wait();
    }
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

fn private_file(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
}

fn read_json(path: &Path, limit: usize) -> io::Result<Value> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.len() == 0 || metadata.len() > limit as u64 {
        return Err(invalid(
            "JSON file is empty, nonregular or exceeds bounded size",
        ));
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take((limit + 1) as u64).read_to_end(&mut bytes)?;
    if bytes.len() > limit {
        return Err(invalid("JSON file exceeds bounded size"));
    }
    serde_json::from_slice(&bytes).map_err(|_| invalid("invalid bounded JSON file"))
}

fn encode_snapshot(state: &Value) -> io::Result<Vec<u8>> {
    validate_snapshot(state)?;
    struct Bounded(Vec<u8>);
    impl Write for Bounded {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            if bytes.len() > STATE_LIMIT - self.0.len() {
                return Err(invalid("module snapshot exceeds 1 MiB"));
            }
            self.0.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let mut encoded = Bounded(Vec::new());
    serde_json::to_writer(&mut encoded, state)
        .map_err(|_| invalid("module snapshot exceeds 1 MiB or cannot serialize"))?;
    Ok(encoded.0)
}

fn validate_snapshot(state: &Value) -> io::Result<()> {
    validate_supported_snapshot(state)
        .map(|_| ())
        .map_err(invalid)
}

fn validate_manifest(manifest: &Value, capability: HostedCapability) -> io::Result<()> {
    if manifest["slug"] != "Bogaudio"
        || manifest["version"] != "2.6.47"
        || !manifest["modules"]
            .as_array()
            .is_some_and(|modules| modules.iter().any(|m| m["slug"] == capability.model()))
    {
        return Err(invalid(
            "manifest requires Bogaudio 2.6.47 with Offset model",
        ));
    }
    Ok(())
}

fn validate_receipt(
    receipt: &Value,
    plugin: &Path,
    runtime: &Path,
    capability: HostedCapability,
) -> io::Result<()> {
    if receipt["protocol"].as_u64() != Some(super::PROTOCOL_VERSION as u64)
        || receipt["profile_id"].as_u64() != Some(capability as u64)
        || receipt["rack_runtime"] != "2.6.6"
        || receipt["plugin_slug"] != "Bogaudio"
        || receipt["plugin_version"] != "2.6.47"
        || receipt["model_slug"] != capability.model()
        || receipt["sample_rate"].as_u64() != Some(SAMPLE_RATE as u64)
        || receipt["quantum"].as_u64() != Some(super::QUANTUM as u64)
    {
        return Err(invalid(
            "companion receipt protocol/runtime/model metadata mismatch",
        ));
    }
    let runtimes = receipt["loaded_rack_runtimes"]
        .as_array()
        .filter(|r| r.len() == 1)
        .ok_or_else(|| invalid("companion must load exactly one Rack runtime"))?;
    for (reported, expected) in [(&receipt["loaded_plugin"], plugin), (&runtimes[0], runtime)] {
        let path = reported
            .as_str()
            .ok_or_else(|| invalid("receipt image path missing"))?;
        if !Path::new(path).is_absolute() || Path::new(path).canonicalize()? != expected {
            return Err(invalid("companion loaded unexpected plugin/runtime image"));
        }
    }
    Ok(())
}

#[derive(PartialEq, Eq)]
struct Fingerprint {
    path: PathBuf,
    sha256: String,
    device: u64,
    inode: u64,
    size: u64,
    modified_seconds: i64,
    modified_nanoseconds: i64,
}

impl Fingerprint {
    fn read(path: &Path) -> io::Result<Self> {
        let path = path.canonicalize()?;
        let mut file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(&path)?;
        let metadata = file.metadata()?;
        if !metadata.is_file() || metadata.len() > FINGERPRINT_LIMIT {
            return Err(invalid(
                "fingerprinted input must be a regular file no larger than 256 MiB",
            ));
        }
        let mut hash = Sha256::new();
        let mut bytes = [0u8; 64 * 1024];
        let mut limited = (&mut file).take(metadata.len() + 1);
        let mut total = 0u64;
        loop {
            let length = limited.read(&mut bytes)?;
            if length == 0 {
                break;
            }
            total += length as u64;
            hash.update(&bytes[..length]);
        }
        if total != metadata.len() {
            return Err(invalid("input length changed while fingerprinting"));
        }
        let after = file.metadata()?;
        if metadata.len() != after.len()
            || metadata.mtime() != after.mtime()
            || metadata.mtime_nsec() != after.mtime_nsec()
        {
            return Err(invalid("input changed while fingerprinting"));
        }
        Ok(Self {
            path,
            sha256: format!("{:x}", hash.finalize()),
            device: metadata.dev(),
            inode: metadata.ino(),
            size: metadata.len(),
            modified_seconds: metadata.mtime(),
            modified_nanoseconds: metadata.mtime_nsec(),
        })
    }

    fn recheck(&self) -> io::Result<()> {
        if Self::read(&self.path)? != *self {
            return Err(invalid(
                "companion/plugin/runtime/manifest changed during startup",
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn snapshot() -> Value {
        json!({"plugin":"Bogaudio", "model":"Bogaudio-Offset", "version":"2.6.47", "id":2,
            "params":[{"id":0,"value":0.25},{"id":1,"value":-0.5}],
            "data":{"offset_first":true,"disableOutputLimit":false}})
    }

    #[test]
    fn snapshot_rejects_missing_custom_state_duplicate_parameters_and_oversize() {
        assert!(encode_snapshot(&snapshot()).is_ok());
        let mut state = snapshot();
        state["data"]
            .as_object_mut()
            .unwrap()
            .remove("offset_first");
        assert!(encode_snapshot(&state).is_err());
        let mut state = snapshot();
        state["params"][1]["id"] = json!(0);
        assert!(encode_snapshot(&state).is_err());
        let mut state = snapshot();
        state["extra"] = json!("x".repeat(STATE_LIMIT));
        assert!(encode_snapshot(&state).is_err());
    }

    #[test]
    fn config_validation_refuses_unbounded_timeout_and_noncanonical_hashes() {
        let mut config = LaunchConfig {
            companion_executable: PathBuf::new(),
            sdk_dir: PathBuf::new(),
            plugin_dir: PathBuf::new(),
            workspace_parent: PathBuf::new(),
            expected_plugin_sha256: "a".repeat(64),
            expected_runtime_sha256: "0".repeat(64),
            fence: Fence {
                incarnation: 1,
                generation: 2,
                revision: 3,
            },
            input_mask: 4,
            snapshot: snapshot(),
            startup_timeout: Duration::from_secs(1),
        };
        assert!(config.validate().is_ok());
        config.startup_timeout = Duration::ZERO;
        assert!(config.validate().is_err());
        config.startup_timeout = Duration::from_secs(31);
        assert!(config.validate().is_err());
        config.startup_timeout = Duration::from_secs(1);
        config.expected_plugin_sha256 = "A".repeat(64);
        assert!(config.validate().is_err());
        config.expected_plugin_sha256 = "a".repeat(64);
        config.input_mask = 0;
        assert!(config.validate().is_err());
    }

    #[test]
    fn fifo_preflight_is_refused_without_a_writer_or_child() {
        use std::os::unix::ffi::OsStrExt;
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("fifo");
        let encoded = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
        // SAFETY: encoded is a valid, NUL-terminated path in this private tempdir.
        assert_eq!(unsafe { libc::mkfifo(encoded.as_ptr(), 0o600) }, 0);
        assert!(read_json(&path, RECEIPT_LIMIT).is_err());
        assert!(Fingerprint::read(&path).is_err());
    }

    #[test]
    fn bounded_json_and_fingerprint_detect_changed_inputs() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("input");
        fs::write(&path, b"{}").unwrap();
        assert!(read_json(&path, 2).is_ok());
        assert!(read_json(&path, 1).is_err());
        let fingerprint = Fingerprint::read(&path).unwrap();
        fs::write(&path, b"[]").unwrap();
        assert!(fingerprint.recheck().is_err());
    }
}

#[cfg(test)]
mod capability_tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn loaded_receipt_is_bound_to_the_requested_model_and_wire_profile() {
        let dir = tempfile::tempdir().unwrap();
        let plugin = dir.path().join("plugin.dylib");
        let runtime = dir.path().join("libRack.dylib");
        fs::write(&plugin, []).unwrap();
        fs::write(&runtime, []).unwrap();
        let plugin = plugin.canonicalize().unwrap();
        let runtime = runtime.canonicalize().unwrap();
        let receipt = json!({"protocol":super::super::PROTOCOL_VERSION,"profile_id":2,"rack_runtime":"2.6.6","plugin_slug":"Bogaudio","plugin_version":"2.6.47","model_slug":"Bogaudio-XFade","sample_rate":48000,"quantum":128,"loaded_plugin":plugin,"loaded_rack_runtimes":[runtime]});
        validate_receipt(&receipt, &plugin, &runtime, HostedCapability::XFade).unwrap();
        assert!(validate_receipt(&receipt, &plugin, &runtime, HostedCapability::Offset).is_err());
        for (field, bad) in [
            ("protocol", json!(1)),
            ("profile_id", json!(1)),
            ("model_slug", json!("Bogaudio-Offset")),
        ] {
            let mut changed = receipt.clone();
            changed[field] = bad;
            assert!(
                validate_receipt(&changed, &plugin, &runtime, HostedCapability::XFade).is_err()
            );
        }
    }
}
