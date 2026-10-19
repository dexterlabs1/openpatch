//! Genuine installed OFFSET through the production Core graph and hosted runner.
//! This paced control driver is not an application callback or device qualification.
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
mod mac {
    use openpatch::binary_host::control::{HostedControl, HostedControlSnapshot};
    use openpatch::binary_host::graph::HostedGraphPlan;
    use openpatch::binary_host::process::LaunchConfig;
    use openpatch::binary_host::runner::{HostedRunner, RunnerFault, TRANSPORT_DELAY_FRAMES};
    use openpatch::binary_host::{Fence, OutputBlock, Status, QUANTUM, SAMPLE_RATE};
    use openpatch::control::{
        DocumentExecutionRequest, HostedEditAdmission, HostedEditProposal, LiveModuleEvents,
        Session,
    };
    use openpatch::project_document::hosted::{HostedIdentity, HostedModule, HostedState};
    use openpatch::project_document::{
        DocumentCable, DocumentNode, ProjectDocument, ProjectLayout,
    };
    use openpatch_core::{ModuleAction, Patch};
    use serde::Serialize;
    use serde_json::{json, Value};
    use sha2::{Digest, Sha256};
    use std::alloc::{GlobalAlloc, Layout, System};
    use std::cell::Cell;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use std::time::{Duration, Instant};
    type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
    const FRAMES: usize = 2048;
    const EVENT_FRAME: usize = 657;

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
    fn write(path: &Path, value: &impl Serialize) -> Result<()> {
        fs::write(path, serde_json::to_vec_pretty(value)?)?;
        Ok(())
    }
    fn hash(path: &Path) -> Result<String> {
        Ok(format!("{:x}", Sha256::digest(fs::read(path)?)))
    }
    #[derive(Clone, Copy, PartialEq)]
    enum Scenario {
        Impulse,
        Feedback,
        Events,
        LiveSession,
    }
    impl Scenario {
        fn name(self) -> &'static str {
            match self {
                Self::Impulse => "impulse",
                Self::Feedback => "feedback",
                Self::Events => "events",
                Self::LiveSession => "live-session",
            }
        }
        fn lengths(self) -> &'static [usize] {
            match self {
                Self::Impulse => &[128],
                Self::Feedback => &[17, 63, 1, 47],
                Self::Events => &[3, 64, 17, 44],
                Self::LiveSession => &[128],
            }
        }
        fn expected(self, frame: usize) -> f32 {
            match self {
                Self::Impulse => {
                    if frame == 387 {
                        0.78125
                    } else {
                        0.0
                    }
                }
                Self::Feedback => {
                    if frame != 0 && frame.is_multiple_of(388) {
                        let mut value = 0.78125_f32;
                        for _ in 1..frame / 388 {
                            value *= 0.078125;
                        }
                        value
                    } else {
                        0.0
                    }
                }
                Self::Events => {
                    if frame < 387 {
                        0.0
                    } else if frame < 1042 {
                        0.0390625
                    } else {
                        -0.234375
                    }
                }
                Self::LiveSession => match frame {
                    0..387 => 0.0,
                    387..897 => 0.0390625,
                    897..1153 => -0.234375,
                    1153..1409 => 0.234375,
                    1409..1665 => 0.0390625,
                    1665..1921 => 0.234375,
                    _ => -0.234375,
                },
            }
        }
    }
    fn document(kind: Scenario, module: HostedModule) -> Result<ProjectDocument> {
        let constant_source = matches!(kind, Scenario::Events | Scenario::LiveSession);
        let nodes = if constant_source {
            json!([{"id":"source","kind":"constant","params":{"value":0.5}},
                {"id":"gain","kind":"gain","params":{"gain":0.125}}])
        } else if kind == Scenario::Feedback {
            json!([{"id":"push","kind":"push"},{"id":"mixer","kind":"mixer"},
                {"id":"gain","kind":"gain","params":{"gain":0.125}}])
        } else {
            json!([{"id":"push","kind":"push"},
                {"id":"gain","kind":"gain","params":{"gain":0.125}}])
        };
        let native: Patch = serde_json::from_value(json!({"version":2,"sample_rate":48000,
            "nodes":nodes,"cables":[],"output":"gain"}))?;
        let mut doc = ProjectDocument::from_patch(native, ProjectLayout::default())?;
        doc.graph.nodes.push(DocumentNode::Hosted {
            id: "offset".into(),
            source: None,
            module,
        });
        let routes = if kind == Scenario::Feedback {
            json!([
                {"id":"push-mixer","from":"push","output":"gate","to":"mixer","input":"a"},
                {"id":"mixer-offset","from":"mixer","to":"offset","input":"in"},
                {"id":"offset-gain","from":"offset","to":"gain","input":"signal"},
                {"id":"gain-mixer","from":"gain","to":"mixer","input":"b"}
            ])
        } else {
            json!([
                {"id":"source-offset","from":if constant_source {"source"} else {"push"},
                    "output":if constant_source {"out"} else {"gate"},"to":"offset","input":"in"},
                {"id":"offset-gain","from":"offset","to":"gain","input":"signal"}
            ])
        };
        for route in serde_json::from_value::<Vec<openpatch_core::Cable>>(routes)? {
            doc.graph.cables.push(DocumentCable {
                route,
                source: None,
            });
        }
        doc.validate_structure()?;
        Ok(doc)
    }
    #[derive(Serialize)]
    struct Callback {
        first_frame: usize,
        last_frame: usize,
        length: usize,
        duration_ns: u128,
        budget_ns: u128,
        start_lateness_ns: u128,
        overrun: bool,
        allocations: usize,
        deallocations: usize,
    }

    /// Delegates shared Session admission to the production control channel.
    /// Signal processing is exclusively the genuine child and production runner.
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

    fn session_edit(
        session: &mut Session,
        control: &HostedControl,
        frame: usize,
        action: &str,
        args: Value,
        edits: &mut Vec<Value>,
    ) -> Result<()> {
        let before = control.snapshot();
        let started = Instant::now();
        let response = session.execute(action, args.clone())?;
        let elapsed = started.elapsed();
        let after = control.snapshot();
        edits.push(json!({"input_frame":frame,"action":action,"arguments":args,
            "authored_revision":session.revision(),"control_duration_ns":elapsed.as_nanos(),
            "accepted_sequence":after.latest_accepted_sequence,
            "applied_sequence_before":before.applied.map(|p|p.sequence),
            "accepted_before_matching_ack":after.latest_accepted_sequence > after.applied.map_or(0,|p|p.sequence),
            "single_ordered_command":after.latest_accepted_sequence == before.latest_accepted_sequence+1
                && after.queued.is_some_and(|r|r.commands==1 && r.authored_revision==session.revision()),
            "requested":after.requested,"response_revision":response.get("revision")}));
        Ok(())
    }
    fn scenario(
        args: &[PathBuf],
        module: &HostedModule,
        kind: Scenario,
        incarnation: u64,
    ) -> Result<Value> {
        let directory = args[3].join(kind.name());
        fs::create_dir(&directory)?;
        let doc = document(kind, module.clone())?;
        let document_path = directory.join("project.json");
        fs::write(&document_path, doc.encode()?)?;
        let mut session = if kind == Scenario::LiveSession {
            let mut session = Session::new(&directory)?;
            session.replace_project_document(doc.clone(), None)?;
            session.request_delayed_hosted_execution(None)?;
            Some(session)
        } else {
            None
        };
        let ticket = session.as_mut().map(Session::audio_snapshot).transpose()?;
        let fence = Fence {
            incarnation,
            generation: ticket.as_ref().map_or(9, |t| t.document_generation),
            revision: 11,
        };
        let (plan, control_pair) = if let Some(ticket) = &ticket {
            let DocumentExecutionRequest::HostedDelayed(grant) = &ticket.execution else {
                return Err("live Session did not retain its execution grant".into());
            };
            (
                grant.prepare(&doc, ticket.document_generation)?,
                Some(HostedControl::new(
                    grant.clone(),
                    &doc,
                    fence,
                    ticket.revision,
                )?),
            )
        } else {
            (HostedGraphPlan::prepare(&doc)?, None)
        };
        let (control, reader) = match control_pair {
            Some((control, reader)) => {
                session
                    .as_mut()
                    .unwrap()
                    .set_live_module_events(Some(Arc::new(SessionBackend(Arc::clone(&control)))))?;
                (Some(control), Some(reader))
            }
            None => (None, None),
        };
        let banks = plan
            .native_patch()
            .prepare_wave_banks(&openpatch_core::WavePreparationCache::default())?;
        let (mut engine, handle) = plan.compile(&banks)?;
        write(
            &directory.join("compiled-cables.json"),
            &engine.host_cable_registry(),
        )?;
        let prepared = LaunchConfig {
            companion_executable: args[0].clone(),
            sdk_dir: args[1].clone(),
            plugin_dir: args[2].clone(),
            workspace_parent: directory.clone(),
            expected_plugin_sha256: module.identity.plugin_sha256.clone(),
            expected_runtime_sha256: module.identity.runtime_sha256.clone(),
            fence,
            input_mask: plan.input_mask(),
            snapshot: plan.module().state.as_ref().clone(),
            startup_timeout: Duration::from_secs(5),
        }
        .launch()?;
        let mut process = prepared.process;
        fs::copy(
            process.workspace_path().join("private-user/identity.json"),
            directory.join("identity.json"),
        )?;
        let mut runner = if let Some(reader) = reader {
            HostedRunner::prepare_with_control(prepared.endpoint, handle, reader)?
        } else {
            HostedRunner::prepare(prepared.endpoint, handle)?
        };
        let push = if !matches!(kind, Scenario::Events | Scenario::LiveSession) {
            Some(engine.resolve_module_action("push", ModuleAction::PushPress)?)
        } else {
            None
        };
        let frames = if kind == Scenario::LiveSession {
            3072
        } else {
            FRAMES
        };
        let mut audio = vec![[0.0; 2]; frames];
        let mut callbacks = Vec::with_capacity(frames);
        let mut control_snapshots: Vec<(usize, HostedControlSnapshot)> = Vec::new();
        let mut shared_edits = Vec::new();
        let origin = Instant::now() + Duration::from_millis(2);
        let mut first = 0;
        let mut callback_index = 0;
        let mut render_error = None;
        let mut previous_start = None;
        while first < frames {
            let length = kind.lengths()[callback_index % kind.lengths().len()].min(frames - first);
            if let (Some(session), Some(control)) = (&mut session, &control) {
                match first {
                    512 => {
                        session_edit(
                            session,
                            control,
                            first,
                            "set_hosted_parameter",
                            json!({"id":"offset","parameter":"offset","value":0.25}),
                            &mut shared_edits,
                        )?;
                        session_edit(
                            session,
                            control,
                            first,
                            "set_hosted_parameter",
                            json!({"id":"offset","parameter":"scale","value":-0.25}),
                            &mut shared_edits,
                        )?;
                    }
                    768 | 1024 => session_edit(
                        session,
                        control,
                        first,
                        "undo",
                        json!({}),
                        &mut shared_edits,
                    )?,
                    1280 | 1536 => session_edit(
                        session,
                        control,
                        first,
                        "redo",
                        json!({}),
                        &mut shared_edits,
                    )?,
                    1792 => session_edit(
                        session,
                        control,
                        first,
                        "set_hosted_parameter",
                        json!({"id":"offset","parameter":"scale","value":-0.25}),
                        &mut shared_edits,
                    )?,
                    _ => {}
                }
            }
            let target = if kind == Scenario::LiveSession {
                previous_start.map_or(origin, |start: Instant| start + Duration::from_millis(5))
            } else {
                origin + Duration::from_secs_f64(first as f64 / f64::from(SAMPLE_RATE))
            };
            // Control driver only: never wait for the companion within a callback.
            if let Some(delay) = target.checked_duration_since(Instant::now()) {
                std::thread::sleep(delay);
            }
            if process.poll_health()? != Status::Ready {
                return Err("companion not READY before callback".into());
            }
            let started = Instant::now();
            previous_start = Some(started);
            let (result, allocations, deallocations) =
                rt(|| -> std::result::Result<(), RunnerFault> {
                    runner.validate_callback(SAMPLE_RATE, length)?;
                    for (offset, sample) in audio[first..first + length].iter_mut().enumerate() {
                        let frame = first + offset;
                        if let Some(push) = push {
                            if frame < 2
                                && !engine.handle_module_event(
                                    push,
                                    if frame == 0 {
                                        ModuleAction::PushPress
                                    } else {
                                        ModuleAction::PushRelease
                                    },
                                )
                            {
                                return Err(RunnerFault::Boundary);
                            }
                        }
                        if kind == Scenario::Events && frame == EVENT_FRAME {
                            runner.queue_parameter(0, 0.25, 1)?;
                            runner.queue_parameter(1, -0.25, 2)?;
                        }
                        *sample = runner.process_frame(&mut engine)?;
                    }
                    Ok(())
                });
            let elapsed = started.elapsed();
            let budget = Duration::from_secs_f64(length as f64 / f64::from(SAMPLE_RATE));
            callbacks.push(Callback {
                first_frame: first,
                last_frame: first + length - 1,
                length,
                duration_ns: elapsed.as_nanos(),
                budget_ns: budget.as_nanos(),
                start_lateness_ns: started.saturating_duration_since(target).as_nanos(),
                overrun: elapsed > budget,
                allocations,
                deallocations,
            });
            if let Some(control) = &control {
                control_snapshots.push((first + length - 1, control.snapshot()));
            }
            if let Err(error) = result {
                render_error = Some(format!("frame {}: {error:?}", runner.frame()));
                break;
            }
            first += length;
            callback_index += 1;
        }
        let rendered_frames = runner.frame();
        let final_live_control = control.as_ref().map(|control| control.snapshot());
        let (mut endpoint, tail, awaiting, fault) = runner.retire();
        let submitted = endpoint.submitted_blocks();
        let received_before_drain = endpoint.received_blocks();
        // Retired control side: drain acknowledgements without rendering/padding.
        let mut drained = OutputBlock::default();
        let drain_start = Instant::now();
        while !endpoint.is_drained() && endpoint.fault().is_none() {
            match endpoint.try_receive(&mut drained) {
                Ok(Some(_)) => {}
                Ok(None) => {
                    if drain_start.elapsed() > Duration::from_secs(5) {
                        return Err("retired drain timeout".into());
                    }
                    process.poll_health()?;
                    std::thread::sleep(Duration::from_micros(50));
                }
                Err(error) => {
                    render_error.get_or_insert(format!("retired drain: {error:?}"));
                    break;
                }
            }
        }
        let received_after_drain = endpoint.received_blocks();
        let captured = process.finish(endpoint);
        let mono: Vec<f32> = audio.iter().map(|frame| frame[0]).collect();
        let trace_path = directory.join("samples.json");
        write(&trace_path, &mono)?;
        write(&directory.join("callbacks.json"), &callbacks)?;
        let wav_path = directory.join("output.wav");
        openpatch_core::write_wav(&wav_path, &mono, SAMPLE_RATE)?;
        let capture_path = directory.join("captured-state.json");
        let mut expected_state = module.state.as_ref().clone();
        if matches!(kind, Scenario::Events | Scenario::LiveSession) {
            expected_state["params"][0]["value"] = json!(0.25);
            expected_state["params"][1]["value"] = json!(-0.25);
        }
        let captured_state = captured.as_ref().ok().map(|capture| capture.state.clone());
        let (capture_matches, capture_error, captured_sha) = match captured {
            Ok(capture) => {
                write(&capture_path, &capture.state)?;
                (
                    capture.state == expected_state,
                    None,
                    Some(hash(&capture_path)?),
                )
            }
            Err(error) => (false, Some(error.to_string()), None),
        };
        let mut live_passed = true;
        let live_evidence = if let (Some(session), Some(control), Some(final_control)) =
            (&mut session, &control, &final_live_control)
        {
            write(
                &directory.join("control-observations.json"),
                &control_snapshots,
            )?;
            write(&directory.join("shared-edits.json"), &shared_edits)?;
            let authored_plan = HostedGraphPlan::prepare(session.project_document())?;
            let authored_state = authored_plan.module().state.as_ref();
            let authored_matches_capture = captured_state.as_ref() == Some(authored_state);
            let first_final_ack = control_snapshots.iter().find_map(|(frame, snapshot)| {
                snapshot
                    .applied
                    .filter(|applied| applied.sequence == 7)
                    .map(|applied| (*frame, applied))
            });
            let ordered = shared_edits.len() == 7
                && shared_edits.iter().enumerate().all(|(index, edit)| {
                    edit["accepted_sequence"] == json!(index + 1)
                        && edit["single_ordered_command"] == true
                        && edit["accepted_before_matching_ack"] == true
                });
            let acknowledgments_valid = control_snapshots.iter().all(|(_, snapshot)| {
                snapshot.applied.is_none_or(|applied| {
                    applied.sequence <= snapshot.latest_accepted_sequence
                        && (applied.sequence == 0
                            || applied.input_frame / (QUANTUM as u64) < applied.received_blocks)
                })
            });
            let final_ack_valid = first_final_ack.is_some_and(|(_, applied)| {
                applied.input_frame == 1792
                    && applied.received_blocks >= 15
                    && applied.observed_frame >= 2176
            });
            let effective_matches_authored = final_control.applied.is_some_and(|applied| {
                applied.sequence == 7
                    && applied.authored_revision == session.revision()
                    && applied.values.offset == authored_plan.module().parameter("offset").unwrap()
                    && applied.values.scale == authored_plan.module().parameter("scale").unwrap()
            });
            let mut reopen_matches = false;
            let mut fresh_restore_matches = false;
            let mut saved_sha = None;
            if authored_matches_capture {
                session.execute(
                    "save_document",
                    json!({"filename":"live-authored.json","expected_revision":session.revision()}),
                )?;
                saved_sha = Some(hash(&directory.join("live-authored.json"))?);
                let mut fresh = Session::new(&directory)?;
                fresh.execute("load_document", json!({"filename":"live-authored.json"}))?;
                reopen_matches = serde_json::to_value(fresh.project_document())?
                    == serde_json::to_value(session.project_document())?;
                let reopened = HostedGraphPlan::prepare(fresh.project_document())?;
                let restored = LaunchConfig {
                    companion_executable: args[0].clone(),
                    sdk_dir: args[1].clone(),
                    plugin_dir: args[2].clone(),
                    workspace_parent: directory.clone(),
                    expected_plugin_sha256: module.identity.plugin_sha256.clone(),
                    expected_runtime_sha256: module.identity.runtime_sha256.clone(),
                    fence: Fence {
                        incarnation: incarnation + 1000,
                        generation: fresh.project_save_baseline().generation(),
                        revision: 12,
                    },
                    input_mask: reopened.input_mask(),
                    snapshot: reopened.module().state.as_ref().clone(),
                    startup_timeout: Duration::from_secs(5),
                }
                .launch()?;
                // A fresh genuine process restores and captures the reopened
                // full snapshot; this does not repeat the graph DSP run.
                let fresh_capture = restored.process.finish(restored.endpoint)?;
                write(
                    &directory.join("fresh-restored-state.json"),
                    &fresh_capture.state,
                )?;
                fresh_restore_matches = fresh_capture.state == *reopened.module().state.as_ref();
            }
            let retired = control.snapshot();
            live_passed = ordered
                && acknowledgments_valid
                && final_ack_valid
                && effective_matches_authored
                && final_control.available
                && final_control.outstanding_commands == 0
                && authored_matches_capture
                && retired.closed
                && !retired.available
                && reopen_matches
                && fresh_restore_matches;
            json!({"passed":live_passed,"scope":"paced shared Session edit/history and genuine child correctness; no application Callback or device qualification",
                "minimum_callback_start_gap_ns":5_000_000,"authoring_outside_tracked_realtime":true,
                "seven_ordered_commands":ordered,"accepted_commands_waited_for_matching_ack":ordered,
                "acknowledgment_watermarks_valid":acknowledgments_valid,"first_sequence7_observation":first_final_ack,
                "sequence7_requires_acknowledgment_of_input_block14":final_ack_valid,"final_before_retirement":final_control,
                "after_retirement":retired,"effective_matches_authored":effective_matches_authored,
                "authored_full_state_matches_retired_child":authored_matches_capture,
                "fresh_session_load_preserves_full_identity_flags_params_routes":reopen_matches,
                "fresh_process_restored_reopened_full_state":fresh_restore_matches,"saved_document_sha256":saved_sha,
                "declared_output_transitions":[[0,0.0],[387,0.0390625],[897,-0.234375],[1153,0.234375],[1409,0.0390625],[1665,0.234375],[1921,-0.234375]],
                "edits":shared_edits})
        } else {
            Value::Null
        };
        let first_mismatch = audio.iter().enumerate().find(|(frame, sample)| **sample != [kind.expected(*frame);2])
            .map(|(frame, sample)| json!({"frame":frame,"actual":sample,"expected":kind.expected(frame)}));
        let nonzero: Vec<_> = mono
            .iter()
            .enumerate()
            .filter(|(_, v)| **v != 0.0)
            .map(|(frame, value)| json!({"frame":frame,"value":value}))
            .collect();
        let transitions: Vec<_> = mono
            .iter()
            .enumerate()
            .filter(|(frame, value)| *frame == 0 || **value != mono[*frame - 1])
            .map(|(frame, value)| json!({"frame":frame,"value":value}))
            .collect();
        let allocations: usize = callbacks.iter().map(|c| c.allocations).sum();
        let deallocations: usize = callbacks.iter().map(|c| c.deallocations).sum();
        let passed = first_mismatch.is_none()
            && render_error.is_none()
            && fault.is_none()
            && tail == 0
            && !awaiting
            && rendered_frames == frames as u64
            && capture_matches
            && allocations == 0
            && deallocations == 0
            && live_passed;
        let result = json!({"passed":passed,"scenario":kind.name(),"rendered_frames":rendered_frames,
            "declared_transport_delay_frames":TRANSPORT_DELAY_FRAMES,"declared_first_signal_frame":if kind == Scenario::Feedback {388}else{387},
            "measured_first_signal_frame":mono.iter().position(|value| *value != 0.0),
            "declared_feedback_cycle_frames":if kind == Scenario::Feedback {Some(388)}else{None},
            "declared_event_input_frame":if kind == Scenario::Events {Some(EVENT_FRAME)}else{None},
            "declared_event_output_frame":if kind == Scenario::Events {Some(1042)}else{None},
            "measured_event_output_frame":if kind == Scenario::Events {mono.iter().position(|value| *value == -0.234375)}else{None},
            "nonzero_samples":if matches!(kind,Scenario::Events|Scenario::LiveSession) {Value::Null}else{json!(nonzero)},"transitions":transitions,
            "first_mismatch":first_mismatch,"render_error":render_error,"runner_fault":format!("{fault:?}"),
            "unsubmitted_tail_frames":tail,"awaiting_inputs_at_retirement":awaiting,"submitted_blocks":submitted,
            "received_before_control_drain":received_before_drain,"received_after_control_drain":received_after_drain,
            "callback_lengths_pattern":kind.lengths(),"callback_count":callbacks.len(),
            "callback_first_frame":callbacks.first().map(|c|c.first_frame),"callback_last_frame":callbacks.last().map(|c|c.last_frame),
            "callback_max_duration_ns":callbacks.iter().map(|c|c.duration_ns).max(),
            "callback_overruns":callbacks.iter().filter(|c|c.overrun).count(),"callback_allocations":allocations,"callback_deallocations":deallocations,
            "live_session":live_evidence,
            "capture_matches_expected_params_and_custom_flags":capture_matches,"capture_error":capture_error,"capture_sha256":captured_sha,
            "wav_sha256":hash(&wav_path)?,"sample_trace_sha256":hash(&trace_path)?,"document_sha256":hash(&document_path)?,
            "wav_non_silent":mono.iter().any(|v|*v!=0.0),"directory":directory});
        write(&directory.join("result.json"), &result)?;
        Ok(result)
    }
    pub fn run() -> Result<()> {
        let args: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
        if args.len() != 5 {
            return Err(
                "usage: binary_host_graph COMPANION SDK PLUGIN OUTPUT_DIR SEED_STATE".into(),
            );
        }
        fs::create_dir_all(&args[3])?;
        let identity = HostedIdentity {
            plugin: "Bogaudio".into(),
            model: "Bogaudio-Offset".into(),
            version: "2.6.47".into(),
            rack_version: "2.6.6".into(),
            plugin_sha256: hash(&args[2].join("plugin.dylib"))?,
            runtime_sha256: hash(&args[1].join("libRack.dylib"))?,
        };
        let mut seed: Value = serde_json::from_slice(&fs::read(&args[4])?)?;
        openpatch::project_document::hosted::validate_offset_snapshot(&seed)?;
        seed["params"] = json!([{"id":0,"value":0.0},{"id":1,"value":0.25}]);
        seed["data"] = json!({"offset_first":true,"disableOutputLimit":false});
        let mut module = HostedModule::offset(identity, 0.0, 0.25, true, false)?;
        module.state = HostedState::new(seed)?;
        module.validate_supported_offset()?;
        write(&args[3].join("initial-state.json"), module.state.as_ref())?;
        let mut scenarios = Vec::new();
        for (index, kind) in [
            Scenario::Impulse,
            Scenario::Feedback,
            Scenario::Events,
            Scenario::LiveSession,
        ]
        .into_iter()
        .enumerate()
        {
            scenarios.push(scenario(&args, &module, kind, index as u64 + 100)?);
        }
        let passed = scenarios.iter().all(|s| s["passed"] == true);
        let result = json!({"passed":passed,"gate":"production core mixed-graph runner correctness; application callback/device integration pending",
            "build":if cfg!(debug_assertions){"debug"}else{"release"},"rate":SAMPLE_RATE,"maximum_whole_callback_frames":128,
            "transport_quantum_frames":QUANTUM,"transport_delay_frames":TRANSPORT_DELAY_FRAMES,"identity":module.identity,
            "companion_sha256":hash(&args[0])?,"input_seed_sha256":hash(&args[4])?,"scenarios":scenarios,
            "realtime_deadlines_qualified":false,"application_callback_integration":false,"native_window_or_device_test":false});
        write(&args[3].join("result.json"), &result)?;
        println!("{}", serde_json::to_string(&result)?);
        if !passed {
            return Err("mixed-graph gate differed from declared sample contract; retained traces and result.json".into());
        }
        Ok(())
    }
}
fn main() {
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    if let Err(error) = mac::run() {
        eprintln!("binary host graph gate: {error}");
        std::process::exit(1);
    }
    #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
    {
        eprintln!("binary host graph gate requires macOS ARM64");
        std::process::exit(1);
    }
}
