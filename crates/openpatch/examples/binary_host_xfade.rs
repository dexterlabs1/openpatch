//! Genuine XFade binary through shared Session control and production graph/runner.
//! Paced headless qualification, not a device deadline or native-window claim.
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
mod mac {
    use openpatch::binary_host::control::HostedControl;
    use openpatch::binary_host::graph::HostedGraphPlan;
    use openpatch::binary_host::process::LaunchConfig;
    use openpatch::binary_host::runner::HostedRunner;
    use openpatch::binary_host::{Fence, OutputBlock, QUANTUM, SAMPLE_RATE};
    use openpatch::control::{
        DocumentExecutionRequest, HostedEditAdmission, HostedEditProposal, LiveModuleEvents,
        Session,
    };
    use openpatch::project_document::{DocumentNode, ProjectDocument};
    use openpatch_core::ModuleAction;
    use serde_json::{json, Value};
    use sha2::{Digest, Sha256};
    use std::{
        alloc::{GlobalAlloc, Layout, System},
        cell::Cell,
        fs,
        path::PathBuf,
        sync::Arc,
        time::{Duration, Instant},
    };
    type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
    thread_local! {
        static TRACK: Cell<bool> = const { Cell::new(false) };
        static ALLOC: Cell<usize> = const { Cell::new(0) };
        static DEALLOC: Cell<usize> = const { Cell::new(0) };
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
        unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
            TRACK.with(|on| {
                if on.get() {
                    ALLOC.with(|n| n.set(n.get() + 1));
                }
            });
            unsafe { System.alloc_zeroed(layout) }
        }
        unsafe fn realloc(&self, p: *mut u8, layout: Layout, size: usize) -> *mut u8 {
            TRACK.with(|on| {
                if on.get() {
                    ALLOC.with(|n| n.set(n.get() + 1));
                    DEALLOC.with(|n| n.set(n.get() + 1));
                }
            });
            unsafe { System.realloc(p, layout, size) }
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
    fn rt<T>(call: impl FnOnce() -> T) -> (T, usize, usize) {
        ALLOC.with(|n| n.set(0));
        DEALLOC.with(|n| n.set(0));
        TRACK.with(|n| n.set(true));
        let result = call();
        TRACK.with(|n| n.set(false));
        (result, ALLOC.with(Cell::get), DEALLOC.with(Cell::get))
    }
    struct SessionBackend(Arc<HostedControl>);
    impl LiveModuleEvents for SessionBackend {
        fn trigger(&self, _: &str, _: ModuleAction) -> std::result::Result<Value, String> {
            Err("this control driver has no native action executor".into())
        }
        fn snapshot(&self) -> Value {
            serde_json::to_value(self.0.snapshot()).expect("control snapshot serializes")
        }
        fn supports_document_execution(&self) -> bool {
            true
        }
        fn reserve_hosted_parameters(
            &self,
            proposal: HostedEditProposal<'_>,
        ) -> std::result::Result<Option<Box<dyn HostedEditAdmission>>, String> {
            proposal.reserve_with_control(&self.0).map(Some)
        }
    }

    fn module(document: &ProjectDocument) -> &openpatch::project_document::hosted::HostedModule {
        document
            .graph
            .nodes
            .iter()
            .find_map(|n| match n {
                DocumentNode::Hosted { module, .. } => Some(module),
                _ => None,
            })
            .unwrap()
    }
    pub fn run() -> Result<()> {
        let args: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
        if args.len() != 5 {
            return Err("expected COMPANION SDK PLUGIN OUTPUT SEED".into());
        }
        fs::create_dir_all(&args[3])?;
        let seed: Value = serde_json::from_slice(&fs::read(&args[4])?)?;
        if openpatch::project_document::hosted::validate_supported_snapshot(&seed)?
            != openpatch::project_document::hosted::HostedCapability::XFade
        {
            return Err("XFade baseline snapshot required".into());
        }

        let mut value: Value = serde_json::from_str(include_str!(
            "../../../examples/experimental-hosted-xfade.json"
        ))?;
        for node in value["graph"]["nodes"].as_array_mut().unwrap() {
            if let Some(native) = node.get_mut("node") {
                let id = native["id"].as_str().unwrap().to_owned();
                if id == "source" || id == "source_b" {
                    native["kind"] = json!("constant");
                    native["params"] = json!({"value":if id=="source" {2.0} else {6.0}});
                } else if id == "amp" {
                    native["params"] = json!({"gain":0.125});
                }
            }
        }
        for cable in value["graph"]["cables"].as_array_mut().unwrap() {
            if cable["route"]["from"] == "source" || cable["route"]["from"] == "source_b" {
                cable["route"]["output"] = json!("out");
            }
        }
        let mut document: ProjectDocument = serde_json::from_value(value)?;
        for node in &mut document.graph.nodes {
            if let DocumentNode::Hosted { module, .. } = node {
                if module.state.as_ref() != &seed {
                    return Err("seed differs from genuine default XFade baseline".into());
                }

                module.identity.plugin_sha256 = format!(
                    "{:x}",
                    Sha256::digest(fs::read(args[2].join("plugin.dylib"))?)
                );
                module.identity.runtime_sha256 = format!(
                    "{:x}",
                    Sha256::digest(fs::read(args[1].join("libRack.dylib"))?)
                );
                *module = module
                    .with_parameter("mix", -1.)?
                    .with_parameter("linear", 1.)?;
            }
        }
        let mut session = Session::new(&args[3].join("session"))?;
        session.replace_project_document(document, None)?;
        session.request_delayed_hosted_execution(None)?;
        let ticket = session.audio_snapshot()?;
        let document = ticket.document.clone();
        let DocumentExecutionRequest::HostedDelayed(grant) = &ticket.execution else {
            return Err("explicit grant missing".into());
        };
        let plan = grant.prepare(&document, ticket.document_generation)?;
        let fence = Fence {
            incarnation: 21,
            generation: ticket.document_generation,
            revision: 1,
        };
        let (control, reader) =
            HostedControl::new(grant.clone(), &document, fence, ticket.revision)?;
        session.set_live_module_events(Some(Arc::new(SessionBackend(control.clone()))))?;
        let banks = plan
            .native_patch()
            .prepare_wave_banks(&openpatch_core::WavePreparationCache::default())?;
        let (mut engine, handle) = plan.compile(&banks)?;
        let launch = |snapshot: Value, incarnation, input_mask| {
            LaunchConfig {
                companion_executable: args[0].clone(),
                sdk_dir: args[1].clone(),
                plugin_dir: args[2].clone(),
                workspace_parent: args[3].clone(),
                expected_plugin_sha256: module(&document).identity.plugin_sha256.clone(),
                expected_runtime_sha256: module(&document).identity.runtime_sha256.clone(),
                fence: Fence {
                    incarnation,
                    ..fence
                },
                input_mask,
                snapshot,
                startup_timeout: Duration::from_secs(5),
            }
            .launch()
        };
        let prepared = launch(
            module(&document).state.as_ref().clone(),
            21,
            plan.input_mask(),
        )?;
        let mut process = prepared.process;
        fs::copy(
            process.workspace_path().join("private-user/identity.json"),
            args[3].join("identity.json"),
        )?;
        let mut runner = HostedRunner::prepare_with_control(prepared.endpoint, handle, reader)?;
        assert!(session.acknowledge_audio_snapshot(ticket.id));
        let mut audio = Vec::with_capacity(128 * 64);
        let mut allocations = 0;
        let mut deallocations = 0;
        for block in 0..64 {
            match block {
                8 => {
                    session.execute(
                        "set_hosted_parameter",
                        json!({"id":"binary","parameter":"mix","value":1.}),
                    )?;
                }
                16 => {
                    session.execute(
                        "set_hosted_parameter",
                        json!({"id":"binary","parameter":"mix","value":0.}),
                    )?;
                }
                24 => {
                    session.execute(
                        "set_hosted_parameter",
                        json!({"id":"binary","parameter":"curve","value":1.}),
                    )?;
                }
                32 => {
                    session.execute(
                        "set_hosted_parameter",
                        json!({"id":"binary","parameter":"curve","value":0.75}),
                    )?;
                    session.execute(
                        "set_hosted_parameter",
                        json!({"id":"binary","parameter":"linear","value":0.}),
                    )?;
                }
                40 => {
                    session.execute("undo", json!({}))?;
                }
                48 => {
                    session.execute("redo", json!({}))?;
                }
                _ => {}
            }
            let mut samples = [[0.; 2]; QUANTUM];
            let (result, a, d) = rt(
                || -> std::result::Result<(), openpatch::binary_host::runner::RunnerFault> {
                    runner.validate_callback(SAMPLE_RATE, QUANTUM)?;
                    for sample in &mut samples {
                        *sample = runner.process_frame(&mut engine)?;
                    }
                    Ok(())
                },
            );
            result.map_err(|e| format!("runner {e:?}"))?;
            allocations += a;
            deallocations += d;
            audio.extend(samples.map(|s| s[0]));
            std::thread::sleep(Duration::from_millis(5));
            process.poll_health()?;
        }
        for (block, expected) in [(7, 0.25), (15, 0.75), (23, 0.5), (31, 1.0), (47, 2.0 / 3.0)] {
            let observed = audio[(block + 1) * QUANTUM - 1];
            if (observed - expected).abs() > 1e-5 {
                return Err(format!("block {block}: expected {expected}, got {observed}").into());
            }
        }
        let nonlinear = audio[40 * QUANTUM - 1];
        if !(0.67..1.0).contains(&nonlinear) || (nonlinear - audio[56 * QUANTUM - 1]).abs() > 1e-5 {
            return Err("nonlinear mode did not differ and return after undo/redo".into());
        }
        let applied = control
            .snapshot()
            .applied
            .ok_or("no acknowledged parameters")?;
        if applied.values.values != [0., 0.75, 0.] {
            return Err("all three final values were not acknowledged".into());
        }
        let final_document = Arc::new(session.project_document().clone());
        let authored = module(&final_document).state.as_ref().clone();
        let (mut endpoint, tail, awaiting, fault) = runner.retire();
        assert_eq!(tail, 0);
        assert!(!awaiting);
        assert!(fault.is_none());
        let mut output = OutputBlock::default();
        let deadline = Instant::now() + Duration::from_secs(5);
        while !endpoint.is_drained() {
            if Instant::now() > deadline {
                return Err("drain timeout".into());
            }
            endpoint
                .try_receive(&mut output)
                .map_err(|e| format!("drain {e:?}"))?;
            std::thread::sleep(Duration::from_micros(50));
        }
        let captured = process.finish(endpoint)?;
        assert_eq!(captured.state, authored);
        fs::write(
            args[3].join("captured-state.json"),
            serde_json::to_vec_pretty(&captured.state)?,
        )?;
        // A fresh genuine child must restore exactly all three persistent values.
        let restored = launch(captured.state.clone(), 22, plan.input_mask())?;
        let fresh = restored.process.finish(restored.endpoint)?;
        assert_eq!(fresh.state, captured.state);
        let saved = serde_json::to_value(&*final_document)?;
        let reopened: ProjectDocument = serde_json::from_value(saved)?;
        HostedGraphPlan::prepare(&reopened)?;
        let mut wrong = authored.clone();
        wrong["model"] = json!("Bogaudio-Offset");
        assert!(launch(wrong, 23, plan.input_mask()).is_err());
        // Independent transport check for mono mix CV driving all 16 A/B voices.
        let cv_module = module(&final_document)
            .with_parameter("mix", 1.)?
            .with_parameter("curve", 0.5)?
            .with_parameter("linear", 1.)?;
        let cv_host = launch(cv_module.state.as_ref().clone(), 24, 7)?;
        let mut cv_endpoint = cv_host.endpoint;
        let mut input = openpatch::binary_host::InputBlock::silence(7);
        input.counts[1].fill(16);
        input.counts[2].fill(16);
        for frame in 0..QUANTUM {
            for channel in 0..16 {
                input.values[1][frame][channel] = channel as f32 + 1.;
                input.values[2][frame][channel] = channel as f32 + 5.;
            }
        }
        let mut output = OutputBlock::default();
        for (cv, addition) in [(-5., 1.), (0., 3.), (2.5, 4.), (5., 5.)] {
            for frame in 0..QUANTUM {
                input.values[0][frame][0] = cv;
            }
            for _ in 0..12 {
                let (submitted, a, d) = rt(|| cv_endpoint.submit(&input));
                submitted.map_err(|e| format!("poly CV submit {e:?}"))?;
                allocations += a;
                deallocations += d;
                let deadline = Instant::now() + Duration::from_secs(5);
                loop {
                    let (received, a, d) = rt(|| cv_endpoint.try_receive(&mut output));
                    allocations += a;
                    deallocations += d;
                    if received
                        .map_err(|e| format!("poly CV receive {e:?}"))?
                        .is_some()
                    {
                        break;
                    }
                    if Instant::now() > deadline {
                        return Err("poly CV timeout".into());
                    }
                    std::thread::sleep(Duration::from_micros(50));
                }
            }
            assert_eq!(output.counts[QUANTUM - 1], 16);
            for channel in 0..16 {
                if (output.values[QUANTUM - 1][channel] - (channel as f32 + addition)).abs() > 1e-5
                {
                    return Err(format!("mix CV {cv}, voice {channel} mismatch").into());
                }
            }
        }
        let cv_capture = cv_host.process.finish(cv_endpoint)?;
        assert_eq!(cv_capture.state, cv_module.state.as_ref().clone());
        let result = json!({"passed":allocations==0 && deallocations==0,"profile":"Bogaudio-XFade","frames":audio.len(),"parent_allocations":allocations,"parent_deallocations":deallocations,"checks":["mono CV across poly16 A/B voices","A/B endpoints","linear midpoint","curve","linear mode","shared Session undo/redo","all three acknowledged values","exact persistent capture","fresh binary restore","foreign snapshot rejected"],"qualification":"paced headless, not device deadlines"});
        fs::write(args[3].join("samples.json"), serde_json::to_vec(&audio)?)?;
        fs::write(
            args[3].join("result.json"),
            serde_json::to_vec_pretty(&result)?,
        )?;
        if result["passed"] != true {
            return Err("callback allocations detected".into());
        }
        println!("{result}");
        Ok(())
    }
}
fn main() {
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    if let Err(e) = mac::run() {
        eprintln!("{e}");
        std::process::exit(1);
    }
    #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
    {
        eprintln!("macOS ARM64 only");
        std::process::exit(2);
    }
}
