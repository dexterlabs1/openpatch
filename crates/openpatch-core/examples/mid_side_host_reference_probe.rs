//! Genuine production Engine counterpart to the frozen MidSide SDK host cases.
//! No MidSide equations, expected storage, or anchor-derived values appear here.
use openpatch_core::{
    Engine, HostModuleSnapshot, HostPortLifecycle, HostPortSnapshot, InputLifecycleTransform, Node,
    OutputLifecycleTransform, Patch, PortDirection, PortFrame,
};
use serde_json::{json, Value};
use std::io::{self, BufRead, Write};

const INPUTS: [&str; 6] = ["encode_cv", "left", "right", "decode_cv", "mid", "side"];
const OUTPUTS: [&str; 4] = ["mid", "side", "left", "right"];

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
fn scalar(value: &Value) -> Result<f32, String> {
    value
        .as_f64()
        .map(|v| v as f32)
        .filter(|v| v.is_finite())
        .ok_or_else(|| "expected finite f32".into())
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
        "module" => Ok(("module".into(), OUTPUTS[index(event, "port", 4)?])),
        other => Err(format!("unsupported source role {other}")),
    }
}
fn destination_endpoint(event: &Value) -> Result<(String, &'static str), String> {
    match text(event, "destination")? {
        "module" => Ok(("module".into(), INPUTS[index(event, "input", 6)?])),
        "sink" => Ok((sink_id(index(event, "input", 8)?), "signal")),
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
    fn new(rate: u32) -> Result<Self, String> {
        let mut patch = Patch::new_v2();
        patch.sample_rate = rate;
        for port in 0..6 {
            let mut params = serde_json::Map::new();
            params.insert("channels".into(), json!(0));
            for lane in 1..=16 {
                params.insert(format!("v{lane}"), json!(0));
            }
            patch.nodes.push(node(
                source_id(port),
                "poly_constant",
                Value::Object(params),
            )?);
        }
        patch.nodes.push(node(
            "module".into(),
            "stock_mid_side",
            json!({"encode_width":1,"decode_width":1}),
        )?);
        for port in 0..8 {
            patch.nodes.push(node(sink_id(port), "mult", json!({}))?);
        }
        // Select only an otherwise unobserved Sink output. This adds no source
        // or MidSide consumer absent from the SDK fixture.
        patch.output = Some(sink_id(0));
        patch.output_port = "out1".into();
        let engine = patch.compile()?;
        Ok(Self {
            patch,
            engine,
            frame: 0,
        })
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
        let mut patch = self.patch.clone();
        match op {
            "source" => {
                let id = source_id(index(event, "port", 6)?);
                let values = event["values"]
                    .as_array()
                    .filter(|v| v.len() <= 16)
                    .ok_or("invalid source values")?;
                let source = patch
                    .nodes
                    .iter_mut()
                    .find(|n| n.id == id)
                    .ok_or("source missing")?;
                source.params.insert("channels".into(), values.len() as f32);
                for lane in 0..16 {
                    source.params.insert(
                        format!("v{}", lane + 1),
                        match values.get(lane) {
                            Some(v) => scalar(v)?,
                            None => 0.0,
                        },
                    );
                }
            }
            "param" => {
                let parameter = ["encode_width", "decode_width"][index(event, "index", 2)?];
                patch
                    .nodes
                    .iter_mut()
                    .find(|n| n.id == "module")
                    .ok_or("module missing")?
                    .params
                    .insert(parameter.into(), scalar(&event["value"])?);
            }
            "reset" => {
                let module = patch
                    .nodes
                    .iter_mut()
                    .find(|n| n.id == "module")
                    .ok_or("module missing")?;
                module.params.insert("encode_width".into(), 1.0);
                module.params.insert("decode_width".into(), 1.0);
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
        next.adopt_state_from(&self.engine);
        for (handle, delta) in prepared {
            if !next.apply_host_port_lifecycle(handle, &delta) {
                return Err("explicit host lifecycle rejected".into());
            }
        }
        self.engine = next;
        self.patch = patch;
        Ok(())
    }
    fn snapshot(&self, id: &str, event: usize, op: &str) -> Result<Value, String> {
        let module = self.snapshot_node("module")?;
        if (module.input_count, module.output_count) != (6, 4) {
            return Err("wrong MidSide port counts".into());
        }
        let port = |p: &HostPortSnapshot| json!({"channels":p.channels,"values":p.values});
        let sources = (0..6)
            .map(|i| {
                self.snapshot_node(&source_id(i))
                    .map(|s| port(&s.outputs[0]))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let sinks = (0..8)
            .map(|i| self.snapshot_node(&sink_id(i)).map(|s| port(&s.inputs[0])))
            .collect::<Result<Vec<_>, _>>()?;
        let mut source_config = Vec::with_capacity(6);
        for i in 0..6 {
            let source = self.configured_node(&source_id(i))?;
            let width = source.params["channels"] as usize;
            source_config.push(
                (0..width)
                    .map(|lane| source.params[&format!("v{}", lane + 1)])
                    .collect::<Vec<_>>(),
            );
        }
        let configured = self.configured_node("module")?;
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
        Ok(
            json!({"case":id,"event":event,"op":op,"frame":self.frame,"bypassed":module.bypassed,
            "inputs":module.inputs[..6].iter().map(port).collect::<Vec<_>>(),
            "outputs":module.outputs[..4].iter().map(port).collect::<Vec<_>>(),
            "source_outputs":sources,"sink_inputs":sinks,"source_config":source_config,
            "params":[configured.params["encode_width"],configured.params["decode_width"]],
            "input_cables":input_cables,"output_cables":output_cables}),
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
        let mut host = Host::new(rate)?;
        for (i, event) in case["events"]
            .as_array()
            .ok_or("missing event array")?
            .iter()
            .enumerate()
        {
            host.event(event)?;
            serde_json::to_writer(&mut out, &host.snapshot(id, i, text(event, "op")?)?)
                .map_err(|e| e.to_string())?;
            writeln!(out).map_err(|e| e.to_string())?;
        }
    }
    out.flush().map_err(|e| e.to_string())
}
fn main() {
    if let Err(error) = run() {
        eprintln!("MidSide production host probe failed: {error}");
        std::process::exit(1);
    }
}
