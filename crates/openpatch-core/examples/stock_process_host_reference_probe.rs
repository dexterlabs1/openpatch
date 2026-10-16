//! Production Engine adapter for independently declared Stock Process host traces.
//! It reports actual Core and host state; no expected Process arithmetic.
use openpatch_core::{
    Engine, HostModuleSnapshot, HostPortLifecycle, HostPortSnapshot, InputLifecycleTransform,
    ModuleParameterActions, Node, OutputLifecycleTransform, Patch, PortDirection, PortFrame,
};
use serde_json::{json, Value};
use std::io::{self, BufRead, Write};

const INPUTS: [&str; 3] = ["slew_cv", "signal", "gate"];
const OUTPUTS: [&str; 6] = ["sh1", "sh2", "track_hold", "hold_track", "slew", "glide"];

fn parameter_bits(value: &Value) -> Result<[f32; 2], String> {
    let values = value
        .as_array()
        .filter(|a| a.len() == 2)
        .ok_or("two parameter bits required")?;
    let mut out = [0.0; 2];
    for i in 0..2 {
        out[i] = scalar_bits(&values[i])?;
    }
    Ok(out)
}
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
    let sample = f32::from_bits(bits);
    sample
        .is_finite()
        .then_some(sample)
        .ok_or_else(|| "nonfinite fixture value".into())
}
fn source_frame(value: &Value) -> Result<PortFrame, String> {
    let values = value
        .as_array()
        .filter(|a| a.len() <= 16)
        .ok_or("invalid source array")?;
    let mut frame = PortFrame {
        channels: values.len() as u8,
        ..PortFrame::default()
    };
    for (lane, value) in values.iter().enumerate() {
        frame.values[lane] = scalar_bits(value)?;
    }
    Ok(frame)
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
        "source" => Ok((source_id(index(event, "port", 3)?), "out")),
        "module" => Ok(("module".into(), OUTPUTS[index(event, "port", 6)?])),
        role => Err(format!("unsupported source role {role}")),
    }
}
fn destination_endpoint(event: &Value) -> Result<(String, &'static str), String> {
    match text(event, "destination")? {
        "module" => Ok(("module".into(), INPUTS[index(event, "input", 3)?])),
        "sink" => Ok((sink_id(index(event, "input", 6)?), "signal")),
        role => Err(format!("unsupported destination role {role}")),
    }
}

struct Host {
    patch: Patch,
    engine: Engine,
    sources: [PortFrame; 3],
}
impl Host {
    fn new(rate: u32, initial: &Value) -> Result<Self, String> {
        let values = initial["sources"]
            .as_array()
            .filter(|a| a.len() == 3)
            .ok_or("three source controls required")?;
        let mut sources = [PortFrame::default(); 3];
        for i in 0..3 {
            sources[i] = source_frame(&values[i])?;
        }
        let mut patch = Patch::new_v2();
        patch.sample_rate = rate;
        for (port, frame) in sources.iter().enumerate() {
            let mut source = node(
                source_id(port),
                "poly_constant",
                json!({"channels":frame.channels}),
            )?;
            // Placeholder params remain in the public editing range. The desired
            // source frame is injected into the real compiled producer below.
            for lane in 1..=16 {
                source.params.insert(format!("v{lane}"), 0.0);
            }
            patch.nodes.push(source);
        }
        let initial_parameters = parameter_bits(&initial["parameter_bits"])?;
        patch.nodes.push(node(
            "module".into(),
            "stock_process",
            json!({"slew":initial_parameters[0]}),
        )?);
        for port in 0..6 {
            patch.nodes.push(node(sink_id(port), "mult", json!({}))?);
        }
        patch.output = Some(sink_id(0));
        patch.output_port = "out1".into();
        let input_mask = index(initial, "input_mask", 8)?;
        let output_mask = index(initial, "output_mask", 64)?;
        for (port, input) in INPUTS.iter().enumerate() {
            if input_mask & (1 << port) != 0 {
                patch.cables.push(
                    serde_json::from_value(json!({"id":format!("in{port}"),
                    "from":source_id(port),"output":"out","to":"module","input":input}))
                    .map_err(|e| e.to_string())?,
                );
            }
        }
        for (port, output) in OUTPUTS.iter().enumerate() {
            if output_mask & (1 << port) != 0 {
                patch.cables.push(
                    serde_json::from_value(json!({"id":format!("out{port}"),
                    "from":"module","output":output,"to":sink_id(port),"input":"signal"}))
                    .map_err(|e| e.to_string())?,
                );
            }
        }
        let engine = patch.compile()?;
        let mut host = Self {
            patch,
            engine,
            sources,
        };
        host.apply_sources()?;
        host.load_data(initial.get("data").ok_or("missing initial data")?)?;
        let handle = host.engine.resolve_module_state("module")?;
        if !host
            .engine
            .set_stock_process_gate_value(handle, initial_parameters[1])
        {
            return Err("actual Process gate setter rejected".into());
        }
        Ok(host)
    }
    fn apply_sources(&mut self) -> Result<(), String> {
        for port in 0..3 {
            let handle = self.engine.resolve_module_state(&source_id(port))?;
            if !self
                .engine
                .set_host_reference_source(handle, self.sources[port])
            {
                return Err("actual source control rejected".into());
            }
        }
        Ok(())
    }
    fn load_data(&mut self, value: &Value) -> Result<(), String> {
        let handle = self.engine.resolve_module_state("module")?;
        if !self.engine.load_stock_process_legacy_data(handle, value) {
            return Err("production legacy load rejected".into());
        }
        Ok(())
    }
    fn snapshot_node(&self, id: &str) -> Result<HostModuleSnapshot, String> {
        let handle = self.engine.resolve_module_state(id)?;
        self.engine
            .host_module_snapshot(handle)
            .ok_or_else(|| format!("host snapshot unavailable: {id}"))
    }
    fn event(&mut self, event: &Value) -> Result<(), String> {
        let op = text(event, "op")?;
        if op == "step" {
            self.engine
                .process_poly_block(&mut [PortFrame::default(); 1]);
            return Ok(());
        }
        if op == "data_load" {
            return self.load_data(event.get("value").ok_or("missing data value")?);
        }
        if op == "save" {
            self.engine
                .stock_process_legacy_data(self.engine.resolve_module_state("module")?)
                .ok_or("actual serializer missing")?;
            return Ok(());
        }
        if op == "rate" {
            let rate = event["sample_rate"]
                .as_u64()
                .and_then(|n| u32::try_from(n).ok())
                .filter(|n| matches!(n, 44100 | 48000 | 96000))
                .ok_or("invalid target sample rate")?;
            if !self.engine.stock_process_sample_rate_event(rate) {
                return Err("actual Process Engine rate transition rejected".into());
            }
            self.patch.sample_rate = rate;
            return Ok(());
        }
        let mut patch = self.patch.clone();
        let fresh = op == "recreate";
        let mut sources = self.sources;
        let mut gate_override = None;
        match op {
            "source" => {
                let port = index(event, "port", 3)?;
                sources[port] = source_frame(&event["values_bits"])?;
                patch
                    .nodes
                    .iter_mut()
                    .find(|n| n.id == source_id(port))
                    .ok_or("source missing")?
                    .params
                    .insert("channels".into(), sources[port].channels as f32);
            }
            "parameter" => {
                let port = index(event, "port", 2)?;
                let value = scalar_bits(&event["value_bits"])?;
                if port == 0 {
                    patch
                        .nodes
                        .iter_mut()
                        .find(|n| n.id == "module")
                        .ok_or("module missing")?
                        .params
                        .insert("slew".into(), value);
                } else {
                    gate_override = Some(value);
                }
            }
            "adopt" => {
                let values = parameter_bits(&event["parameter_bits"])?;
                patch
                    .nodes
                    .iter_mut()
                    .find(|n| n.id == "module")
                    .ok_or("module missing")?
                    .params
                    .insert("slew".into(), values[0]);
                gate_override = Some(values[1]);
            }
            "reset" => {
                let module = patch
                    .nodes
                    .iter_mut()
                    .find(|n| n.id == "module")
                    .ok_or("module missing")?;
                ModuleParameterActions::new(patch.sample_rate).initialize(module)?;
            }
            "recreate" => {
                patch
                    .cables
                    .retain(|c| c.from != "module" && c.to != "module");
                *patch
                    .nodes
                    .iter_mut()
                    .find(|n| n.id == "module")
                    .ok_or("module missing")? = node("module".into(), "stock_process", json!({}))?;
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
                let source = self.engine.resolve_module_state(&from)?;
                let destination = self.engine.resolve_module_state(&to)?;
                self.engine
                    .resolve_host_port(source, PortDirection::Output, output)
                    .ok_or("unresolved output")?;
                self.engine
                    .resolve_host_port(destination, PortDirection::Input, input)
                    .ok_or("unresolved input")?;
                patch.cables.push(
                    serde_json::from_value(json!({"id":id,"from":from,
                    "output":output,"to":to,"input":input}))
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
                return Err("port layout changed".into());
            }
            let mut delta = HostPortLifecycle {
                input_presence: presence,
                output_consumers: consumers,
                bypassed,
                ..HostPortLifecycle::default()
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
        if let Some(gate) = gate_override {
            let handle = next.resolve_module_state("module")?;
            if !next.set_stock_process_gate_value(handle, gate) {
                return Err("production raw Process gate setter rejected".into());
            }
        }
        self.engine = next;
        self.patch = patch;
        self.sources = sources;
        self.apply_sources()?;
        Ok(())
    }
    fn snapshot(&self, id: &str, event: i64, op: &str) -> Result<Value, String> {
        let module = self.snapshot_node("module")?;
        if (module.input_count, module.output_count) != (3, 6) {
            return Err("wrong Process port counts".into());
        }
        let handle = self.engine.resolve_module_state("module")?;
        let controls = self
            .engine
            .stock_process_snapshot_bits(handle)
            .ok_or("actual Process controls missing")?;
        let saved_data = self
            .engine
            .stock_process_legacy_data(handle)
            .ok_or("actual serializer missing")?;
        let port = |p: &HostPortSnapshot| json!({"channels":p.channels,"values_bits":p.values.map(f32::to_bits)});
        let source_outputs = (0..3)
            .map(|i| {
                self.snapshot_node(&source_id(i))
                    .map(|s| port(&s.outputs[0]))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let sink_inputs = (0..6)
            .map(|i| self.snapshot_node(&sink_id(i)).map(|s| port(&s.inputs[0])))
            .collect::<Result<Vec<_>, _>>()?;
        let source_config = (0..3)
            .map(|i| {
                let handle = self.engine.resolve_module_state(&source_id(i))?;
                let actual = self
                    .engine
                    .get_host_reference_source(handle)
                    .ok_or("actual source control missing")?;
                Ok(actual.values[..usize::from(actual.channels)]
                    .iter()
                    .map(|v| v.to_bits())
                    .collect::<Vec<_>>())
            })
            .collect::<Result<Vec<_>, String>>()?;
        let mut input_cables = [0; 3];
        let mut output_cables = [0; 6];
        let mut source_cables = [0; 3];
        let mut sink_cables = [0; 6];
        let actual = self
            .engine
            .host_cable_registry()
            .ok_or("compiled registry missing")?;
        let mut cable_registry = Vec::with_capacity(actual.len());
        for c in actual {
            let (source, port) = if c.source == "module" {
                *output_cables
                    .get_mut(usize::from(c.source_port))
                    .ok_or("module output outside actual Process layout")? += 1;
                ("module", usize::from(c.source_port))
            } else {
                let i = (0..3)
                    .find(|&i| c.source == source_id(i))
                    .ok_or("unknown actual source")?;
                if c.source_port != 0 {
                    return Err("unexpected source local output".into());
                }
                source_cables[i] += 1;
                ("source", i)
            };
            let (destination, input) = if c.destination == "module" {
                *input_cables
                    .get_mut(usize::from(c.destination_port))
                    .ok_or("module input outside actual Process layout")? += 1;
                ("module", usize::from(c.destination_port))
            } else {
                let i = (0..6)
                    .find(|&i| c.destination == sink_id(i))
                    .ok_or("unknown actual sink")?;
                if c.destination_port != 0 {
                    return Err("unexpected sink local input".into());
                }
                sink_cables[i] += 1;
                ("sink", i)
            };
            cable_registry.push(json!({"id":c.id.ok_or("missing actual cable identity")?,
                "source":source,"port":port,"destination":destination,"input":input}));
        }
        cable_registry.sort_by(|a, b| a["id"].as_str().cmp(&b["id"].as_str()));
        let output_consumer_mask =
            output_cables
                .iter()
                .enumerate()
                .fold(
                    0_u32,
                    |mask, (i, count)| if *count > 0 { mask | (1 << i) } else { mask },
                );
        if output_consumer_mask != module.output_consumers {
            return Err("compiled consumer mask differs from registry".into());
        }
        let (sample_rate, sample_time) = self
            .engine
            .host_reference_timing()
            .ok_or("compiled timing missing")?;
        if (sample_rate, sample_time.to_bits()) != (controls.sample_rate, controls.sample_time_bits)
        {
            return Err("Process timing differs from actual Engine timing".into());
        }
        Ok(
            json!({"case":id,"event":event,"op":op,"frame":self.engine.port_light_processed_frames(),
            "sample_rate":sample_rate,"sample_time_bits":sample_time.to_bits(),
            "bypassed":module.bypassed,"parameter_bits":controls.parameter_bits,
            "engines":controls.engines,"saved_data":saved_data,
            "light_bits":controls.light_bits,
            "inputs":module.inputs[..3].iter().map(port).collect::<Vec<_>>(),
            "outputs":module.outputs[..6].iter().map(port).collect::<Vec<_>>(),
            "source_outputs":source_outputs,"sink_inputs":sink_inputs,"source_config":source_config,
            "input_cables":input_cables,"output_cables":output_cables,"source_cables":source_cables,
            "sink_cables":sink_cables,"cable_registry":cable_registry,
            "output_consumer_mask":module.output_consumers}),
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
        eprintln!("Stock Process production host probe failed: {error}");
        std::process::exit(1);
    }
}
