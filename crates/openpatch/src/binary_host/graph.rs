//! Control-thread compilation of an explicit native/hosted document graph.
//! Preparing this plan does not grant execution permission or launch a child.
use crate::project_document::hosted::HostedModule;
use crate::project_document::{DocumentNode, ProjectDocument};
use openpatch_core::{
    Cable, Engine, ExternalModuleSpec, Patch, PreparedWaveBanks, ResolvedExternalModule,
};

pub struct HostedGraphPlan {
    native: Patch,
    hosted_id: String,
    module: HostedModule,
    routes: Vec<Cable>,
    output: Option<String>,
    output_port: String,
    input_mask: u32,
}

impl HostedGraphPlan {
    pub fn prepare(document: &ProjectDocument) -> Result<Self, String> {
        document.validate_structure()?;
        if document.graph.version != 2 || document.graph.sample_rate != super::SAMPLE_RATE {
            return Err("hosted execution currently requires graph v2 at 48000 Hz".into());
        }
        let mut hosted = None;
        for node in &document.graph.nodes {
            match node {
                DocumentNode::Hosted { id, module, .. } => {
                    module.validate_supported_offset()?;
                    if hosted.replace((id, module)).is_some() {
                        return Err("the first hosted capability supports one OFFSET module".into());
                    }
                }
                DocumentNode::Unresolved { .. } => {
                    return Err("unresolved modules prevent hosted graph execution".into())
                }
                DocumentNode::Native { .. } => {}
            }
        }
        let (id, module) = hosted.ok_or("hosted graph requires an explicit hosted module")?;
        let carrier = document.state_carrier()?;
        if !carrier.omitted_automation.is_empty() {
            return Err("unsupported automation prevents hosted graph execution".into());
        }
        let mut input_mask = 0;
        let mut input_cables = [0usize; 3];
        for cable in &document.graph.cables {
            if cable.route.to == *id {
                let port = module
                    .ports
                    .iter()
                    .find(|port| {
                        port.direction == crate::project_document::PortDirection::Input
                            && port.key == cable.route.input
                    })
                    .ok_or("undeclared hosted input")?;
                input_cables[port.index as usize] += 1;
                if input_cables[port.index as usize] > 1 {
                    return Err("current host transport requires one cable per hosted input; stacked routes remain preserved but cannot execute yet".into());
                }
                input_mask |= 1 << port.index;
            }
        }
        if input_mask & 4 == 0 {
            return Err("the current companion requires a connected OFFSET signal input".into());
        }
        Ok(Self {
            native: carrier.patch,
            hosted_id: id.clone(),
            module: module.clone(),
            routes: document
                .graph
                .cables
                .iter()
                .map(|cable| cable.route.clone())
                .collect(),
            output: document.graph.output.clone(),
            output_port: document.graph.output_port.clone(),
            input_mask,
        })
    }

    pub fn native_patch(&self) -> &Patch {
        &self.native
    }
    pub fn hosted_id(&self) -> &str {
        &self.hosted_id
    }
    pub fn module(&self) -> &HostedModule {
        &self.module
    }
    pub fn input_mask(&self) -> u32 {
        self.input_mask
    }

    pub fn compile(
        &self,
        banks: &PreparedWaveBanks,
    ) -> Result<(Engine, ResolvedExternalModule), String> {
        let spec = ExternalModuleSpec {
            id: self.hosted_id.clone(),
            inputs: vec!["offset_cv".into(), "scale_cv".into(), "in".into()],
            outputs: vec!["out".into()],
        };
        let engine = Engine::compile_with_externals(
            &self.native,
            &[spec],
            &self.routes,
            self.output.as_deref(),
            &self.output_port,
            banks,
        )?;
        let handle = engine.resolve_external_module(&self.hosted_id)?;
        Ok((engine, handle))
    }
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use crate::project_document::{hosted::HostedIdentity, DocumentCable, ProjectLayout};
    use serde_json::json;

    pub(crate) fn document() -> ProjectDocument {
        let native: Patch = serde_json::from_value(json!({"version":2,"sample_rate":48000,
            "nodes":[{"id":"dc","kind":"constant","params":{"value":0.5}},
                     {"id":"amp","kind":"gain","params":{"gain":0.5}}],"cables":[],"output":"amp"}))
        .unwrap();
        let mut document = ProjectDocument::from_patch(native, ProjectLayout::default()).unwrap();
        document.graph.nodes.push(DocumentNode::Hosted {
            id: "offset".into(),
            source: None,
            module: HostedModule::offset(
                HostedIdentity {
                    plugin: "Bogaudio".into(),
                    model: "Bogaudio-Offset".into(),
                    version: "2.6.47".into(),
                    rack_version: "2.6.6".into(),
                    plugin_sha256:
                        "264e55e1389274c3873439372c5c51f48f63168bd6c98fb2c6ffec170224b6f6".into(),
                    runtime_sha256:
                        "55dd40f7dff09a6ed7b93374b150f33931034d3754c128da7b209ea37b6aaf1c".into(),
                },
                0.,
                0.5,
                true,
                false,
            )
            .unwrap(),
        });
        for route in [
            json!({"id":"host-in","from":"dc","to":"offset","output":"out","input":"in"}),
            json!({"id":"host-out","from":"offset","to":"amp","output":"out","input":"signal"}),
        ] {
            document.graph.cables.push(DocumentCable {
                route: serde_json::from_value(route).unwrap(),
                source: None,
            });
        }
        document
    }

    pub(crate) fn compiled() -> (Engine, ResolvedExternalModule) {
        let plan = HostedGraphPlan::prepare(&document()).unwrap();
        let banks = plan
            .native_patch()
            .prepare_wave_banks(&openpatch_core::WavePreparationCache::default())
            .unwrap();
        plan.compile(&banks).unwrap()
    }

    #[test]
    fn authored_mixed_graph_compiles_the_actual_external_routes_and_native_models() {
        let document = document();
        let plan = HostedGraphPlan::prepare(&document).unwrap();
        assert_eq!(plan.input_mask(), 4);
        assert_eq!(plan.native_patch().nodes.len(), 2);
        assert!(plan
            .native_patch()
            .nodes
            .iter()
            .all(|node| node.id != "offset"));
        let (mut engine, handle) = compiled();
        assert_eq!(handle.input_presence(), 4);
        let mut io = openpatch_core::ExternalFrameIo::new(handle);
        io.outputs[0] = openpatch_core::PortFrame::mono(2.);
        assert_eq!(
            engine
                .process_audio_frame_with_externals(std::slice::from_mut(&mut io))
                .unwrap(),
            [0.; 2]
        );
        assert_eq!(io.inputs[2], openpatch_core::PortFrame::mono(0.));
        assert_eq!(
            engine
                .process_audio_frame_with_externals(std::slice::from_mut(&mut io))
                .unwrap(),
            [1.; 2]
        );
        assert_eq!(io.inputs[2], openpatch_core::PortFrame::mono(0.5));
        assert!(
            document
                .project(crate::project_document::PlaybackMode::Exact)
                .is_err(),
            "preparation is not execution permission"
        );
    }

    #[test]
    fn unsupported_profiles_models_and_stacked_host_inputs_refuse_without_losing_the_document() {
        let original = document();
        for (version, rate) in [(1, 48000), (2, 44100)] {
            let mut unsupported = original.clone();
            unsupported.graph.version = version;
            unsupported.graph.sample_rate = rate;
            assert!(HostedGraphPlan::prepare(&unsupported).is_err());
        }
        let mut stacked = original.clone();
        let mut cable = stacked.graph.cables[0].clone();
        cable.route.id = Some("stacked".into());
        stacked.graph.cables.push(cable);
        stacked.validate_structure().unwrap();
        assert!(HostedGraphPlan::prepare(&stacked)
            .err()
            .unwrap()
            .contains("one cable"));
        let recovered = ProjectDocument::decode(&stacked.encode().unwrap()).unwrap();
        assert_eq!(recovered.graph.cables.len(), 3);
        let mut missing = original.clone();
        missing.graph.cables.remove(0);
        assert!(HostedGraphPlan::prepare(&missing)
            .err()
            .unwrap()
            .contains("connected"));
    }
}
