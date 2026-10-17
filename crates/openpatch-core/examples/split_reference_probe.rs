//! Text-only adapter to public production Engine taps; no DSP mirror or test API.
use openpatch_core::{Cable, Node, Patch, TapSample};
pub use openpatch_core::{ModuleLightState, PortFrame, MAX_CHANNELS};
// Compile the exact private production module for the separate raw-storage
// phase. Lifecycle methods are intentionally unused by this fresh-row adapter.
#[allow(dead_code)]
#[path = "../src/split.rs"]
mod raw_split;
use std::collections::BTreeMap;
use std::io::{self, BufRead, Write};

fn run() -> Result<(), String> {
    let rate = std::env::args()
        .nth(1)
        .ok_or("missing sample rate")?
        .parse::<u32>()
        .map_err(|e| e.to_string())?;
    let raw = std::env::args().nth(2).as_deref() == Some("--raw");
    let stdout = io::stdout();
    let mut writer = io::BufWriter::new(stdout.lock());
    for row in io::stdin().lock().lines() {
        let row = row.map_err(|e| e.to_string())?;
        if raw {
            let fields = row.split_whitespace().collect::<Vec<_>>();
            if fields.len() != 17 {
                return Err("expected width and sixteen raw bit fields".into());
            }
            let channels = fields[0].parse::<u8>().map_err(|e| e.to_string())?;
            if channels > 16 {
                return Err("invalid raw width".into());
            }
            let mut input = PortFrame {
                channels,
                ..PortFrame::default()
            };
            for (lane, field) in fields[1..].iter().enumerate() {
                if field.len() != 8 {
                    return Err("raw values must have eight hex digits".into());
                }
                input.values[lane] =
                    f32::from_bits(u32::from_str_radix(field, 16).map_err(|e| e.to_string())?);
            }
            let mut split = raw_split::Split::default();
            if split.lights().input_channels != Some(0) {
                return Err("initial raw count must be zero".into());
            }
            let output = split.process(&input);
            let observed = split
                .lights()
                .input_channels
                .ok_or("missing raw input count")?;
            write!(writer, "{observed}").map_err(|e| e.to_string())?;
            for frame in output {
                write!(
                    writer,
                    " {} {:08x}",
                    frame.channels,
                    frame.values[0].to_bits()
                )
                .map_err(|e| e.to_string())?;
            }
            writeln!(writer).map_err(|e| e.to_string())?;
            continue;
        }
        let fields = row
            .split_whitespace()
            .map(str::parse::<f32>)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        if fields.len() != 17 || fields.iter().any(|v| !v.is_finite() || v.abs() > 16.0) {
            return Err("expected seventeen finite fields within +/-16".into());
        }
        let width = fields[0];
        if width.fract() != 0.0 || !(0.0..=16.0).contains(&width) {
            return Err("invalid source width".into());
        }
        if fields[1 + width as usize..]
            .iter()
            .any(|v| v.to_bits() != 0)
        {
            return Err("inactive protocol padding must be positive zero".into());
        }
        let mut params = BTreeMap::from([("channels".into(), width)]);
        for lane in 0..16 {
            params.insert(format!("v{}", lane + 1), fields[lane + 1]);
        }
        let p = Patch {
            sample_rate: rate,
            nodes: vec![
                Node {
                    id: "source".into(),
                    kind: "poly_constant".into(),
                    params,
                    bypassed: false,
                    content: None,
                    rackmcp_sequence: None,
                    score: None,
                },
                Node {
                    id: "split".into(),
                    kind: "poly_split".into(),
                    params: BTreeMap::new(),
                    bypassed: false,
                    content: None,
                    rackmcp_sequence: None,
                    score: None,
                },
            ],
            cables: vec![Cable {
                from: "source".into(),
                to: "split".into(),
                input: "signal".into(),
                output: "out".into(),
                id: None,
                color: None,
                input_order: None,
                output_order: None,
            }],
            ..Patch::new_v2()
        };
        let mut engine = p.compile()?;
        let handle = engine.resolve_module_lights("split")?;
        if engine.module_lights(handle).and_then(|s| s.input_channels) != Some(0) {
            return Err("initial Split count must be zero".into());
        }
        let mut taps = [None; 16];
        for (lane, tap) in taps.iter_mut().enumerate() {
            *tap = Some(engine.resolve_tap("split", &format!("out{}", lane + 1), 0)?);
        }
        let mut audio = [[0.0; 2]; 2];
        let mut samples = [[TapSample::default(); 16]; 2];
        engine.process_audio_block_with_taps(&mut audio, &taps, &mut samples);
        let observed = engine
            .module_lights(handle)
            .and_then(|s| s.input_channels)
            .ok_or("missing Split input-count observation")?;
        write!(writer, "{observed}").map_err(|e| e.to_string())?;
        for sample in samples[1] {
            write!(writer, " {} {:.9e}", u8::from(sample.active), sample.value)
                .map_err(|e| e.to_string())?;
        }
        writeln!(writer).map_err(|e| e.to_string())?;
    }
    writer.flush().map_err(|e| e.to_string())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
