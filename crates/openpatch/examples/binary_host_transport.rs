//! Actual companion transport gate, not application playback or a device test.
//! Waits occur only in the control driver. Instrumented endpoint calls never wait.
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
mod mac {
    use openpatch::binary_host::process::LaunchConfig;
    use openpatch::binary_host::{
        Fault, Fence, InputBlock, OutputBlock, ParamEvent, Region, Status, TransportEndpoint,
        QUANTUM, SAMPLE_RATE,
    };
    use serde_json::{json, Value};
    use sha2::{Digest, Sha256};
    use std::alloc::{GlobalAlloc, Layout, System};
    use std::cell::Cell;
    use std::fs::{self, File};
    use std::os::unix::fs::FileExt;
    use std::path::{Path, PathBuf};
    use std::process::{Child, Command, Stdio};
    use std::time::{Duration, Instant};
    type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
    thread_local! {
        static TRACK:Cell<bool>=const{Cell::new(false)};
        static ALLOC:Cell<usize>=const{Cell::new(0)};
        static DEALLOC:Cell<usize>=const{Cell::new(0)};
    }
    struct Allocator;
    unsafe impl GlobalAlloc for Allocator {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            TRACK.with(|on| {
                if on.get() {
                    ALLOC.with(|n| n.set(n.get() + 1));
                }
            });
            unsafe { System.alloc(layout) }
        }
        unsafe fn dealloc(&self, p: *mut u8, layout: Layout) {
            TRACK.with(|on| {
                if on.get() {
                    DEALLOC.with(|n| n.set(n.get() + 1));
                }
            });
            unsafe { System.dealloc(p, layout) }
        }
    }
    #[global_allocator]
    static ALLOCATOR: Allocator = Allocator;
    fn rt<T>(call: impl FnOnce() -> T) -> T {
        ALLOC.with(|n| n.set(0));
        DEALLOC.with(|n| n.set(0));
        TRACK.with(|n| n.set(true));
        let result = call();
        TRACK.with(|n| n.set(false));
        assert_eq!(ALLOC.with(Cell::get), 0, "endpoint allocated");
        assert_eq!(DEALLOC.with(Cell::get), 0, "endpoint deallocated");
        result
    }
    fn write(path: &Path, value: &Value) -> Result<()> {
        fs::write(path, serde_json::to_vec_pretty(value)?)?;
        Ok(())
    }
    struct Guard(Child);
    impl Drop for Guard {
        fn drop(&mut self) {
            if !matches!(self.0.try_wait(), Ok(Some(_))) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }
    }
    struct Host {
        child: Guard,
        region: Region,
        directory: PathBuf,
    }
    fn wait_for(mut check: impl FnMut() -> Result<bool>) -> Result<()> {
        let start = Instant::now();
        while !check()? {
            if start.elapsed() > Duration::from_secs(10) {
                return Err("control-driver timeout".into());
            }
            std::thread::sleep(Duration::from_micros(50));
        }
        Ok(())
    }
    impl Host {
        fn start(args: &[PathBuf], name: &str, seed: &Path, incarnation: u64) -> Result<Self> {
            Self::start_with_mask(args, name, seed, incarnation, 4)
        }
        fn start_with_mask(
            args: &[PathBuf],
            name: &str,
            seed: &Path,
            incarnation: u64,
            mask: u32,
        ) -> Result<Self> {
            let directory = args[3].join(name);
            fs::create_dir(&directory)?;
            let user = directory.join("private-user");
            fs::create_dir(&user)?;
            let shared = directory.join("transport");
            let region = Region::create(
                &shared,
                Fence {
                    incarnation,
                    generation: 9,
                    revision: 11,
                },
                mask,
                SAMPLE_RATE,
            )?;
            let mut command = Command::new(&args[0]);
            command.args([
                &args[2],
                &user,
                &shared,
                &seed.to_path_buf(),
                &directory.join("saved-module.json"),
            ]);
            for (key, _) in std::env::vars_os() {
                if key.to_string_lossy().starts_with("DYLD_") {
                    command.env_remove(key);
                }
            }
            command.env("DYLD_LIBRARY_PATH", &args[1]);
            command.stdout(Stdio::from(File::create(directory.join("child.stdout"))?));
            command.stderr(Stdio::from(File::create(directory.join("child.stderr"))?));
            let mut child = Guard(command.spawn()?);
            wait_for(|| {
                match region.status() {
                    Status::Ready => return Ok(true),
                    Status::Fault(code) => {
                        return Err(format!("companion startup fault {code}").into())
                    }
                    _ => {}
                }
                if let Some(exit) = child.0.try_wait()? {
                    return Err(format!("companion startup exited {exit}").into());
                }
                Ok(false)
            })?;
            let identity: Value = serde_json::from_slice(&fs::read(user.join("identity.json"))?)?;
            assert_eq!(
                identity["protocol"],
                openpatch::binary_host::PROTOCOL_VERSION
            );
            assert_eq!(identity["rack_runtime"], "2.6.6");
            assert_eq!(
                identity["loaded_rack_runtimes"]
                    .as_array()
                    .ok_or("runtime array missing")?
                    .len(),
                1
            );
            assert_eq!(
                Path::new(
                    identity["loaded_rack_runtimes"][0]
                        .as_str()
                        .ok_or("runtime path missing")?
                )
                .canonicalize()?,
                args[1].join("libRack.dylib").canonicalize()?
            );
            assert_eq!(
                Path::new(
                    identity["loaded_plugin"]
                        .as_str()
                        .ok_or("plugin path missing")?
                )
                .canonicalize()?,
                args[2].join("plugin.dylib").canonicalize()?
            );
            Ok(Self {
                child,
                region,
                directory,
            })
        }
        fn signal(&mut self, signal: i32) -> Result<()> {
            // Test driver only, always the specific child spawned above.
            if unsafe { libc::kill(self.child.0.id() as i32, signal) } != 0 {
                return Err(std::io::Error::last_os_error().into());
            }
            Ok(())
        }
        fn freeze(&mut self) -> Result<()> {
            self.signal(libc::SIGSTOP)?;
            let mut status = 0;
            let pid =
                unsafe { libc::waitpid(self.child.0.id() as i32, &mut status, libc::WUNTRACED) };
            if pid != self.child.0.id() as i32 || !libc::WIFSTOPPED(status) {
                return Err("child did not stop".into());
            }
            Ok(())
        }
        fn finish(&mut self) -> Result<Value> {
            self.region.request_stop();
            wait_for(|| Ok(self.child.0.try_wait()?.is_some()))?;
            let exit = self.child.0.wait()?;
            assert!(exit.success(), "companion exit {exit}");
            assert_eq!(self.region.status(), Status::Stopped);
            Ok(serde_json::from_slice(&fs::read(
                self.directory.join("saved-module.json"),
            )?)?)
        }
    }
    fn transfer<R: AsRef<Region>>(
        endpoint: &mut TransportEndpoint<R>,
        input: &InputBlock,
        output: &mut OutputBlock,
    ) -> Result<()> {
        rt(|| endpoint.submit(input)).map_err(|e| format!("submit {e:?}"))?;
        wait_for(|| match endpoint.status() {
            Status::Ready => Ok(endpoint.is_output_ready()),
            s => Err(format!("processing stopped {s:?}").into()),
        })?;
        rt(|| endpoint.receive_due(output)).map_err(|e| format!("receive {e:?}"))?;
        Ok(())
    }
    fn managed_config(args: &[PathBuf], seed: &Value, incarnation: u64) -> Result<LaunchConfig> {
        Ok(LaunchConfig {
            companion_executable: args[0].clone(),
            sdk_dir: args[1].clone(),
            plugin_dir: args[2].clone(),
            workspace_parent: args[3].clone(),
            expected_plugin_sha256: format!(
                "{:x}",
                Sha256::digest(fs::read(args[2].join("plugin.dylib"))?)
            ),
            expected_runtime_sha256: format!(
                "{:x}",
                Sha256::digest(fs::read(args[1].join("libRack.dylib"))?)
            ),
            fence: Fence {
                incarnation,
                generation: 9,
                revision: 11,
            },
            input_mask: 4,
            snapshot: seed.clone(),
            startup_timeout: Duration::from_secs(5),
        })
    }
    fn managed_gate(args: &[PathBuf], seed: &Value) -> Result<()> {
        let mut prepared = managed_config(args, seed, 20)?.launch()?;
        let workspace = prepared.process.workspace_path().to_path_buf();
        fs::copy(
            workspace.join("private-user/identity.json"),
            args[3].join("managed-identity.json"),
        )?;
        let worker = std::thread::spawn(move || -> std::result::Result<_, String> {
            let mut endpoint = prepared.endpoint;
            let mut input = InputBlock::silence(4);
            for frame in &mut input.values[2] {
                frame[0] = 4.;
            }
            let mut output = OutputBlock::default();
            for block in 0..4 {
                input.event_count = usize::from(block == 1);
                input.events[0] = ParamEvent {
                    sample_offset: 64,
                    parameter: 1,
                    value: 0.25,
                    order: 1,
                };
                transfer(&mut endpoint, &input, &mut output).map_err(|e| e.to_string())?;
                for (frame, values) in output.values.iter().enumerate() {
                    let expected = if block == 0 && frame == 0 {
                        0.
                    } else if block < 1 || (block == 1 && frame < 64) {
                        10.
                    } else {
                        2.5
                    };
                    assert_eq!(values[0], expected);
                }
            }
            Ok(endpoint)
        });
        assert_eq!(prepared.process.poll_health()?, Status::Ready);
        while !worker.is_finished() {
            assert_eq!(prepared.process.poll_health()?, Status::Ready);
            std::thread::sleep(Duration::from_millis(1));
        }
        let endpoint = worker
            .join()
            .map_err(|_| "owned endpoint worker panicked")??;
        let captured = prepared.process.finish(endpoint)?;
        let mut expected = seed.clone();
        expected["params"][1]["value"] = json!(0.25);
        assert_eq!(captured.state, expected);
        assert_eq!(
            captured.fence,
            Fence {
                incarnation: 20,
                generation: 9,
                revision: 11
            }
        );
        write(&args[3].join("managed-capture.json"), &captured.state)?;
        assert!(
            !workspace.exists(),
            "successful controller must clean private workspace"
        );
        let restored = managed_config(args, &captured.state, 21)?.launch()?;
        assert_eq!(
            restored.process.finish(restored.endpoint)?.state,
            captured.state
        );
        let mut mismatch = managed_config(args, seed, 22)?;
        mismatch.expected_plugin_sha256 = "0".repeat(64);
        assert!(
            mismatch.launch().is_err(),
            "wrong fingerprint must never be prepared"
        );
        let mut pending = managed_config(args, seed, 23)?.launch()?;
        let pending_pid = pending.process.id() as i32;
        let pending_workspace = pending.process.workspace_path().to_path_buf();
        rt(|| pending.endpoint.submit(&InputBlock::silence(4))).unwrap();
        assert!(
            pending.process.finish(pending.endpoint).is_err(),
            "unreceived block must prevent a successful capture"
        );
        assert!(!pending_workspace.exists());
        assert_eq!(unsafe { libc::kill(pending_pid, 0) }, -1);
        assert_eq!(
            std::io::Error::last_os_error().raw_os_error(),
            Some(libc::ESRCH)
        );
        // Even identical public fences cannot bind capture to another mapping.
        let first = managed_config(args, seed, 24)?.launch()?;
        let second = managed_config(args, seed, 24)?.launch()?;
        assert!(first.process.finish(second.endpoint).is_err());
        drop(first.endpoint);
        drop(second.process);
        Ok(())
    }
    fn all_zero(output: &OutputBlock) -> bool {
        output.counts.iter().all(|c| *c == 0) && output.values.iter().flatten().all(|v| *v == 0.)
    }
    pub fn run() -> Result<()> {
        let mut args = std::env::args_os()
            .skip(1)
            .map(PathBuf::from)
            .collect::<Vec<_>>();
        let orphan_helper = args
            .first()
            .is_some_and(|p| p.as_os_str() == "--parent-death-helper");
        if orphan_helper {
            args.remove(0);
        }
        if args.len() != 5 {
            return Err("usage: example COMPANION SDK PLUGIN OUTPUT_DIR SEED_STATE".into());
        }
        fs::create_dir_all(&args[3])?;
        if orphan_helper {
            let host = Host::start(&args, "orphan", &args[4], 7)?;
            write(
                &args[3].join("helper-ready.json"),
                &json!({"pid":host.child.0.id()}),
            )?;
            // Deliberately bypass guards: the actual companion must notice
            // supervising-parent death and terminate without their assistance.
            std::process::exit(0);
        }
        let mut seed: Value = serde_json::from_slice(&fs::read(&args[4])?)?;
        seed["params"] = json!([{"id":0,"value":0.0},{"id":1,"value":0.5}]);
        let seed_path = args[3].join("initial-state.json");
        write(&seed_path, &seed)?;
        let mut host = Host::start(&args, "processing", &seed_path, 1)?;
        let mut input = InputBlock::silence(4);
        let mut output = OutputBlock::default();
        let impulse_frame;
        let mut tone = Vec::with_capacity(QUANTUM * 376);
        // Explicit proposed transport delay in this offline artifact, independent
        // of the measured genuine Rack routing delay. Not a device qualification.
        tone.resize(QUANTUM, 0.);
        {
            let mut endpoint = host
                .region
                .take_endpoint()
                .ok_or("endpoint already taken")?;
            input.values[2][16][0] = 4.;
            transfer(&mut endpoint, &input, &mut output)?;
            let hits = output
                .values
                .iter()
                .enumerate()
                .filter(|(_, v)| v[0] != 0.)
                .map(|(i, v)| (i, v[0]))
                .collect::<Vec<_>>();
            assert_eq!(
                hits,
                vec![(17, 10.)],
                "genuine Engine impulse route changed"
            );
            impulse_frame = hits[0].0;
            input.values[2].fill([0.; 16]);
            input.counts[2].fill(16);
            for values in &mut input.values[2] {
                for (c, v) in values.iter_mut().enumerate() {
                    *v = c as f32 - 5.;
                }
            }
            for _ in 0..20 {
                transfer(&mut endpoint, &input, &mut output)?;
            }
            assert_eq!(output.counts, [16; QUANTUM]);
            for frame in &output.values {
                for (c, v) in frame.iter().enumerate() {
                    assert_eq!(*v, (c as f32 - 5.) * 2.5);
                }
            }
            // At this pinned source cadence the next width decisions occur at
            // absolute frames2760/2880: offsets72/64 in these successive blocks.
            // Input cable propagation adds one frame before the plugin sees it.
            for frame in 37..93 {
                input.counts[2][frame] = 1;
                input.values[2][frame][1..].fill(0.);
            }
            transfer(&mut endpoint, &input, &mut output)?;
            assert!(output.counts[..72].iter().all(|c| *c == 16));
            assert!(output.counts[72..].iter().all(|c| *c == 1));
            assert_eq!(output.values[37][1], -10.);
            assert_eq!(output.values[38][1], 0.);
            input.counts[2].fill(16);
            for values in &mut input.values[2] {
                for (c, v) in values.iter_mut().enumerate() {
                    *v = c as f32 - 5.;
                }
            }
            transfer(&mut endpoint, &input, &mut output)?;
            assert!(output.counts[..64].iter().all(|c| *c == 1));
            assert!(output.counts[64..].iter().all(|c| *c == 16));
            input = InputBlock::silence(4);
            for frame in &mut input.values[2] {
                frame[0] = 4.;
            }
            transfer(&mut endpoint, &input, &mut output)?;
            input.event_count = 3;
            input.events[0] = ParamEvent {
                sample_offset: 32,
                parameter: 1,
                value: 0.25,
                order: 1,
            };
            input.events[1] = ParamEvent {
                sample_offset: 32,
                parameter: 1,
                value: -0.25,
                order: 2,
            };
            input.events[2] = ParamEvent {
                sample_offset: 64,
                parameter: 0,
                value: 0.5,
                order: 3,
            };
            transfer(&mut endpoint, &input, &mut output)?;
            for (frame, v) in output.values.iter().enumerate() {
                assert_eq!(
                    v[0],
                    if frame < 32 {
                        10.
                    } else if frame < 64 {
                        -2.5
                    } else {
                        -5.625
                    }
                );
            }
            input.event_count = 2;
            input.events[0] = ParamEvent {
                sample_offset: 0,
                parameter: 0,
                value: 0.,
                order: 1,
            };
            input.events[1] = ParamEvent {
                sample_offset: 0,
                parameter: 1,
                value: 0.5,
                order: 2,
            };
            input.events[2] = ParamEvent::default();
            transfer(&mut endpoint, &input, &mut output)?;
            input.event_count = 0;
            input.values[2].fill([0.; 16]);
            transfer(&mut endpoint, &input, &mut output)?;
            let patch: openpatch_core::Patch = serde_json::from_value(
                json!({"version":2,"sample_rate":48000,"nodes":[{"id":"tone","kind":"oscillator","params":{"frequency":220.0,"amplitude":0.25}}],"output":"tone"}),
            )?;
            let mut engine = patch.compile().map_err(|e| format!("native source {e}"))?;
            let mut native = [0.; QUANTUM];
            let mut previous_native = 0.;
            for _ in 0..375 {
                engine.process_block(&mut native);
                for (f, v) in native.iter().enumerate() {
                    input.values[2][f][0] = *v;
                }
                transfer(&mut endpoint, &input, &mut output)?;
                assert_eq!(output.counts, [1; QUANTUM]);
                for (frame, source) in output.values.iter().zip(&native) {
                    assert_eq!(
                        frame[0],
                        previous_native * 2.5,
                        "native-to-hosted waveform/gain changed"
                    );
                    previous_native = *source;
                }
                tone.extend(output.values.iter().map(|frame| frame[0]));
            }
            assert_eq!(endpoint.fault(), None);
        }
        let saved = host.finish()?;
        assert_eq!(saved, seed, "persistent state at quiescent stop changed");
        let rms =
            (tone.iter().map(|v| f64::from(*v).powi(2)).sum::<f64>() / tone.len() as f64).sqrt();
        assert!(
            rms > 0.4 && rms < 0.45,
            "offline audio must have expected audible energy"
        );
        openpatch_core::write_wav(
            &args[3].join("native-source-through-existing-offset.wav"),
            &tone,
            SAMPLE_RATE,
        )?;
        let mut restored = Host::start(
            &args,
            "restored",
            &host.directory.join("saved-module.json"),
            2,
        )?;
        {
            let mut endpoint = restored.region.take_endpoint().unwrap();
            input = InputBlock::silence(4);
            for v in &mut input.values[2] {
                v[0] = 4.;
            }
            transfer(&mut endpoint, &input, &mut output)?;
            assert!(output.values[1..].iter().all(|v| v[0] == 10.));
        }
        assert_eq!(restored.finish()?, saved);
        let mut stalled = Host::start(&args, "stalled", &seed_path, 3)?;
        stalled.freeze()?;
        let stalled_pid = stalled.child.0.id() as i32;
        {
            let mut endpoint = stalled.region.take_endpoint().unwrap();
            rt(|| endpoint.submit(&input)).unwrap();
            assert_eq!(
                rt(|| endpoint.receive_due(&mut output)),
                Err(Fault::MissedDeadline)
            );
            assert!(all_zero(&output));
            if unsafe { libc::kill(stalled_pid, libc::SIGCONT) } != 0 {
                return Err(std::io::Error::last_os_error().into());
            }
            wait_for(|| Ok(endpoint.processed_blocks() == 1))?;
            assert_eq!(
                rt(|| endpoint.receive_due(&mut output)),
                Err(Fault::MissedDeadline)
            );
            assert!(all_zero(&output));
            assert_eq!(rt(|| endpoint.submit(&input)), Err(Fault::MissedDeadline));
        }
        // Reacquisition is refused: a deadline-failed endpoint requires a fresh
        // region/incarnation, never resets sequence zero over a late result.
        assert!(stalled.region.take_endpoint().is_none());
        stalled.finish()?;
        let mut crashed = Host::start(&args, "crashed", &seed_path, 4)?;
        crashed.freeze()?;
        let crashed_pid = crashed.child.0.id() as i32;
        {
            let mut endpoint = crashed.region.take_endpoint().unwrap();
            rt(|| endpoint.submit(&input)).unwrap();
            if unsafe { libc::kill(crashed_pid, libc::SIGKILL) } != 0 {
                return Err(std::io::Error::last_os_error().into());
            }
            wait_for(|| Ok(crashed.child.0.try_wait()?.is_some()))?;
            assert_eq!(
                rt(|| endpoint.receive_due(&mut output)),
                Err(Fault::MissedDeadline)
            );
            assert!(all_zero(&output));
        }
        // Child exit does not magically mutate shared words. A callback must
        // still enforce its deadline; liveness monitoring is control-thread work.
        let mut malformed = Host::start(&args, "malformed", &seed_path, 5)?;
        malformed.freeze()?;
        let malformed_pid = malformed.child.0.id() as i32;
        {
            let mut endpoint = malformed.region.take_endpoint().unwrap();
            rt(|| endpoint.submit(&input)).unwrap();
            // Deliberately violate the protocol as a broken external peer. The
            // production endpoint cannot publish this invalid event count.
            let file = fs::OpenOptions::new()
                .write(true)
                .open(malformed.directory.join("transport"))?;
            file.write_all_at(&65u32.to_le_bytes(), 128 + 40)?;
            if unsafe { libc::kill(malformed_pid, libc::SIGCONT) } != 0 {
                return Err(std::io::Error::last_os_error().into());
            }
            wait_for(|| Ok(matches!(endpoint.status(), Status::Fault(6))))?;
            assert_eq!(
                endpoint.processed_blocks(),
                0,
                "invalid block must not execute partially"
            );
            assert_eq!(
                rt(|| endpoint.receive_due(&mut output)),
                Err(Fault::CompanionFault(6))
            );
            assert!(all_zero(&output));
        }
        wait_for(|| Ok(malformed.child.0.try_wait()?.is_some()))?;
        assert!(!malformed.child.0.wait()?.success());
        let mut cv_seed = seed.clone();
        cv_seed["params"][0]["value"] = json!(0.25);
        let cv_seed_path = args[3].join("cv-state.json");
        write(&cv_seed_path, &cv_seed)?;
        let mut cv_host = Host::start_with_mask(&args, "cv-connected", &cv_seed_path, 6, 7)?;
        {
            let mut endpoint = cv_host.region.take_endpoint().unwrap();
            let mut cv_input = InputBlock::silence(7);
            cv_input.counts[1].fill(16);
            cv_input.counts[2].fill(16);
            for frame in 0..QUANTUM {
                cv_input.values[0][frame][0] = 5.;
                for channel in 0..16 {
                    cv_input.values[1][frame][channel] = [-10., -5., 0., 5., 10.][channel % 5];
                    cv_input.values[2][frame][channel] = 4.;
                }
            }
            for _ in 0..20 {
                transfer(&mut endpoint, &cv_input, &mut output)?;
            }
            assert_eq!(output.counts, [16; QUANTUM]);
            for frame in &output.values {
                for (channel, value) in frame.iter().enumerate() {
                    assert_eq!(
                        *value,
                        [-13.125, -3.28125, 0., 3.28125, 13.125][channel % 5]
                    );
                }
            }
        }
        assert_eq!(cv_host.finish()?, cv_seed);
        let helper_dir = args[3].join("parent-death");
        let mut helper = Guard(
            Command::new(std::env::current_exe()?)
                .arg("--parent-death-helper")
                .args([&args[0], &args[1], &args[2], &helper_dir, &seed_path])
                .stdout(Stdio::null())
                .stderr(Stdio::from(File::create(
                    args[3].join("parent-death-helper.stderr"),
                )?))
                .spawn()?,
        );
        wait_for(|| Ok(helper.0.try_wait()?.is_some()))?;
        assert!(helper.0.wait()?.success());
        let ready: Value =
            serde_json::from_slice(&fs::read(helper_dir.join("helper-ready.json"))?)?;
        let orphan_pid = ready["pid"].as_u64().ok_or("missing owned companion pid")? as i32;
        let file = File::open(helper_dir.join("orphan/transport"))?;
        wait_for(|| {
            let mut status = [0; 4];
            let mut error = [0; 4];
            file.read_exact_at(&mut status, 68)?;
            file.read_exact_at(&mut error, 76)?;
            Ok(u32::from_le_bytes(status) == 2 && u32::from_le_bytes(error) == 9)
        })?;
        wait_for(|| {
            let result = unsafe { libc::kill(orphan_pid, 0) };
            Ok(result < 0 && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH))
        })?;
        managed_gate(&args, &seed)?;
        let report = json!({"passed":true,"gate":"actual binary transport foundation; application integration pending",
            "build":if cfg!(debug_assertions){"debug"}else{"release"},"quantum":QUANTUM,"rate":SAMPLE_RATE,
            "rack_input_to_observed_output_frames":impulse_frame-16,"proposed_transport_delay_frames":QUANTUM,
            "offline_wav_frames":tone.len(),"offline_wav_rms":rms,"endpoint_heap_allocations":0,"endpoint_heap_deallocations":0,
            "checks":["actual plugin/runtime image identities","mono impulse","settled poly16","within-block channel changes at pinned source cadence","mono offset CV and poly16 bipolar scale CV","sample-offset and simultaneous ordered parameter events","native Rust oscillator waveform and gain through existing plugin offline WAV","quiescent capture/fresh-process restore","SIGSTOP deadline and late-result silence","no endpoint reacquisition","SIGKILL deadline silence","malformed input rejected before processing"],
            "parent_death_cleanup":true,"production_process_owner_checks":["pinned launch and identity receipt","owned endpoint moved to worker while controller stays on control thread","healthy acknowledged capture and fresh restore","wrong fingerprint refusal","pending capture refusal plus child reap/workspace cleanup","wrong mapping refusal even with identical fences"],"realtime_deadlines_qualified":false,"native_window_or_device_test":false,"document_ui_mcp_integration":false});
        write(&args[3].join("result.json"), &report)?;
        println!("{}", serde_json::to_string(&report)?);
        Ok(())
    }
}
fn main() {
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    if let Err(error) = mac::run() {
        eprintln!("binary host transport gate: {error}");
        std::process::exit(1);
    }
    #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
    {
        eprintln!("binary host transport gate requires macOS ARM64");
        std::process::exit(1);
    }
}
