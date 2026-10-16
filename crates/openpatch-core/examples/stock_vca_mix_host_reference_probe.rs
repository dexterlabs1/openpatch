//! Production Engine adapter for independently declared Stock VCA Mix host traces.
//! It reports actual Core state and contains no expected mixer arithmetic.
use openpatch_core::{
    Engine, HostModuleSnapshot, HostPortLifecycle, HostPortSnapshot, InputLifecycleTransform,
    ModuleParameterActions, Node, OutputLifecycleTransform, Patch, PortDirection, PortFrame,
};
use serde_json::{json, Value};
use std::io::{self, BufRead, Write};

const INPUTS: [&str; 9] = [
    "mix_cv", "in1", "in2", "in3", "in4", "cv1", "cv2", "cv3", "cv4",
];
const OUTPUTS: [&str; 5] = ["mix", "out1", "out2", "out3", "out4"];
const PARAMS: [&str; 5] = ["mix_level", "level1", "level2", "level3", "level4"];

fn parameter_bits(value: &Value) -> Result<[f32; 5], String> {
    let values = value
        .as_array()
        .filter(|a| a.len() == 5)
        .ok_or("five parameter bits required")?;
    let mut out = [1.0; 5];
    for i in 0..5 {
        out[i] = scalar_bits(&values[i])?;
    }
    Ok(out)
}
fn parameter_json(parameters: &[f32; 5]) -> Value {
    json!({"mix_level":parameters[0],"level1":parameters[1],"level2":parameters[2],"level3":parameters[3],"level4":parameters[4]})
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
        "source" => Ok((source_id(index(event, "port", 9)?), "out")),
        "module" => Ok(("module".into(), OUTPUTS[index(event, "port", 5)?])),
        role => Err(format!("unsupported source role {role}")),
    }
}
fn destination_endpoint(event: &Value) -> Result<(String, &'static str), String> {
    match text(event, "destination")? {
        "module" => Ok(("module".into(), INPUTS[index(event, "input", 9)?])),
        "sink" => Ok((sink_id(index(event, "input", 5)?), "signal")),
        role => Err(format!("unsupported destination role {role}")),
    }
}

struct Host {
    patch: Patch,
    engine: Engine,
    sources: [PortFrame; 9],
}
impl Host {
    fn new(rate: u32, initial: &Value) -> Result<Self, String> {
        let values = initial["sources"]
            .as_array()
            .filter(|a| a.len() == 9)
            .ok_or("nine source controls required")?;
        let mut sources = [PortFrame::default(); 9];
        for i in 0..9 {
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
        patch.nodes.push(node(
            "module".into(),
            "stock_vca_mix",
            parameter_json(&parameter_bits(&initial["parameter_bits"])?),
        )?);
        for port in 0..5 {
            patch.nodes.push(node(sink_id(port), "mult", json!({}))?);
        }
        patch.output = Some(sink_id(0));
        patch.output_port = "out1".into();
        let input_mask = index(initial, "input_mask", 512)?;
        let output_mask = index(initial, "output_mask", 32)?;
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
        Ok(host)
    }
    fn apply_sources(&mut self) -> Result<(), String> {
        for port in 0..9 {
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
        if !self.engine.load_stock_vca_mix_legacy_data(handle, value) {
            return Err("production legacy load rejected".into());
        }
        let actual = self
            .engine
            .stock_vca_mix_snapshot(handle)
            .ok_or("actual VCA Mix state missing")?;
        let node = self
            .patch
            .nodes
            .iter_mut()
            .find(|n| n.id == "module")
            .ok_or("module missing")?;
        node.params
            .insert("channel_exp".into(), f32::from(u8::from(actual.ch_exp)));
        node.params
            .insert("mix_exp".into(), f32::from(u8::from(actual.mix_exp)));
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
                .stock_vca_mix_legacy_data(self.engine.resolve_module_state("module")?)
                .ok_or("actual serializer missing")?;
            return Ok(());
        }
        let mut patch = self.patch.clone();
        let fresh = op == "recreate";
        let mut sources = self.sources;
        match op {
            "source" => {
                let port = index(event, "port", 9)?;
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
                let port = index(event, "port", 5)?;
                patch
                    .nodes
                    .iter_mut()
                    .find(|n| n.id == "module")
                    .ok_or("module missing")?
                    .params
                    .insert(PARAMS[port].into(), scalar_bits(&event["value_bits"])?);
            }
            "adopt" => {
                let values = parameter_bits(&event["parameter_bits"])?;
                let module = patch
                    .nodes
                    .iter_mut()
                    .find(|n| n.id == "module")
                    .ok_or("module missing")?;
                for (name, value) in PARAMS.iter().zip(values) {
                    module.params.insert((*name).into(), value);
                }
                // Receiving JSON fields win only where present. Missing fields
                // retain the current durable aliases, as dataFromJson does.
                let mut flags = self
                    .engine
                    .stock_vca_mix_snapshot(self.engine.resolve_module_state("module")?)
                    .ok_or("actual flags missing")?;
                if let Some(object) = event["data"].as_object() {
                    if let Some(value) = object.get("chExp") {
                        flags.ch_exp = value.as_bool().unwrap_or(false);
                    }
                    if let Some(value) = object.get("mixExp") {
                        flags.mix_exp = value.as_bool().unwrap_or(false);
                    }
                }
                module
                    .params
                    .insert("channel_exp".into(), f32::from(u8::from(flags.ch_exp)));
                module
                    .params
                    .insert("mix_exp".into(), f32::from(u8::from(flags.mix_exp)));
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
                    .ok_or("module missing")? = node("module".into(), "stock_vca_mix", json!({}))?;
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
        self.engine = next;
        self.patch = patch;
        self.sources = sources;
        self.apply_sources()?;
        Ok(())
    }
    fn snapshot(&self, id: &str, event: i64, op: &str) -> Result<Value, String> {
        let module = self.snapshot_node("module")?;
        if (module.input_count, module.output_count) != (9, 5) {
            return Err("wrong VCA Mix port counts".into());
        }
        let handle = self.engine.resolve_module_state("module")?;
        let controls = self
            .engine
            .stock_vca_mix_snapshot(handle)
            .ok_or("actual VCA Mix controls missing")?;
        let saved_data = self
            .engine
            .stock_vca_mix_legacy_data(handle)
            .ok_or("actual serializer missing")?;
        let port = |p: &HostPortSnapshot| json!({"channels":p.channels,"values_bits":p.values.map(f32::to_bits)});
        let source_outputs = (0..9)
            .map(|i| {
                self.snapshot_node(&source_id(i))
                    .map(|s| port(&s.outputs[0]))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let sink_inputs = (0..5)
            .map(|i| self.snapshot_node(&sink_id(i)).map(|s| port(&s.inputs[0])))
            .collect::<Result<Vec<_>, _>>()?;
        let source_config = (0..9)
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
        let mut input_cables = [0; 9];
        let mut output_cables = [0; 5];
        let mut source_cables = [0; 9];
        let mut sink_cables = [0; 5];
        let actual = self
            .engine
            .host_cable_registry()
            .ok_or("compiled registry missing")?;
        let mut cable_registry = Vec::with_capacity(actual.len());
        for c in actual {
            let (source, port) = if c.source == "module" {
                output_cables[usize::from(c.source_port)] += 1;
                ("module", usize::from(c.source_port))
            } else {
                let i = (0..9)
                    .find(|&i| c.source == source_id(i))
                    .ok_or("unknown actual source")?;
                if c.source_port != 0 {
                    return Err("unexpected source local output".into());
                }
                source_cables[i] += 1;
                ("source", i)
            };
            let (destination, input) = if c.destination == "module" {
                input_cables[usize::from(c.destination_port)] += 1;
                ("module", usize::from(c.destination_port))
            } else {
                let i = (0..5)
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
        let meters = controls.meters.map(
            |m| json!({"v_bits":m.v.to_bits(),"mode":m.mode,"lambda_bits":m.lambda.to_bits()}),
        );
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
        Ok(
            json!({"case":id,"event":event,"op":op,"frame":self.engine.port_light_processed_frames(),
            "sample_rate":sample_rate,"sample_time_bits":sample_time.to_bits(),
            "bypassed":module.bypassed,"parameter_bits":controls.parameters.map(f32::to_bits),
            "ch_exp":controls.ch_exp,"mix_exp":controls.mix_exp,"saved_data":saved_data,
            "meters":meters,"divider_clock":controls.divider_clock,"divider_division":controls.divider_division,
            "light_bits":controls.light_brightness.map(f32::to_bits),
            "inputs":module.inputs[..9].iter().map(port).collect::<Vec<_>>(),
            "outputs":module.outputs[..5].iter().map(port).collect::<Vec<_>>(),
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
        eprintln!("Stock VCA Mix production host probe failed: {error}");
        std::process::exit(1);
    }
}
