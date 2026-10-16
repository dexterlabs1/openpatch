//! Production Engine counterpart to the independently declared Stock Mix host cases.
//! No Mixer arithmetic or expected reference values appear in this adapter.
use openpatch_core::{
    Engine, HostModuleSnapshot, HostPortLifecycle, HostPortSnapshot, InputLifecycleTransform,
    ModuleParameterActions, Node, OutputLifecycleTransform, Patch, PortDirection, PortFrame,
};
use serde_json::{json, Value};
use std::io::{self, BufRead, Write};

const INPUTS: [&str; 6] = ["in1", "in2", "in3", "in4", "in5", "in6"];
const OUTPUTS: [&str; 1] = ["out"];

fn text<'a>(value: &'a Value, key: &str) -> Result<&'a str, String> {
    value[key]
        .as_str()
        .ok_or_else(|| format!("missing string {key}"))
}
fn index(value: &Value, key: &str, limit: usize) -> Result<usize, String> {
    value[key]
        .as_u64()
        .and_then(|n| usize::try_from(n).ok())
        .filter(|n| *n < limit)
        .ok_or_else(|| format!("invalid {key}"))
}
fn scalar_bits(value: &Value) -> Result<f32, String> {
    let bits = value
        .as_u64()
        .and_then(|n| u32::try_from(n).ok())
        .ok_or("invalid f32 bits")?;
    let value = f32::from_bits(bits);
    value
        .is_finite()
        .then_some(value)
        .ok_or_else(|| "nonfinite fixture value".into())
}
fn configure_source(node: &mut Node, values: &Value) -> Result<(), String> {
    let values = values
        .as_array()
        .filter(|v| v.len() <= 16)
        .ok_or("invalid source array")?;
    node.params.insert("channels".into(), values.len() as f32);
    for lane in 0..16 {
        let value = values
            .get(lane)
            .map(scalar_bits)
            .transpose()?
            .unwrap_or(0.0);
        node.params.insert(format!("v{}", lane + 1), value);
    }
    Ok(())
}
fn node(id: String, kind: &str, params: Value) -> Result<Node, String> {
    serde_json::from_value(json!({"id":id,"kind":kind,"params":params})).map_err(|e| e.to_string())
}
fn source_id(port: usize) -> String {
    format!("source{port}")
}
fn sink_id(port: usize) -> String {
    format!("sink{port}")
}
fn source_endpoint(event: &Value) -> Result<(String, &'static str), String> {
    match text(event, "source")? {
        "source" => Ok((source_id(index(event, "port", 6)?), "out")),
        "module" => Ok(("module".into(), OUTPUTS[index(event, "port", 1)?])),
        other => Err(format!("unsupported source role {other}")),
    }
}
fn destination_endpoint(event: &Value) -> Result<(String, &'static str), String> {
    match text(event, "destination")? {
        "module" => Ok(("module".into(), INPUTS[index(event, "input", 6)?])),
        "sink" => Ok((sink_id(index(event, "input", 2)?), "signal")),
        other => Err(format!("unsupported destination role {other}")),
    }
}
struct Host {
    patch: Patch,
    engine: Engine,
    // Exactly one increment per completed production process_poly_block([1]).
    // Recompilation/adoption never processes or advances this host sample clock.
    frame: u64,
}
impl Host {
    fn new(rate: u32, initial: &Value) -> Result<Self, String> {
        let mut patch = Patch::new_v2();
        patch.sample_rate = rate;
        let sources = initial["sources"]
            .as_array()
            .filter(|v| v.len() == 6)
            .ok_or("six source controls required")?;
        for (port, values) in sources.iter().enumerate() {
            let mut source = node(source_id(port), "poly_constant", json!({}))?;
            configure_source(&mut source, values)?;
            patch.nodes.push(source);
        }
        patch.nodes.push(node(
            "module".into(),
            "stock_mix",
            json!({
                "level": scalar_bits(&initial["level_bits"])?, "invert": 0, "average": 0
            }),
        )?);
        for port in 0..2 {
            patch.nodes.push(node(sink_id(port), "mult", json!({}))?);
        }
        patch.output = Some(sink_id(0));
        patch.output_port = "out1".into();
        let input_mask = index(initial, "input_mask", 64)?;
        let output_mask = index(initial, "output_mask", 4)?;
        for (port, input) in INPUTS.iter().enumerate() {
            if input_mask & (1 << port) != 0 {
                patch.cables.push(
                    serde_json::from_value(json!({"id":format!("in{port}"),
                    "from":source_id(port),"output":"out","to":"module","input":input}))
                    .map_err(|e| e.to_string())?,
                );
            }
        }
        for port in 0..2 {
            if output_mask & (1 << port) != 0 {
                patch.cables.push(
                    serde_json::from_value(json!({"id":format!("out{port}"),
                    "from":"module","output":"out","to":sink_id(port),"input":"signal"}))
                    .map_err(|e| e.to_string())?,
                );
            }
        }
        let engine = patch.compile()?;
        let mut result = Self {
            patch,
            engine,
            frame: 0,
        };
        result.load_data(initial.get("data").ok_or("missing initial data")?)?;
        Ok(result)
    }
    fn load_data(&mut self, value: &Value) -> Result<(), String> {
        let handle = self.engine.resolve_module_state("module")?;
        if !self.engine.load_stock_mix_legacy_data(handle, value) {
            return Err("production legacy load rejected".into());
        }
        let state = self
            .engine
            .stock_mix_snapshot(handle)
            .ok_or("actual Mixer state missing")?;
        let node = self
            .patch
            .nodes
            .iter_mut()
            .find(|n| n.id == "module")
            .ok_or("module absent")?;
        // Persist actual loaded controls for later graph recompilation. This is
        // serialization of production state, not a second flag loader.
        node.params
            .insert("invert".into(), u8::from(state.invert) as f32);
        node.params
            .insert("average".into(), u8::from(state.average) as f32);
        Ok(())
    }
    fn snapshot_node(&self, id: &str) -> Result<HostModuleSnapshot, String> {
        let handle = self.engine.resolve_module_state(id)?;
        self.engine
            .host_module_snapshot(handle)
            .ok_or_else(|| format!("host snapshot unavailable: {id}"))
    }
    fn configured_node(&self, id: &str) -> Result<&Node, String> {
        self.patch
            .nodes
            .iter()
            .find(|n| n.id == id)
            .ok_or_else(|| format!("unknown module {id}"))
    }
    fn event(&mut self, event: &Value) -> Result<(), String> {
        let op = text(event, "op")?;
        if op == "step" {
            self.engine
                .process_poly_block(&mut [PortFrame::default(); 1]);
            self.frame += 1;
            return Ok(());
        }
        if op == "data_load" {
            return self.load_data(event.get("value").ok_or("missing data value")?);
        }
        if op == "save" {
            self.engine
                .stock_mix_legacy_data(self.engine.resolve_module_state("module")?)
                .ok_or("actual serializer missing")?;
            return Ok(());
        }
        let mut patch = self.patch.clone();
        let fresh = op == "recreate";
        match op {
            "source" => {
                let id = source_id(index(event, "port", 6)?);
                configure_source(
                    patch
                        .nodes
                        .iter_mut()
                        .find(|n| n.id == id)
                        .ok_or("source missing")?,
                    &event["values_bits"],
                )?;
            }
            "level" => {
                patch
                    .nodes
                    .iter_mut()
                    .find(|n| n.id == "module")
                    .ok_or("module missing")?
                    .params
                    .insert("level".into(), scalar_bits(&event["value_bits"])?);
            }
            "reset" => {
                let module = patch
                    .nodes
                    .iter_mut()
                    .find(|n| n.id == "module")
                    .ok_or("module missing")?;
                ModuleParameterActions::new(patch.sample_rate).initialize(module)?;
            }
            "adopt" => {
                let module = patch
                    .nodes
                    .iter_mut()
                    .find(|n| n.id == "module")
                    .ok_or("module missing")?;
                module
                    .params
                    .insert("level".into(), scalar_bits(&event["level_bits"])?);
                for flag in ["invert", "average"] {
                    let value = event[flag].as_bool().ok_or("invalid receiving flag")?;
                    module.params.insert(flag.into(), u8::from(value) as f32);
                }
            }
            "recreate" => {
                patch
                    .cables
                    .retain(|c| c.from != "module" && c.to != "module");
                let module = patch
                    .nodes
                    .iter_mut()
                    .find(|n| n.id == "module")
                    .ok_or("module missing")?;
                *module = node(
                    "module".into(),
                    "stock_mix",
                    json!({"level":1,"invert":0,"average":0}),
                )?;
            }
            "bypass" => {
                patch
                    .nodes
                    .iter_mut()
                    .find(|n| n.id == "module")
                    .ok_or("module missing")?
                    .bypassed = event["value"].as_bool().ok_or("invalid bypass value")?;
            }
            "connect" => {
                let id = text(event, "id")?;
                if patch.cables.iter().any(|c| c.id.as_deref() == Some(id)) {
                    return Err("duplicate cable name".into());
                }
                let (from, output) = source_endpoint(event)?;
                let (to, input) = destination_endpoint(event)?;
                // Resolve the actual production endpoint names rather than
                // assuming numeric Core port ordering matches the SDK order.
                let source = self.engine.resolve_module_state(&from)?;
                let destination = self.engine.resolve_module_state(&to)?;
                self.engine
                    .resolve_host_port(source, PortDirection::Output, output)
                    .ok_or("unresolved output")?;
                self.engine
                    .resolve_host_port(destination, PortDirection::Input, input)
                    .ok_or("unresolved input")?;
                patch.cables.push(
                    serde_json::from_value(
                        json!({"id":id,"from":from,"output":output,"to":to,"input":input}),
                    )
                    .map_err(|e| e.to_string())?,
                );
            }
            "disconnect" => {
                let id = text(event, "id")?;
                let at = patch
                    .cables
                    .iter()
                    .position(|c| c.id.as_deref() == Some(id))
                    .ok_or("missing cable name")?;
                patch.cables.remove(at);
            }
            _ => return Err(format!("unknown event {op}")),
        }
        let mut next = patch.compile()?;
        // Resolve final context BEFORE adoption, which preserves the donor's
        // raw port storage and previous connection/bypass context.
        let mut prepared = Vec::with_capacity(patch.nodes.len());
        for node in &patch.nodes {
            if fresh && node.id == "module" {
                continue;
            }
            let old_handle = self.engine.resolve_module_state(&node.id)?;
            let next_handle = next.resolve_module_state(&node.id)?;
            let (old_inputs, old_outputs, old_presence, old_consumers, old_bypass) = self
                .engine
                .host_port_layout(old_handle)
                .ok_or("old host layout unavailable")?;
            let (inputs, outputs, presence, consumers, bypassed) = next
                .host_port_layout(next_handle)
                .ok_or("new host layout unavailable")?;
            if (old_inputs, old_outputs) != (inputs, outputs) {
                return Err("unexpected port-layout change".into());
            }
            let mut delta = HostPortLifecycle {
                input_presence: presence,
                output_consumers: consumers,
                bypassed,
                ..Default::default()
            };
            for port in 0..usize::from(inputs) {
                let before = old_presence & (1 << port) != 0;
                let after = presence & (1 << port) != 0;
                delta.inputs[port] = match (before, after) {
                    (false, true) => InputLifecycleTransform::connect(),
                    (true, false) => InputLifecycleTransform::disconnect(),
                    _ => InputLifecycleTransform::identity(),
                };
            }
            for port in 0..usize::from(outputs) {
                let before = old_consumers & (1 << port) != 0;
                let after = consumers & (1 << port) != 0;
                delta.outputs[port] = match (before, after) {
                    (false, true) => OutputLifecycleTransform::connect(),
                    (true, false) => OutputLifecycleTransform::disconnect(),
                    _ => OutputLifecycleTransform::identity(),
                };
                if old_bypass != bypassed {
                    delta.outputs[port] =
                        delta.outputs[port].then(OutputLifecycleTransform::bypass_edge());
                }
            }
            prepared.push((next_handle, delta));
        }
        if fresh {
            let handle = next.resolve_module_state("module")?;
            next.adopt_state_from_excluding(&self.engine, &[handle]);
        } else {
            next.adopt_state_from(&self.engine);
        }
        for (handle, delta) in prepared {
            if !next.apply_host_port_lifecycle(handle, &delta) {
                return Err("explicit host lifecycle rejected".into());
            }
        }
        if op == "reset" {
            let handle = next.resolve_module_state("module")?;
            if !next.initialize_module_state(handle) {
                return Err("production Initialize rejected".into());
            }
        }
        self.engine = next;
        self.patch = patch;
        Ok(())
    }
    fn snapshot(&self, id: &str, event: i64, op: &str) -> Result<Value, String> {
        let module = self.snapshot_node("module")?;
        if (module.input_count, module.output_count) != (6, 1) {
            return Err("wrong Mixer port counts".into());
        }
        let handle = self.engine.resolve_module_state("module")?;
        let controls = self
            .engine
            .stock_mix_snapshot(handle)
            .ok_or("actual Mixer controls missing")?;
        let saved_data = self
            .engine
            .stock_mix_legacy_data(handle)
            .ok_or("actual serializer missing")?;
        let port = |p: &HostPortSnapshot| json!({"channels":p.channels,"values_bits":p.values.map(f32::to_bits)});
        let sources = (0..6)
            .map(|i| {
                self.snapshot_node(&source_id(i))
                    .map(|s| port(&s.outputs[0]))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let sinks = (0..2)
            .map(|i| self.snapshot_node(&sink_id(i)).map(|s| port(&s.inputs[0])))
            .collect::<Result<Vec<_>, _>>()?;
        let mut source_config = Vec::with_capacity(6);
        for i in 0..6 {
            let source = self.configured_node(&source_id(i))?;
            let width = source.params["channels"] as usize;
            source_config.push(
                (0..width)
                    .map(|lane| source.params[&format!("v{}", lane + 1)].to_bits())
                    .collect::<Vec<_>>(),
            );
        }
        let input_cables = INPUTS.map(|name| {
            self.patch
                .cables
                .iter()
                .filter(|c| c.to == "module" && c.input == name)
                .count()
        });
        let output_cables = OUTPUTS.map(|name| {
            self.patch
                .cables
                .iter()
                .filter(|c| c.from == "module" && c.output == name)
                .count()
        });
        let source_cables: Vec<_> = (0..6)
            .map(|i| {
                self.patch
                    .cables
                    .iter()
                    .filter(|c| c.from == source_id(i))
                    .count()
            })
            .collect();
        let sink_cables: Vec<_> = (0..2)
            .map(|i| {
                self.patch
                    .cables
                    .iter()
                    .filter(|c| c.to == sink_id(i))
                    .count()
            })
            .collect();
        Ok(
            json!({"case":id,"event":event,"op":op,"frame":self.frame,"bypassed":module.bypassed,
            "level_bits":controls.level.to_bits(),"average":controls.average,"invert":controls.invert,"saved_data":saved_data,
            "inputs":module.inputs[..6].iter().map(port).collect::<Vec<_>>(),
            "outputs":module.outputs[..1].iter().map(port).collect::<Vec<_>>(),
            "source_outputs":sources,"sink_inputs":sinks,"source_config":source_config,
            "input_cables":input_cables,"output_cables":output_cables,"source_cables":source_cables,"sink_cables":sink_cables}),
        )
    }
}
fn run() -> Result<(), String> {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut out = io::BufWriter::new(stdout.lock());
    for line in stdin.lock().lines() {
        let line = line.map_err(|e| e.to_string())?;
        if line.trim().is_empty() {
            return Err("blank case line".into());
        }
        let case: Value = serde_json::from_str(&line).map_err(|e| e.to_string())?;
        let rate = case["sample_rate"]
            .as_u64()
            .filter(|v| (8000..=192000).contains(v))
            .ok_or("invalid sample rate")? as u32;
        let id = text(&case, "id")?;
        let mut host = Host::new(rate, &case["initial"])?;
        serde_json::to_writer(&mut out, &host.snapshot(id, -1, "initial")?)
            .map_err(|e| e.to_string())?;
        writeln!(out).map_err(|e| e.to_string())?;
        for (i, event) in case["events"]
            .as_array()
            .ok_or("missing event array")?
            .iter()
            .enumerate()
        {
            host.event(event)?;
            serde_json::to_writer(&mut out, &host.snapshot(id, i as i64, text(event, "op")?)?)
                .map_err(|e| e.to_string())?;
            writeln!(out).map_err(|e| e.to_string())?;
        }
    }
    out.flush().map_err(|e| e.to_string())
}
fn main() {
    if let Err(error) = run() {
        eprintln!("Stock Mix production host probe failed: {error}");
        std::process::exit(1);
    }
}
