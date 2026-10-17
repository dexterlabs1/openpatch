//! Local Rust safety exercise for the 46 frozen Stock Scope safety inputs.
//! This invokes the actual Engine, not an acquisition model or an SDK oracle.
//! The records lie outside the independently declared ordinary SDK parity
//! domain; accepted finite extremes carry no source-equivalence assertion.
use openpatch_core::{Engine, Node, Patch, PortFrame, StockScopeDiagnosticBits};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashSet;
use std::io::{self, BufRead, Write};

const RATE: u32 = 48_000;
const ROLES: [&str; 7] = [
    "x",
    "y",
    "trigger",
    "time",
    "threshold",
    "xy",
    "trigger-mode",
];
const BITS: [u32; 6] = [
    0x7f80_0000,
    0xff80_0000,
    0x7fc0_0001,
    0xffc0_0001,
    0x7f7f_ffff,
    0xff7f_ffff,
];
const OUTSIDE_TIMES: [i32; 4] = [-200, -130, 160, 200];

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SafetyRecord {
    id: String,
    classification: String,
    role: Option<String>,
    bits: Option<u32>,
    time_raw: Option<i32>,
}

impl SafetyRecord {
    fn validate(&self) -> Result<(), String> {
        // The two frozen finite TIME extremes predate the local-only suffix.
        // Keep their original classification intact; run_record always labels
        // every output as local safety and never claims SDK equivalence.
        let frozen_finite_time_extreme = self.role.as_deref() == Some("time")
            && matches!(self.bits, Some(0x7f7f_ffff | 0xff7f_ffff))
            && self.classification == "finite TIME outside the declared bounded domain; float-to-int32 conversion in exp2Floor exceeds its range";
        if !self.classification.contains("local Rust safety only") && !frozen_finite_time_extreme {
            return Err(format!(
                "{}: missing explicit local-only classification",
                self.id
            ));
        }
        match (&self.role, self.bits, self.time_raw) {
            (Some(role), Some(bits), None)
                if ROLES.contains(&role.as_str())
                    && BITS.contains(&bits)
                    && self.id == format!("{role}/{bits:08x}") =>
            {
                Ok(())
            }
            (None, None, Some(time))
                if OUTSIDE_TIMES.contains(&time)
                    && self.id == format!("time-exp2-floor-outside/{time}") =>
            {
                Ok(())
            }
            _ => Err(format!(
                "{}: missing or invalid frozen safety schema",
                self.id
            )),
        }
    }
}

fn node(id: &str, kind: &str, params: Value) -> Result<Node, String> {
    serde_json::from_value(json!({"id":id,"kind":kind,"params":params}))
        .map_err(|error| error.to_string())
}

fn safety_engine() -> Result<Engine, String> {
    let mut patch = Patch::new_v2();
    patch.sample_rate = RATE;
    for (index, name) in ["x_source", "y_source", "trigger_source"]
        .iter()
        .enumerate()
    {
        patch
            .nodes
            .push(node(name, "poly_constant", json!({"channels":1,"v1":0}))?);
        patch.cables.push(
            serde_json::from_value(json!({
                "id":format!("input{index}"),"from":name,"output":"out",
                "to":"scope","input":(["x","y","trigger"][index])
            }))
            .map_err(|error| error.to_string())?,
        );
    }
    patch.nodes.push(node("scope", "stock_scope", json!({}))?);
    patch.nodes.push(node("x_sink", "mult", json!({}))?);
    patch.nodes.push(node("y_sink", "mult", json!({}))?);
    for (output, sink) in [("x", "x_sink"), ("y", "y_sink")] {
        patch.cables.push(
            serde_json::from_value(json!({
                "from":"scope","output":output,"to":sink,"input":"signal"
            }))
            .map_err(|error| error.to_string())?,
        );
    }
    patch.output = Some("x_sink".into());
    patch.output_port = "out1".into();
    let mut engine = patch.compile()?;
    let scope = engine.resolve_module_state("scope")?;
    if !engine.set_stock_scope_parameter_raw(scope, 4, 8.0)
        || !engine.set_stock_scope_parameter_raw(scope, 7, 0.0)
    {
        return Err("actual Scope refused bounded short-sweep setup".into());
    }
    for name in ["x_source", "y_source", "trigger_source"] {
        let source = engine.resolve_module_state(name)?;
        if !engine.set_host_reference_source(source, PortFrame::mono(-2.0)) {
            return Err(format!("actual source setup rejected: {name}"));
        }
    }
    Ok(engine)
}

fn advance(engine: &mut Engine, frames: usize) {
    let mut scratch = [PortFrame::default(); 256];
    let mut remaining = frames;
    while remaining > 0 {
        let count = remaining.min(scratch.len());
        engine.process_poly_block(&mut scratch[..count]);
        remaining -= count;
    }
}

fn diagnostic(engine: &Engine) -> Result<StockScopeDiagnosticBits, String> {
    let scope = engine.resolve_module_state("scope")?;
    engine
        .stock_scope_diagnostic_bits(scope)
        .ok_or_else(|| "actual Scope diagnostic unavailable".into())
}

fn source_name(role: &str) -> Option<&'static str> {
    match role {
        "x" => Some("x_source"),
        "y" => Some("y_source"),
        "trigger" => Some("trigger_source"),
        _ => None,
    }
}

fn parameter_index(role: &str) -> Option<usize> {
    match role {
        "time" => Some(4),
        "threshold" => Some(6),
        "xy" => Some(5),
        "trigger-mode" => Some(7),
        _ => None,
    }
}

fn source_bits(frame: PortFrame) -> Value {
    json!({"channels":frame.channels,"values_bits":frame.values.map(f32::to_bits)})
}

fn actual_source(engine: &Engine, name: &str) -> Result<PortFrame, String> {
    let handle = engine.resolve_module_state(name)?;
    engine
        .get_host_reference_source(handle)
        .ok_or_else(|| format!("actual source unavailable: {name}"))
}

fn set_source(engine: &mut Engine, name: &str, value: f32) -> Result<bool, String> {
    let handle = engine.resolve_module_state(name)?;
    Ok(engine.set_host_reference_source(handle, PortFrame::mono(value)))
}

fn run_record(record: &SafetyRecord) -> Result<Value, String> {
    let mut engine = safety_engine()?;
    let role = record.role.as_deref().unwrap_or("time");
    let waiting_case = matches!(role, "trigger" | "threshold");
    if waiting_case {
        // TIME=8 is within the declared diagnostic domain and makes every
        // capture bin one sample at 48 kHz. This reaches the completed sweep
        // before applying a trigger/threshold extreme, so rescale and the
        // tri-state detector really execute on the subsequent edge frames.
        advance(&mut engine, 260);
        if diagnostic(&engine)?.buffer_index != 256 {
            return Err(format!(
                "{}: real Scope did not reach trigger wait",
                record.id
            ));
        }
    }
    let before = diagnostic(&engine)?;
    let mut source_before = None;
    let mut source_after = None;
    let attempted_bits;
    let accepted;
    if let Some(name) = source_name(role) {
        let value = f32::from_bits(record.bits.ok_or("missing port bits")?);
        attempted_bits = value.to_bits();
        let original = actual_source(&engine, name)?;
        accepted = set_source(&mut engine, name, value)?;
        let now = actual_source(&engine, name)?;
        source_before = Some(source_bits(original));
        source_after = Some(source_bits(now));
        if accepted && now.values[0].to_bits() != attempted_bits {
            return Err(format!("{}: accepted raw source bits changed", record.id));
        }
        if !accepted && now != original {
            return Err(format!("{}: rejected raw source changed", record.id));
        }
    } else {
        let (index, value) = match record.bits {
            Some(bits) => (
                parameter_index(role).ok_or("invalid control role")?,
                f32::from_bits(bits),
            ),
            None => (4, record.time_raw.ok_or("missing raw TIME")? as f32),
        };
        attempted_bits = value.to_bits();
        let scope = engine.resolve_module_state("scope")?;
        accepted = engine.set_stock_scope_parameter_raw(scope, index, value);
        let immediate = diagnostic(&engine)?;
        if accepted && immediate.parameter_bits[index] != attempted_bits {
            return Err(format!(
                "{}: accepted raw parameter bits changed",
                record.id
            ));
        }
    }
    let immediate = diagnostic(&engine)?;
    let expected_acceptance = record.bits.is_some_and(|bits| {
        let value = f32::from_bits(bits);
        value.is_finite() && (role != "time" || (-6.0..=8.0).contains(&value))
    });
    if accepted != expected_acceptance {
        return Err(format!(
            "{}: actual API acceptance violates bounded local policy",
            record.id
        ));
    }
    let rejected_atomically = !accepted && immediate == before && source_before == source_after;
    if !accepted && !rejected_atomically {
        return Err(format!(
            "{}: rejected update changed actual Engine state",
            record.id
        ));
    }

    let mut edge_inputs_bits = Vec::new();
    if role == "threshold" {
        // Continue from the real completed sweep. The threshold extreme is
        // exercised against a low, high, and both finite f32 extremes; a
        // rejected nonfinite threshold leaves the safe prior control in use.
        for edge in [-2.0, 2.0, -f32::MAX, f32::MAX] {
            if !set_source(&mut engine, "trigger_source", edge)? {
                return Err("actual finite trigger edge rejected".into());
            }
            edge_inputs_bits.push(edge.to_bits());
            advance(&mut engine, 2);
        }
    } else {
        advance(&mut engine, if waiting_case { 2 } else { 3 });
    }
    let scope = engine.resolve_module_state("scope")?;
    let host = engine
        .host_module_snapshot(scope)
        .ok_or("actual Scope host snapshot unavailable")?;
    let final_snapshot = diagnostic(&engine)?;
    if final_snapshot.point_buffer.len() != 256
        || final_snapshot.current_points.len() != 2
        || final_snapshot.triggers.len() != 16
    {
        return Err(format!("{}: incomplete real diagnostic", record.id));
    }
    Ok(json!({
        "id":record.id,
        "role":role,
        "input_classification":record.classification,
        "policy":"local Rust finite-input and bounded-TIME safety only; no SDK/source equivalence assertion",
        "attempted_bits":attempted_bits,
        "accepted":accepted,
        "rejected_atomically":rejected_atomically,
        "waiting_precondition":waiting_case,
        "before_attempt":{"parameter_bits":before.parameter_bits,
                          "buffer_index":before.buffer_index,
                          "frame_index":before.frame_index,
                          "trigger_states":before.triggers},
        "after_attempt":{"parameter_bits":immediate.parameter_bits,
                         "buffer_index":immediate.buffer_index,
                         "frame_index":immediate.frame_index,
                         "trigger_states":immediate.triggers},
        "source_before":source_before,
        "source_after":source_after,
        "edge_inputs_bits":edge_inputs_bits,
        "host":host,
        "diagnostic":final_snapshot,
    }))
}

fn read_records() -> Result<Vec<SafetyRecord>, String> {
    let stdin = io::stdin();
    let mut records = Vec::new();
    let mut ids = HashSet::new();
    for (line_number, line) in stdin.lock().lines().enumerate() {
        let line = line.map_err(|error| error.to_string())?;
        let record: SafetyRecord = serde_json::from_str(&line)
            .map_err(|error| format!("line {}: {error}", line_number + 1))?;
        record.validate()?;
        if !ids.insert(record.id.clone()) {
            return Err(format!("duplicate safety id {}", record.id));
        }
        records.push(record);
    }
    if records.len() != 46 {
        return Err(format!(
            "expected exactly 46 frozen safety records, received {}",
            records.len()
        ));
    }
    for role in ROLES {
        for bits in BITS {
            if !ids.contains(&format!("{role}/{bits:08x}")) {
                return Err(format!("missing frozen safety record {role}/{bits:08x}"));
            }
        }
    }
    for time in OUTSIDE_TIMES {
        if !ids.contains(&format!("time-exp2-floor-outside/{time}")) {
            return Err(format!("missing frozen TIME safety record {time}"));
        }
    }
    Ok(records)
}

fn run() -> Result<(), String> {
    let records = read_records()?;
    let stdout = io::stdout();
    let mut output = stdout.lock();
    for record in &records {
        let row = run_record(record)?;
        serde_json::to_writer(&mut output, &row).map_err(|error| error.to_string())?;
        output.write_all(b"\n").map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("Stock Scope local safety probe: {error}");
        std::process::exit(1);
    }
}
