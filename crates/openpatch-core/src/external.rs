//! Explicit graph boundaries for processors owned by an external runtime.
//! Compilation and resolution run on the control thread. Frame exchange borrows
//! fixed caller storage and performs no allocation, transport, or runtime calls.
use crate::{
    Cable, Engine, EngineKind, ListenSink, ModuleHandle, Patch, PortFrame, PreparedWaveBanks,
    ResolvedTap, TapSample, MAX_CHANNELS, MAX_NODES,
};
use std::sync::atomic::Ordering;

/// A real graph vertex with named ports, independent of the native catalog.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExternalModuleSpec {
    pub id: String,
    pub inputs: Vec<String>,
    pub outputs: Vec<String>,
}

impl ExternalModuleSpec {
    pub(crate) fn validate(&self) -> Result<(), String> {
        if self.id.is_empty()
            || self.id.len() > 64
            || !self
                .id
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-' | b'.'))
        {
            return Err(
                "external node IDs must contain 1..=64 ASCII letters, digits, '_', '-', or '.'"
                    .into(),
            );
        }
        for ports in [&self.inputs, &self.outputs] {
            if ports.len() > 16 {
                return Err(format!(
                    "external node '{}': at most 16 ports per direction",
                    self.id
                ));
            }
            for (index, port) in ports.iter().enumerate() {
                if port.is_empty()
                    || port.len() > 64
                    || !port
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-' | b'.'))
                {
                    return Err(format!("external node '{}': invalid port name", self.id));
                }
                if ports[..index].contains(port) {
                    return Err(format!(
                        "external node '{}': duplicate port '{port}'",
                        self.id
                    ));
                }
            }
        }
        Ok(())
    }
}

/// Bound to exactly one compiled engine, including its fixed port topology.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResolvedExternalModule {
    pub(crate) owner: u64,
    pub(crate) node: usize,
    inputs: u8,
    outputs: u8,
    input_presence: u16,
    output_consumers: u16,
}
impl ResolvedExternalModule {
    pub fn input_count(self) -> usize {
        usize::from(self.inputs)
    }
    pub fn output_count(self) -> usize {
        usize::from(self.outputs)
    }
    /// Physical cable presence; connected inputs can carry mono zero.
    pub fn input_presence(self) -> u16 {
        self.input_presence
    }
    pub fn output_consumers(self) -> u16 {
        self.output_consumers
    }
}

/// Supply outputs for this sample; receive the canonical raw inputs consumed
/// at its start, before end-of-sample cable transfer. Inactive lanes are zero.
/// Supplied outputs authorize exactly one sample. Ordinary processing entry
/// points silence the external producer while consuming valid previous edges.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ExternalFrameIo {
    pub handle: ResolvedExternalModule,
    pub outputs: [PortFrame; 16],
    pub inputs: [PortFrame; 16],
}
impl ExternalFrameIo {
    pub fn new(handle: ResolvedExternalModule) -> Self {
        Self {
            handle,
            outputs: [PortFrame::default(); 16],
            inputs: [PortFrame::default(); 16],
        }
    }
}

/// Rejected atomically before the engine or caller input storage changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExternalFrameError {
    RequiresVersionTwo,
    ForeignHandle,
    DuplicateHandle,
    IncompleteFrameSet,
    InvalidOutputFrame,
}
impl std::fmt::Display for ExternalFrameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::RequiresVersionTwo => "external frames require a version-2 engine",
            Self::ForeignHandle => "external module handle belongs to another engine",
            Self::DuplicateHandle => "duplicate external module frame",
            Self::IncompleteFrameSet => "one frame is required for every external module",
            Self::InvalidOutputFrame => {
                "external output must have 0..=16 finite active lanes and zero inactive lanes"
            }
        })
    }
}
impl std::error::Error for ExternalFrameError {}

impl Engine {
    /// Compile native vertices/parameters/assets plus explicit external vertices.
    /// `routes` is the complete graph cable set and `output` its final sink.
    /// Native nodes keep their original order and external vertices are appended.
    pub fn compile_with_externals(
        patch: &Patch,
        specs: &[ExternalModuleSpec],
        routes: &[Cable],
        output: Option<&str>,
        output_port: &str,
        banks: &PreparedWaveBanks,
    ) -> Result<Self, String> {
        if patch.version != 2 {
            return Err("external modules require patch version 2".into());
        }
        patch.validate_compilation(banks)?;
        crate::validate_cable_ids(routes)?;
        let inner = crate::poly::PolyEngine::compile_with_externals(
            patch,
            banks,
            specs,
            routes,
            output,
            output_port,
        )?;
        Ok(Self {
            inner: EngineKind::Poly(Box::new(inner)),
            identity: crate::NEXT_ENGINE_ID.fetch_add(1, Ordering::Relaxed),
        })
    }

    pub fn resolve_external_module(&self, id: &str) -> Result<ResolvedExternalModule, String> {
        let layout = match &self.inner {
            EngineKind::Poly(engine) => engine.external_layout(id),
            EngineKind::Legacy(_) => None,
        };
        let (node, inputs, outputs, input_presence, consumers) =
            layout.ok_or_else(|| format!("module '{id}' is not an external module"))?;
        Ok(ResolvedExternalModule {
            owner: self.identity,
            node,
            inputs,
            outputs,
            input_presence,
            output_consumers: consumers as u16,
        })
    }

    pub fn process_audio_frame_with_externals(
        &mut self,
        frames: &mut [ExternalFrameIo],
    ) -> Result<[f32; 2], ExternalFrameError> {
        self.process_audio_frame_with_externals_taps_and_listen(frames, &[], &mut [], None)
    }

    /// Execute the ordinary sample path with its existing taps and Listen
    /// processing point. All external handles and output frames are validated
    /// before mutation. Every external vertex must be supplied exactly once.
    /// Stale ordinary tap/Listen handles retain their usual inactive behavior.
    pub fn process_audio_frame_with_externals_taps_and_listen<const N: usize>(
        &mut self,
        frames: &mut [ExternalFrameIo],
        taps: &[Option<ResolvedTap>; N],
        samples: &mut [TapSample; N],
        listen: Option<(ModuleHandle, &mut dyn ListenSink)>,
    ) -> Result<[f32; 2], ExternalFrameError> {
        let EngineKind::Poly(engine) = &mut self.inner else {
            return Err(ExternalFrameError::RequiresVersionTwo);
        };
        if frames.len() != engine.external_count() {
            return Err(ExternalFrameError::IncompleteFrameSet);
        }
        let mut seen = [false; MAX_NODES];
        for frame in frames.iter() {
            let handle = frame.handle;
            if handle.owner != self.identity || !engine.is_external(handle.node) {
                return Err(ExternalFrameError::ForeignHandle);
            }
            if seen[handle.node] {
                return Err(ExternalFrameError::DuplicateHandle);
            }
            seen[handle.node] = true;
            for (index, output) in frame.outputs.iter().enumerate() {
                let count = usize::from(output.channels);
                if count > MAX_CHANNELS
                    || (index >= handle.output_count() && count != 0)
                    || !output.values[..count.min(MAX_CHANNELS)]
                        .iter()
                        .all(|value| value.is_finite())
                    || !output.values[count.min(MAX_CHANNELS)..]
                        .iter()
                        .all(|value| *value == 0.0)
                {
                    return Err(ExternalFrameError::InvalidOutputFrame);
                }
            }
        }
        engine.begin_taps(&taps.map(|tap| {
            tap.filter(|tap| tap.owner == self.identity)
                .map(|tap| tap.index)
        }));
        engine.begin_external_frame(frames);
        let (index, sink) = match listen {
            Some((handle, sink)) => (
                (handle.generation == self.identity).then_some(handle.node),
                Some(sink),
            ),
            None => (None, None),
        };
        let frame = engine.process_sample_with_listen_capture(index, sink);
        for (sample, tap) in samples.iter_mut().zip(taps) {
            *sample = tap
                .filter(|tap| tap.owner == self.identity)
                .map_or(TapSample::default(), |tap| {
                    engine.tap_sample(tap.index, tap.channel)
                });
        }
        if N != 0 {
            engine.end_taps();
        }
        let left = frame.channel(0);
        Ok([
            left,
            if engine.output_channels() == 2 {
                frame.channel(1)
            } else {
                left
            },
        ])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn spec(id: &str, inputs: &[&str], outputs: &[&str]) -> ExternalModuleSpec {
        ExternalModuleSpec {
            id: id.into(),
            inputs: inputs.iter().map(|s| (*s).into()).collect(),
            outputs: outputs.iter().map(|s| (*s).into()).collect(),
        }
    }
    fn patch(nodes: serde_json::Value) -> Patch {
        serde_json::from_value(json!({"version":2,"nodes":nodes})).unwrap()
    }
    fn cable(id: &str, from: &str, output: &str, to: &str, input: &str) -> Cable {
        serde_json::from_value(json!({"id":id,"from":from,"output":output,"to":to,"input":input}))
            .unwrap()
    }
    fn compile(
        patch: &Patch,
        specs: &[ExternalModuleSpec],
        routes: &[Cable],
        output: Option<&str>,
        port: &str,
    ) -> Engine {
        let banks = patch
            .prepare_wave_banks(&crate::WavePreparationCache::default())
            .unwrap();
        Engine::compile_with_externals(patch, specs, routes, output, port, &banks).unwrap()
    }

    #[test]
    fn native_external_native_uses_the_normal_delayed_edges_and_registry() {
        let patch = patch(json!([
            {"id":"source","kind":"poly_constant","params":{"channels":1,"v1":2}},
            {"id":"sink","kind":"gain","params":{"gain":3}}
        ]));
        let routes = [
            cable("incoming", "source", "out", "binary", "in"),
            cable("outgoing", "binary", "out", "sink", "signal"),
        ];
        let mut engine = compile(
            &patch,
            &[spec("binary", &["in"], &["out"])],
            &routes,
            Some("sink"),
            "out",
        );
        let handle = engine.resolve_external_module("binary").unwrap();
        assert_eq!(handle.input_presence(), 1);
        assert_eq!(handle.output_consumers(), 1);
        assert_eq!(
            engine
                .host_cable_registry()
                .unwrap()
                .iter()
                .map(|c| c.id.as_deref())
                .collect::<Vec<_>>(),
            vec![Some("incoming"), Some("outgoing")]
        );
        let mut io = [ExternalFrameIo::new(handle)];
        io[0].outputs[0] = PortFrame::mono(10.0);
        assert_eq!(
            engine.process_audio_frame_with_externals(&mut io).unwrap(),
            [0.0; 2]
        );
        assert_eq!(io[0].inputs[0], PortFrame::mono(0.0));
        io[0].outputs[0] = PortFrame::mono(11.0);
        assert_eq!(
            engine.process_audio_frame_with_externals(&mut io).unwrap(),
            [30.0; 2]
        );
        assert_eq!(io[0].inputs[0], PortFrame::mono(2.0));
        assert_eq!(
            engine.process_audio_frame_with_externals(&mut io).unwrap(),
            [33.0; 2]
        );
    }

    #[test]
    fn raw_boundary_inputs_preserve_connected_zero_normaling_and_poly_changes() {
        let patch = Patch::new_v2();
        let specs = [
            spec("source", &[], &["out"]),
            spec("sink", &["in", "normal"], &[]),
        ];
        let mut engine = compile(
            &patch,
            &specs,
            &[cable("route", "source", "out", "sink", "in")],
            None,
            "out",
        );
        let mut io = [
            ExternalFrameIo::new(engine.resolve_external_module("source").unwrap()),
            ExternalFrameIo::new(engine.resolve_external_module("sink").unwrap()),
        ];
        let poly = PortFrame {
            channels: 16,
            values: [4.0; 16],
        };
        for (supplied, consumed) in [
            (poly, PortFrame::mono(0.0)),
            (PortFrame::mono(7.0), poly),
            (PortFrame::default(), PortFrame::mono(7.0)),
            (poly, PortFrame::mono(0.0)),
            (poly, poly),
        ] {
            io[0].outputs[0] = supplied;
            engine.process_audio_frame_with_externals(&mut io).unwrap();
            assert_eq!(io[1].inputs[0], consumed);
            assert_eq!(io[1].inputs[1], PortFrame::default());
        }
        assert_eq!(io[1].handle.input_presence(), 1);
    }

    #[test]
    fn feedback_and_stacked_routes_keep_canonical_sample_order() {
        let patch =
            patch(json!([{ "id":"dc", "kind":"poly_constant", "params":{"channels":1,"v1":2}}]));
        let mut engine = compile(
            &patch,
            &[spec("loop", &["in"], &["out"])],
            &[
                cable("dc", "dc", "out", "loop", "in"),
                cable("feedback", "loop", "out", "loop", "in"),
            ],
            Some("loop"),
            "out",
        );
        let mut io = [ExternalFrameIo::new(
            engine.resolve_external_module("loop").unwrap(),
        )];
        for (sent, expected) in [(1.0, 0.0), (3.0, 3.0), (5.0, 5.0), (7.0, 7.0)] {
            io[0].outputs[0] = PortFrame::mono(sent);
            assert_eq!(
                engine.process_audio_frame_with_externals(&mut io).unwrap(),
                [sent; 2]
            );
            assert_eq!(io[0].inputs[0], PortFrame::mono(expected));
        }
    }

    #[test]
    fn native_only_compilation_and_frame_wrapper_match_existing_processing() {
        let mut patch = patch(json!([
            {"id":"source","kind":"poly_constant","params":{"channels":3,"v1":2,"v2":-4,"v3":9}},
            {"id":"sink","kind":"gain","params":{"gain":0.5}}
        ]));
        patch.cables = vec![cable("native", "source", "out", "sink", "signal")];
        patch.output = Some("sink".into());
        let mut existing = patch.compile().unwrap();
        let mut explicit = compile(&patch, &[], &patch.cables, patch.output.as_deref(), "out");
        for _ in 0..64 {
            let mut audio = [[0.0; 2]];
            existing.process_audio_block(&mut audio);
            assert_eq!(
                explicit
                    .process_audio_frame_with_externals(&mut [])
                    .unwrap(),
                audio[0]
            );
        }
    }

    #[test]
    fn invalid_specs_routes_and_unknown_native_kinds_are_rejected() {
        let mut patch = Patch::new_v2();
        let banks = patch
            .prepare_wave_banks(&crate::WavePreparationCache::default())
            .unwrap();
        let rejects = |specs: &[ExternalModuleSpec], routes: &[Cable]| {
            assert!(
                Engine::compile_with_externals(&patch, specs, routes, None, "out", &banks).is_err()
            );
        };
        rejects(&[spec("bad id", &[], &[])], &[]);
        rejects(&[spec("x", &["a", "a"], &[])], &[]);
        rejects(&[spec("x", &[], &[""])], &[]);
        rejects(&[spec("x", &[], &[]), spec("x", &[], &[])], &[]);
        let mut excess = spec("x", &[], &[]);
        excess.inputs = (0..17).map(|i| format!("p{i}")).collect();
        rejects(&[excess], &[]);
        rejects(
            &[spec("x", &[], &["out"])],
            &[cable("bad", "x", "missing", "x", "in")],
        );
        rejects(
            &[spec("x", &["in"], &["out"])],
            &[
                cable("same", "x", "out", "x", "in"),
                cable("same", "x", "out", "x", "in"),
            ],
        );
        patch.nodes =
            serde_json::from_value(json!([{ "id":"x", "kind":"not_a_native_kind" }])).unwrap();
        assert!(Engine::compile_with_externals(&patch, &[], &[], None, "out", &banks).is_err());
        patch.nodes[0].kind = "gain".into();
        assert!(Engine::compile_with_externals(
            &patch,
            &[spec("x", &[], &[])],
            &[],
            None,
            "out",
            &banks
        )
        .is_err());
        patch.version = 1;
        assert!(Engine::compile_with_externals(&patch, &[], &[], None, "out", &banks).is_err());
    }

    #[test]
    fn foreign_missing_duplicate_and_malformed_frames_reject_without_mutation() {
        let patch = Patch::new_v2();
        let specs = [spec("a", &["in"], &["out"]), spec("b", &[], &["out"])];
        let routes = [cable("route", "a", "out", "a", "in")];
        let mut engine = compile(&patch, &specs, &routes, Some("a"), "out");
        let foreign = compile(&patch, &specs, &routes, Some("a"), "out");
        let mut io = [
            ExternalFrameIo::new(engine.resolve_external_module("a").unwrap()),
            ExternalFrameIo::new(engine.resolve_external_module("b").unwrap()),
        ];
        assert_eq!(
            engine.process_audio_frame_with_externals(&mut io[..1]),
            Err(ExternalFrameError::IncompleteFrameSet)
        );
        let local = io[0].handle;
        io[0].handle = foreign.resolve_external_module("a").unwrap();
        let before = io;
        assert_eq!(
            engine.process_audio_frame_with_externals(&mut io),
            Err(ExternalFrameError::ForeignHandle)
        );
        assert_eq!(io, before);
        io[0].handle = local;
        let second = io[1];
        io[1] = io[0];
        assert_eq!(
            engine.process_audio_frame_with_externals(&mut io),
            Err(ExternalFrameError::DuplicateHandle)
        );
        io[1] = second;
        for invalid in [
            PortFrame {
                channels: 17,
                ..PortFrame::default()
            },
            PortFrame::mono(f32::NAN),
            PortFrame {
                channels: 0,
                values: [1.0; 16],
            },
        ] {
            io[1].outputs[0] = invalid;
            assert_eq!(
                engine.process_audio_frame_with_externals(&mut io),
                Err(ExternalFrameError::InvalidOutputFrame)
            );
        }
        io[1].outputs[0] = PortFrame::default();
        io[0].outputs[0] = PortFrame::mono(8.0);
        engine.process_audio_frame_with_externals(&mut io).unwrap();
        assert_eq!(io[0].inputs[0], PortFrame::mono(0.0));
        engine.process_audio_frame_with_externals(&mut io).unwrap();
        assert_eq!(io[0].inputs[0], PortFrame::mono(8.0));
    }

    #[test]
    fn external_payloads_are_not_implicitly_adopted_into_a_new_runtime() {
        let patch = patch(json!([{ "id":"listen", "kind":"rackmcp_listen" }]));
        let specs = [spec("binary", &["in"], &["out"])];
        let routes = [
            cable("in", "binary", "out", "binary", "in"),
            cable("native", "binary", "out", "listen", "left"),
        ];
        let mut old = compile(&patch, &specs, &routes, Some("listen"), "left");
        let mut receiving = compile(&patch, &specs, &routes, Some("listen"), "left");
        let mut io = [ExternalFrameIo::new(
            old.resolve_external_module("binary").unwrap(),
        )];
        io[0].outputs[0] = PortFrame::mono(9.0);
        for _ in 0..3 {
            old.process_audio_frame_with_externals(&mut io).unwrap();
        }
        receiving.adopt_state_from(&old);
        let mut io = [ExternalFrameIo::new(
            receiving.resolve_external_module("binary").unwrap(),
        )];
        assert_eq!(
            receiving
                .process_audio_frame_with_externals(&mut io)
                .unwrap(),
            [0.0; 2]
        );
        assert_eq!(io[0].inputs[0], PortFrame::mono(0.0));
    }
    #[test]
    fn external_frame_preserves_taps_and_actual_listen_capture_point() {
        #[derive(Default)]
        struct Sink {
            count: usize,
            last: crate::ListenFrame,
        }
        impl ListenSink for Sink {
            fn armed(&self) -> bool {
                true
            }
            fn push(&mut self, frame: crate::ListenFrame) -> bool {
                self.count += 1;
                self.last = frame;
                true
            }
        }
        let patch = patch(json!([{ "id":"listen", "kind":"rackmcp_listen" }]));
        let mut engine = compile(
            &patch,
            &[spec("binary", &[], &["out"])],
            &[cable("capture", "binary", "out", "listen", "left")],
            Some("listen"),
            "left",
        );
        let listen = engine.resolve_module_state("listen").unwrap();
        let taps = [
            Some(engine.resolve_tap("binary", "out", 0).unwrap()),
            Some(engine.resolve_tap("listen", "left", 0).unwrap()),
        ];
        let mut samples = [TapSample::default(); 2];
        let mut io = [ExternalFrameIo::new(
            engine.resolve_external_module("binary").unwrap(),
        )];
        io[0].outputs[0] = PortFrame::mono(3.0);
        let mut sink = Sink::default();
        engine
            .process_audio_frame_with_externals_taps_and_listen(
                &mut io,
                &taps,
                &mut samples,
                Some((listen, &mut sink)),
            )
            .unwrap();
        assert_eq!(sink.count, 1);
        assert_eq!(sink.last.left, 0.0);
        assert_eq!(samples[0].value, 3.0);
        assert_eq!(samples[1].value, 0.0);
        engine
            .process_audio_frame_with_externals_taps_and_listen(
                &mut io,
                &taps,
                &mut samples,
                Some((listen, &mut sink)),
            )
            .unwrap();
        assert_eq!(sink.count, 2);
        assert_eq!(sink.last.left, 3.0);
        assert_eq!(samples[1].value, 3.0);
    }
    #[test]
    fn native_automation_refuses_external_targets_without_indexing_native_nodes() {
        let patch = patch(json!([{
            "id":"sequence", "kind":"rackmcp_sequencer",
            "rackmcp_sequence": {
                "data": {
                    "patterns":[{"name":"A", "length":16}],
                    "lanes":[{"moduleId":"42", "paramId":0, "mode":"normalized", "points":[{"bar":0,"value":0.5}]}]
                },
                "targets":[{"source_module_id":"42","source_param_id":0,"module":"binary","parameter":"gain"}]
            }
        }]));
        let banks = patch
            .prepare_wave_banks(&crate::WavePreparationCache::default())
            .unwrap();
        let error = Engine::compile_with_externals(
            &patch,
            &[spec("binary", &[], &["out"])],
            &[],
            None,
            "out",
            &banks,
        )
        .unwrap_err();
        assert!(
            error.contains("has no live native parameter capability"),
            "{error}"
        );
    }

    #[test]
    fn total_native_and_external_node_capacity_is_enforced() {
        let patch = patch(json!([{ "id":"dc", "kind":"poly_constant" }]));
        let banks = patch
            .prepare_wave_banks(&crate::WavePreparationCache::default())
            .unwrap();
        let specs: Vec<_> = (0..MAX_NODES)
            .map(|index| spec(&format!("e{index}"), &[], &[]))
            .collect();
        assert!(Engine::compile_with_externals(&patch, &specs, &[], None, "out", &banks).is_err());
    }
    #[test]
    fn ordinary_processing_consumes_previous_edges_without_replaying_external_outputs() {
        let patch = patch(json!([{ "id":"gain", "kind":"gain", "params":{"gain":0.5} }]));
        let mut engine = compile(
            &patch,
            &[spec("binary", &[], &["out"])],
            &[cable("out", "binary", "out", "gain", "signal")],
            Some("gain"),
            "out",
        );
        let mut io = [ExternalFrameIo::new(
            engine.resolve_external_module("binary").unwrap(),
        )];
        io[0].outputs[0] = PortFrame::mono(8.0);
        assert_eq!(
            engine.process_audio_frame_with_externals(&mut io).unwrap(),
            [0.0; 2]
        );
        let mut ordinary = [[0.0; 2]; 3];
        engine.process_audio_block(&mut ordinary);
        assert_eq!(ordinary, [[4.0; 2], [0.0; 2], [0.0; 2]]);

        let mut direct = compile(
            &Patch::new_v2(),
            &[spec("binary", &[], &["out"])],
            &[],
            Some("binary"),
            "out",
        );
        let mut io = [ExternalFrameIo::new(
            direct.resolve_external_module("binary").unwrap(),
        )];
        io[0].outputs[0] = PortFrame::mono(8.0);
        assert_eq!(
            direct.process_audio_frame_with_externals(&mut io).unwrap(),
            [8.0; 2]
        );
        let handle = direct.resolve_module_state("binary").unwrap();
        // Completed output remains available between frames for observation.
        assert_eq!(
            direct.host_module_snapshot(handle).unwrap().outputs[0].values[0],
            8.0
        );
        direct.process_audio_block(&mut ordinary);
        assert_eq!(ordinary, [[0.0; 2]; 3]);
        // A new explicit frame still works after ordinary processing.
        assert_eq!(
            direct.process_audio_frame_with_externals(&mut io).unwrap(),
            [8.0; 2]
        );
    }
}
