//! Actual Core Engine host trace adapter for the independent Stock Noise JSONL
//! declaration. This file contains no reference waveform implementation.
use openpatch_core::{
    Engine, HostModuleSnapshot, HostPortLifecycle, HostPortSnapshot, InputLifecycleTransform, Node,
    OutputLifecycleTransform, Patch, PortFrame,
};
use serde_json::{json, Value};
use std::io::{self, BufRead, Write};

const OUTPUTS: [&str; 7] = ["white", "pink", "red", "violet", "blue", "gray", "black"];

fn string<'a>(value: &'a Value, field: &str) -> Result<&'a str, String> {
    value[field]
        .as_str()
        .ok_or_else(|| format!("missing string {field}"))
}
fn index(value: &Value, field: &str, limit: usize) -> Result<usize, String> {
    value[field]
        .as_u64()
        .and_then(|n| usize::try_from(n).ok())
        .filter(|&n| n < limit)
        .ok_or_else(|| format!("invalid {field}"))
}
fn tape(value: &Value, field: &str) -> Result<Vec<u32>, String> {
    value[field]
        .as_array()
        .filter(|a| !a.is_empty())
        .ok_or_else(|| format!("missing {field} tape"))?
        .iter()
        .map(|v| {
            v.as_u64()
                .and_then(|n| u32::try_from(n).ok())
                .ok_or_else(|| format!("invalid {field} word"))
        })
        .collect()
}
fn node(id: String, kind: &str) -> Result<Node, String> {
    serde_json::from_value(json!({"id":id,"kind":kind})).map_err(|e| e.to_string())
}
fn sink_id(port: usize) -> String {
    format!("sink{port}")
}
fn cable(id: &str, output: usize, input: usize) -> Result<openpatch_core::Cable, String> {
    serde_json::from_value(json!({"id":id,"from":"module","output":OUTPUTS[output],
        "to":sink_id(input),"input":"signal"}))
    .map_err(|e| e.to_string())
}
fn raw(port: &HostPortSnapshot) -> Value {
    json!({"channels":port.channels,"values_bits":port.values.map(f32::to_bits)})
}

struct Host {
    patch: Patch,
    engine: Engine,
    normal_bits: Vec<u32>,
    uniform_bits: Vec<u32>,
    frame: u64,
    module_incarnation: u32,
}
impl Host {
    fn new(rate: u32, initial: &Value) -> Result<Self, String> {
        let mask = index(initial, "output_mask", 128)?;
        let normal_bits = tape(initial, "normal_bits")?;
        let uniform_bits = tape(initial, "uniform_bits")?;
        let mut patch = Patch::new_v2();
        patch.sample_rate = rate;
        patch.nodes.push(node("module".into(), "stock_noise")?);
        for port in 0..7 {
            patch.nodes.push(node(sink_id(port), "poly_viz")?);
        }
        for port in 0..7 {
            if mask & (1 << port) != 0 {
                patch.cables.push(cable(&format!("out{port}"), port, port)?);
            }
        }
        let mut engine = patch.compile()?;
        let handle = engine.resolve_module_state("module")?;
        if !engine.set_stock_noise_controlled_draws(handle, &normal_bits, &uniform_bits) {
            return Err("initial controlled draw configuration rejected".into());
        }
        for port in 0..7 {
            let sink = engine.resolve_module_state(&sink_id(port))?;
            if !engine.enable_host_input_observer(sink, 0) {
                return Err(format!("sink{port} input observation rejected"));
            }
        }
        Ok(Self {
            patch,
            engine,
            normal_bits,
            uniform_bits,
            frame: 0,
            module_incarnation: 0,
        })
    }

    fn module(&self, id: &str) -> Result<HostModuleSnapshot, String> {
        self.engine
            .host_module_snapshot(self.engine.resolve_module_state(id)?)
            .ok_or_else(|| format!("missing host snapshot for {id}"))
    }

    fn event(&mut self, event: &Value) -> Result<(), String> {
        let op = string(event, "op")?;
        if op == "step" {
            let frames = event.get("frames").map_or(Ok(1), |v| {
                v.as_u64()
                    .and_then(|n| usize::try_from(n).ok())
                    .filter(|&n| (1..=2048).contains(&n))
                    .ok_or("invalid frames")
            })?;
            let mut output = vec![PortFrame::default(); frames];
            self.engine.process_poly_block(&mut output);
            self.frame += frames as u64;
            return Ok(());
        }
        if op == "reset" {
            let handle = self.engine.resolve_module_state("module")?;
            if !self.engine.initialize_module_state(handle) {
                return Err("source reset was rejected".into());
            }
            return Ok(());
        }

        let mut patch = self.patch.clone();
        let fresh = op == "recreate";
        match op {
            "connect" => {
                let id = string(event, "id")?;
                if string(event, "source")? != "module" || string(event, "destination")? != "sink" {
                    return Err("unexpected cable endpoint role".into());
                }
                if patch.cables.iter().any(|c| c.id.as_deref() == Some(id)) {
                    return Err(format!("duplicate cable {id}"));
                }
                patch.cables.push(cable(
                    id,
                    index(event, "port", 7)?,
                    index(event, "input", 7)?,
                )?);
            }
            "disconnect" => {
                let id = string(event, "id")?;
                let at = patch
                    .cables
                    .iter()
                    .position(|c| c.id.as_deref() == Some(id))
                    .ok_or_else(|| format!("missing cable {id}"))?;
                patch.cables.remove(at);
            }
            "bypass" => {
                patch.nodes[0].bypassed = event["value"].as_bool().ok_or("invalid bypass")?;
            }
            "adopt" => {}
            "recreate" => {
                patch
                    .cables
                    .retain(|c| c.from != "module" && c.to != "module");
                patch.nodes[0] = node("module".into(), "stock_noise")?;
            }
            _ => return Err(format!("unknown event {op}")),
        }

        let prior = self
            .engine
            .stock_noise_snapshot(self.engine.resolve_module_state("module")?)
            .ok_or("missing prior stock noise runtime")?;
        let mut next = patch.compile()?;
        let next_module = next.resolve_module_state("module")?;
        let (normal_start, uniform_start) = if fresh {
            (prior.normal_draws, prior.uniform_draws)
        } else {
            (0, 0)
        };
        if !next.set_stock_noise_controlled_draws_at(
            next_module,
            &self.normal_bits,
            &self.uniform_bits,
            normal_start,
            uniform_start,
        ) {
            return Err("receiving controlled draw configuration rejected".into());
        }
        for port in 0..7 {
            let sink = next.resolve_module_state(&sink_id(port))?;
            if !next.enable_host_input_observer(sink, 0) {
                return Err(format!("sink{port} input observation rejected"));
            }
        }

        // Resolve every host transition before adopting. A fresh Noise node has
        // new raw output storage; unchanged sink instances retain actual input
        // storage and receive their physical cable effects.
        let mut prepared = Vec::with_capacity(patch.nodes.len());
        for node in &patch.nodes {
            if fresh && node.id == "module" {
                continue;
            }
            let old_handle = self.engine.resolve_module_state(&node.id)?;
            let new_handle = next.resolve_module_state(&node.id)?;
            let (old_inputs, old_outputs, old_presence, old_consumers, old_bypass) = self
                .engine
                .host_port_layout(old_handle)
                .ok_or("old host layout unavailable")?;
            let (inputs, outputs, presence, consumers, bypassed) = next
                .host_port_layout(new_handle)
                .ok_or("new host layout unavailable")?;
            if (old_inputs, old_outputs) != (inputs, outputs) {
                return Err("host port layout changed".into());
            }
            let mut delta = HostPortLifecycle {
                input_presence: presence,
                output_consumers: consumers,
                bypassed,
                ..HostPortLifecycle::default()
            };
            for port in 0..usize::from(inputs) {
                delta.inputs[port] =
                    match (old_presence & (1 << port) != 0, presence & (1 << port) != 0) {
                        (false, true) => InputLifecycleTransform::connect(),
                        (true, false) => InputLifecycleTransform::disconnect(),
                        _ => InputLifecycleTransform::identity(),
                    };
            }
            for port in 0..usize::from(outputs) {
                delta.outputs[port] = match (
                    old_consumers & (1 << port) != 0,
                    consumers & (1 << port) != 0,
                ) {
                    (false, true) => OutputLifecycleTransform::connect(),
                    (true, false) => OutputLifecycleTransform::disconnect(),
                    _ => OutputLifecycleTransform::identity(),
                };
                if old_bypass != bypassed {
                    delta.outputs[port] =
                        delta.outputs[port].then(OutputLifecycleTransform::bypass_edge());
                }
            }
            prepared.push((new_handle, delta));
        }
        if fresh {
            next.adopt_state_from_excluding(&self.engine, &[next_module]);
        } else {
            next.adopt_state_from(&self.engine);
        }
        for (handle, delta) in prepared {
            if !next.apply_host_port_lifecycle(handle, &delta) {
                return Err("prepared host lifecycle rejected".into());
            }
        }
        self.patch = patch;
        self.engine = next;
        if fresh {
            self.module_incarnation += 1;
        }
        Ok(())
    }

    fn snapshot(&self, case: &str, event: i64, op: &str) -> Result<Value, String> {
        let module = self.module("module")?;
        if (module.input_count, module.output_count) != (0, 7) {
            return Err("stock noise host port layout mismatch".into());
        }
        let handle = self.engine.resolve_module_state("module")?;
        let state = self
            .engine
            .stock_noise_snapshot(handle)
            .ok_or("missing stock noise runtime")?;
        if self.engine.resolve_module_lights("module").is_ok() || !state.light_bits.is_empty() {
            return Err("stock noise unexpectedly has module lamps".into());
        }
        let (sample_rate, sample_time) = self
            .engine
            .host_reference_timing()
            .ok_or("missing Engine timing")?;
        let outputs = module.outputs[..7].iter().map(raw).collect::<Vec<_>>();
        let sink_inputs = (0..7)
            .map(|port| self.module(&sink_id(port)).map(|s| raw(&s.inputs[0])))
            .collect::<Result<Vec<_>, _>>()?;
        let sink_seen = (0..7)
            .map(|port| {
                let sink = self.engine.resolve_module_state(&sink_id(port))?;
                self.engine
                    .observed_host_input(sink, 0)
                    .map(|seen| raw(&seen))
                    .ok_or_else(|| format!("missing sink{port} process-time observation"))
            })
            .collect::<Result<Vec<_>, String>>()?;
        let mut output_cables = [0_u32; 7];
        let mut sink_cables = [0_u32; 7];
        let mut registry = Vec::new();
        for edge in self
            .engine
            .host_cable_registry()
            .ok_or("missing cable registry")?
        {
            if edge.source != "module" || edge.destination_port != 0 {
                return Err("unexpected compiled cable endpoint".into());
            }
            let output = usize::from(edge.source_port);
            let input = (0..7)
                .find(|&port| edge.destination == sink_id(port))
                .ok_or("unknown sink endpoint")?;
            if output >= 7 {
                return Err("invalid stock output index".into());
            }
            output_cables[output] += 1;
            sink_cables[input] += 1;
            registry.push(json!({"id":edge.id.ok_or("cable identity missing")?,
                "source":"module","port":output,"destination":"sink","input":input}));
        }
        registry.sort_by(|a, b| a["id"].as_str().cmp(&b["id"].as_str()));
        Ok(json!({
            "case":case,"event":event,"op":op,"frame":self.frame,
            "module_incarnation":self.module_incarnation,
            "sample_rate":sample_rate,"sample_time_bits":sample_time.to_bits(),
            "bypassed":module.bypassed,"outputs":outputs,"sink_inputs":sink_inputs,
            "sink_seen":sink_seen,
            "output_cables":output_cables,"sink_cables":sink_cables,
            "cable_registry":registry,"output_consumer_mask":module.output_consumers,
            "normal_draws":state.normal_draws,"uniform_draws":state.uniform_draws,
            "last_normal_bits":state.last_normal_bits,"last_uniform_bits":state.last_uniform_bits,
            "draw_events":state.draw_events,
            "red_x_bits":state.red_x_bits,"red_y_bits":state.red_y_bits,
            "last_white_bits":state.last_white_bits,"last_pink_bits":state.last_pink_bits,
            "pink_frame":state.pink_frame,"pink_values_bits":state.pink_values_bits,
            "gray_frame":state.gray_frame,"gray_input_bits":state.gray_input_bits,
            "gray_output_bits":state.gray_output_bits,
            "black_divider_clock":state.black_divider_clock,
            "black_divider_division":state.black_divider_division,"light_bits":state.light_bits,
        }))
    }
}

fn run() -> Result<(), String> {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut output = io::BufWriter::new(stdout.lock());
    for line in stdin.lock().lines() {
        let line = line.map_err(|e| e.to_string())?;
        if line.trim().is_empty() {
            return Err("blank case line".into());
        }
        let case: Value = serde_json::from_str(&line).map_err(|e| e.to_string())?;
        let rate = case["sample_rate"]
            .as_u64()
            .filter(|r| (8000..=192000).contains(r))
            .ok_or("invalid sample rate")? as u32;
        let id = string(&case, "id")?;
        let mut host = Host::new(rate, &case["initial"])?;
        serde_json::to_writer(&mut output, &host.snapshot(id, -1, "initial")?)
            .map_err(|e| e.to_string())?;
        writeln!(output).map_err(|e| e.to_string())?;
        for (event_index, event) in case["events"]
            .as_array()
            .ok_or("missing events array")?
            .iter()
            .enumerate()
        {
            host.event(event)?;
            serde_json::to_writer(
                &mut output,
                &host.snapshot(id, event_index as i64, string(event, "op")?)?,
            )
            .map_err(|e| e.to_string())?;
            writeln!(output).map_err(|e| e.to_string())?;
        }
    }
    output.flush().map_err(|e| e.to_string())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("Stock Noise production host probe failed: {error}");
        std::process::exit(1);
    }
}
