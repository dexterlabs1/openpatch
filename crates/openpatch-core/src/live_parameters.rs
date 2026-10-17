//! Effective parameter values live in the DSP model, not in a second mutable
//! parameter bank. Resolve handles on the control thread; reads, writes and
//! adoption below are bounded callback operations without allocation or locks.
use crate::{Engine, EngineKind, ModuleHandle, MAX_NODES};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ParameterId {
    StockVcaLevel,
    Rack264VcfFrequency,
}

impl ParameterId {
    pub(crate) fn for_kind(kind: &str) -> &'static [Self] {
        match kind {
            "stock_vca" => &[Self::StockVcaLevel],
            "rack264_vcf" => &[Self::Rack264VcfFrequency],
            _ => &[],
        }
    }
    pub(crate) fn mask(self) -> u8 {
        // Bits are local to a model's capability list, not global enum IDs.
        // Both currently expose one parameter at slot zero. Adding another
        // model does not consume a bit in every existing model's mask.
        match self {
            Self::StockVcaLevel | Self::Rack264VcfFrequency => 1,
        }
    }
    fn descriptor(self) -> ParameterDescriptor {
        match self {
            Self::StockVcaLevel => ParameterDescriptor {
                name: "level",
                min: 0.0,
                max: 1.0,
                snap: false,
            },
            Self::Rack264VcfFrequency => {
                let (min, max) = crate::rack264_vcf::frequency_bounds();
                ParameterDescriptor {
                    name: "frequency",
                    min,
                    max,
                    snap: false,
                }
            }
        }
    }
}

/// Parameter capabilities currently implemented by a native model. This list
/// is independent of lane ownership, so removing a writer cannot remove its
/// target's effective state. Empty means no live capability is claimed.
pub fn runtime_parameter_names(kind: &str) -> &'static [&'static str] {
    match kind {
        "stock_vca" => &["level"],
        "rack264_vcf" => &["frequency"],
        _ => &[],
    }
}

/// Control-thread metadata for validating a published native raw value.
pub fn runtime_parameter_descriptor(kind: &str, name: &str) -> Option<ParameterDescriptor> {
    let parameter = match (kind, name) {
        ("stock_vca", "level") => ParameterId::StockVcaLevel,
        ("rack264_vcf", "frequency") => ParameterId::Rack264VcfFrequency,
        _ => return None,
    };
    Some(parameter.descriptor())
}

/// A target bound to one compiled engine. Foreign/stale handles cannot read or
/// write a replacement incarnation. Re-resolve after graph compilation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ParameterHandle {
    generation: u64,
    node: usize,
    parameter: ParameterId,
    descriptor: ParameterDescriptor,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ParameterDescriptor {
    pub name: &'static str,
    pub min: f32,
    pub max: f32,
    pub snap: bool,
}

impl ParameterHandle {
    pub fn module(self) -> ModuleHandle {
        ModuleHandle {
            generation: self.generation,
            node: self.node,
        }
    }
}

impl Engine {
    /// Resolve a supported native parameter key. Source numeric IDs and source
    /// provenance belong to the import/control plan, outside this RT handle.
    pub fn resolve_parameter(&self, id: &str, name: &str) -> Result<ParameterHandle, String> {
        let (node, parameter) = match &self.inner {
            EngineKind::Poly(engine) => engine.resolve_parameter(id, name)?,
            _ => return Err("live parameters require patch version 2".into()),
        };
        Ok(ParameterHandle {
            generation: self.identity,
            node,
            parameter,
            descriptor: parameter.descriptor(),
        })
    }
    pub fn parameter_descriptor(&self, handle: ParameterHandle) -> Option<ParameterDescriptor> {
        self.parameter_value(handle)?;
        Some(handle.descriptor)
    }
    /// Read the actual value consumed by DSP. This is not a snapshot of the
    /// authored patch; the host must fence callback publication coherently.
    pub fn parameter_value(&self, handle: ParameterHandle) -> Option<f32> {
        if handle.generation != self.identity {
            return None;
        }
        match &self.inner {
            EngineKind::Poly(engine) => engine.parameter_value(handle.node, handle.parameter),
            _ => None,
        }
    }
    /// Set a bounded native raw value directly, without smoothing. A control
    /// plan evaluates source normalized/raw mapping, clamping and snap first.
    /// Invalid values/handles are refused atomically. No history is generated.
    pub fn set_parameter_value(&mut self, handle: ParameterHandle, value: f32) -> bool {
        if handle.generation != self.identity || !value.is_finite() {
            return false;
        }
        let descriptor = handle.descriptor;
        if value < descriptor.min || value > descriptor.max {
            return false;
        }
        match &mut self.inner {
            EngineKind::Poly(engine) => {
                engine.set_parameter_value(handle.node, handle.parameter, value)
            }
            _ => false,
        }
    }
    /// Inherit effective values across compiled graph replacement, independently
    /// of DSP-history/rate compatibility. Call after DSP adoption and before any
    /// ordered receiving assignments/actions. Explicit assignments retain their
    /// compiled receiving values, including equal authored values; fresh-module
    /// exclusions retain all receiving values. Resolve both lists against self.
    ///
    /// This is an explicit host integration step; ordinary historical adoption
    /// keeps its existing receiving-parameter policy. Never infer assignment
    /// intent from comparing patch floats or from whether a lane still exists.
    pub fn inherit_parameter_values(
        &mut self,
        previous: &Self,
        fresh: &[ModuleHandle],
        assigned: &[ParameterHandle],
    ) -> usize {
        let mut excluded = [false; MAX_NODES];
        let mut assignment_masks = [0_u8; MAX_NODES];
        for handle in fresh {
            if handle.generation == self.identity {
                if let Some(slot) = excluded.get_mut(handle.node) {
                    *slot = true;
                }
            }
        }
        for handle in assigned {
            if handle.generation == self.identity {
                if let Some(slot) = assignment_masks.get_mut(handle.node) {
                    *slot |= handle.parameter.mask();
                }
            }
        }
        match (&mut self.inner, &previous.inner) {
            (EngineKind::Poly(current), EngineKind::Poly(old)) => {
                current.inherit_parameter_values(old, &excluded, &assignment_masks)
            }
            _ => 0,
        }
    }
}
