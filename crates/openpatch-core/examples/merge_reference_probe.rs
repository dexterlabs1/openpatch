//! Text-only adapter to the public production Patch/Engine API; no DSP mirror.
use openpatch_core::{Cable, Node, Patch, PortFrame};
use std::collections::BTreeMap;
use std::io::{self, BufRead, Write};

fn run() -> Result<(), String> {
    let rate = std::env::args()
        .nth(1)
        .ok_or("missing sample rate")?
        .parse::<u32>()
        .map_err(|e| e.to_string())?;
    let stdout = io::stdout();
    let mut writer = io::BufWriter::new(stdout.lock());
    for row in io::stdin().lock().lines() {
        let row = row.map_err(|e| e.to_string())?;
        let fields = row
            .split_whitespace()
            .map(str::parse::<f32>)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        if fields.len() != 273 || fields.iter().any(|v| !v.is_finite()) {
            return Err("expected 273 finite fields".into());
        }
        let selection = fields[0];
        if selection.fract() != 0.0 || !(-1.0..=16.0).contains(&selection) {
            return Err("invalid channel selection".into());
        }
        let mut patch = Patch {
            sample_rate: rate,
            output: Some("merge".into()),
            ..Patch::new_v2()
        };
        for slot in 0..16 {
            let start = 1 + slot * 17;
            let width = fields[start];
            if width.fract() != 0.0 || !(0.0..=16.0).contains(&width) {
                return Err("invalid source width".into());
            }
            let id = format!("source{slot}");
            let mut params = BTreeMap::from([("channels".into(), width)]);
            for lane in 0..16 {
                // Public sources validate even inactive saved parameters. As with
                // Rack cable bookkeeping, missing lanes enter the graph as zero;
                // protocol padding is not direct PortFrame padding injection.
                let value = if lane < width as usize {
                    fields[start + lane + 1]
                } else {
                    0.0
                };
                params.insert(format!("v{}", lane + 1), value);
            }
            patch.nodes.push(Node {
                id: id.clone(),
                kind: "poly_constant".into(),
                params,
                bypassed: false,
                content: None,
                rackmcp_sequence: None,
                score: None,
            });
            patch.cables.push(Cable {
                from: id,
                to: "merge".into(),
                input: format!("in{}", slot + 1),
                output: "out".into(),
                id: None,
                color: None,
                input_order: None,
                output_order: None,
            });
        }
        patch.nodes.push(Node {
            id: "merge".into(),
            kind: "poly_merge".into(),
            params: BTreeMap::from([("channels".into(), selection)]),
            bypassed: false,
            content: None,
            rackmcp_sequence: None,
            score: None,
        });
        let mut engine = patch.compile()?;
        let handle = engine.resolve_module_lights("merge")?;
        let mut output = [PortFrame::default(); 2];
        engine.process_poly_block(&mut output);
        let observation = engine
            .module_lights(handle)
            .ok_or("missing Merge observation")?;
        let count = observation.output_channels.ok_or("missing output count")?;
        let automatic = observation
            .automatic_channels
            .ok_or("missing automatic count")?;
        if count != output[1].channels {
            return Err("published count differs from actual output frame".into());
        }
        write!(writer, "{count} {automatic}").map_err(|e| e.to_string())?;
        for value in output[1].values {
            write!(writer, " {value:.9e}").map_err(|e| e.to_string())?;
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
