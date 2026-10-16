// Control-side composition and callback-side application of ordered host edges.
use openpatch::control::HostLifecycleDelta;
use openpatch_core::{HostPortLifecycle, ModuleHandle, PortDirection};

#[derive(Default)]
struct PreparedHostLifecycle {
    named: HostLifecycleDelta,
    effects: Vec<(ModuleHandle, HostPortLifecycle)>,
    resets: Vec<ModuleHandle>,
    applied: bool,
}

impl PreparedHostLifecycle {
    fn prepare(engine: &Engine, named: HostLifecycleDelta) -> Result<Self, String> {
        let mut result = Self {
            named,
            ..Self::default()
        };
        for (id, module) in &result.named.modules {
            // Deleted modules have no receiving incarnation. A reused ID is
            // fenced by reset, which also discarded its earlier port effects.
            let Ok(handle) = engine.resolve_module_state(id) else {
                continue;
            };
            if module.reset {
                result.resets.push(handle);
                continue;
            }
            let Some((_, _, input_presence, output_consumers, bypassed)) =
                engine.host_port_layout(handle)
            else {
                continue;
            };
            let mut effect = HostPortLifecycle {
                inputs: [openpatch_core::InputLifecycleTransform::identity(); 16],
                outputs: Default::default(),
                input_presence,
                output_consumers,
                bypassed,
            };
            for (name, transform) in &module.inputs {
                let port = engine
                    .resolve_host_port(handle, PortDirection::Input, name)
                    .ok_or_else(|| format!("unknown lifecycle input '{id}.{name}'"))?;
                effect.inputs[port] = *transform;
            }
            for (name, transform) in &module.outputs {
                let port = engine
                    .resolve_host_port(handle, PortDirection::Output, name)
                    .ok_or_else(|| format!("unknown lifecycle output '{id}.{name}'"))?;
                effect.outputs[port] = *transform;
            }
            result.effects.push((handle, effect));
        }
        Ok(result)
    }

    /// Exactly one active recipient consumes these edges. Fading donors never
    /// apply them, and repeated callbacks cannot repeat irreversible clearing.
    fn apply(&mut self, engine: &mut Engine) {
        if self.applied {
            return;
        }
        for (handle, effect) in &self.effects {
            let applied = engine.apply_host_port_lifecycle(*handle, effect);
            debug_assert!(applied);
        }
        self.applied = true;
    }
}
