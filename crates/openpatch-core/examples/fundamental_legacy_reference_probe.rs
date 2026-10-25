//! Production side of scripts/fundamental_legacy_reference: drives the real
//! Core VCA-2 and Unity modules with the same token protocol as
//! reference_probe.cpp. Original OpenPatch harness, MIT licensed.
use openpatch_core::{Node, PortFrame, StockUnityProbe, StockVca2Probe};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::io::{self, BufRead, BufWriter, Write};

fn bits(token: &str) -> f32 {
    f32::from_bits(u32::from_str_radix(token, 16).expect("hex bits"))
}

fn node(kind: &str, params: &[(&str, f32)]) -> Node {
    Node {
        id: "probe".into(),
        kind: kind.into(),
        params: params
            .iter()
            .map(|(k, v)| (k.to_string(), *v))
            .collect::<BTreeMap<_, _>>(),
        content: None,
        rackmcp_sequence: None,
        score: None,
        bypassed: false,
    }
}

fn run_vca(out: &mut impl Write) {
    for line in io::stdin().lock().lines() {
        let line = line.unwrap();
        let tokens: Vec<&str> = line.split_whitespace().collect();
        assert_eq!(tokens.len(), 3 + 6 * 17, "frame line");
        assert_eq!(tokens[0], "F");
        let vca = StockVca2Probe::new(&node(
            "stock_vca2",
            &[("level1", bits(tokens[1])), ("level2", bits(tokens[2]))],
        ))
        .unwrap();
        let mut inputs = [PortFrame::default(); 6];
        for (index, input) in inputs.iter_mut().enumerate() {
            let at = 3 + index * 17;
            input.channels = tokens[at].parse().unwrap();
            for lane in 0..16 {
                // Lanes above the channel count are poisoned: the module must
                // read them as the zero a Rack cable leaves there.
                input.values[lane] = if lane < usize::from(input.channels) {
                    bits(tokens[at + 1 + lane])
                } else {
                    f32::NAN
                };
            }
        }
        let outputs = vca.step(&inputs);
        let mut row = Vec::new();
        for output in outputs {
            row.push(output.channels.to_string());
            for value in &output.values[..usize::from(output.channels)] {
                row.push(format!("{:08x}", value.to_bits()));
            }
        }
        writeln!(out, "{}", row.join(" ")).unwrap();
    }
}

fn run_unity(rate: u32, out: &mut impl Write) {
    let mut params = [0.0_f32; 3];
    let build = |params: &[f32; 3]| {
        node(
            "stock_unity",
            &[
                ("average1", params[0]),
                ("average2", params[1]),
                ("merge", params[2]),
            ],
        )
    };
    let mut unity = StockUnityProbe::new(&build(&params), rate).unwrap();
    for line in io::stdin().lock().lines() {
        let line = line.unwrap();
        let (op, rest) = line.split_once(' ').unwrap_or((line.as_str(), ""));
        match op {
            "F" => {
                let tokens: Vec<&str> = rest.split_whitespace().collect();
                assert_eq!(tokens.len(), 2 + 12 * 2);
                let next = [bits(tokens[0]), bits(tokens[1]), params[2]];
                if next != params {
                    params = next;
                    unity.edit(&build(&params), rate).unwrap();
                }
                let mut inputs = [PortFrame::default(); 12];
                for (index, input) in inputs.iter_mut().enumerate() {
                    input.channels = tokens[2 + index * 2].parse().unwrap();
                    input.values = [7.0; 16];
                    input.values[0] = bits(tokens[3 + index * 2]);
                }
                let outputs = unity.step(&inputs);
                let state = unity.snapshot();
                let lights = unity.lights();
                assert_eq!(lights.count, 10);
                let mut row: Vec<String> = outputs
                    .iter()
                    .map(|frame| {
                        assert_eq!(frame.channels, 1);
                        format!("{:08x}", frame.values[0].to_bits())
                    })
                    .collect();
                for (light, value) in lights.brightness[..10].iter().zip(state.lights) {
                    assert_eq!(light[0].to_bits(), value.to_bits());
                    row.push(format!("{:08x}", value.to_bits()));
                }
                for meter in state.meters {
                    row.push(format!("{:08x}", meter.to_bits()));
                }
                row.push(state.clock.to_string());
                row.push(u8::from(state.merge).to_string());
                writeln!(out, "{}", row.join(" ")).unwrap();
            }
            "M" => {
                params[2] = if rest.trim() == "0" { 0.0 } else { 1.0 };
                unity.edit(&build(&params), rate).unwrap();
            }
            "R" => {
                // Initialize: every parameter returns to its catalog default.
                params = [0.0; 3];
                unity.edit(&build(&params), rate).unwrap();
                unity.initialize();
                writeln!(
                    out,
                    "R {:08x} {:08x} {}",
                    params[0].to_bits(),
                    params[1].to_bits(),
                    u8::from(unity.snapshot().merge)
                )
                .unwrap();
            }
            "D" => {
                let data: Value = serde_json::from_str(rest.trim()).unwrap();
                let merge = StockUnityProbe::merge_from_data(&data);
                writeln!(out, "D {} {}", u8::from(merge), json!({ "merge": merge })).unwrap();
            }
            other => panic!("unknown op {other}"),
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    assert_eq!(args.len(), 3, "usage: probe vca|unity RATE");
    let rate: u32 = args[2].parse().unwrap();
    let stdout = io::stdout();
    let mut out = BufWriter::new(stdout.lock());
    match args[1].as_str() {
        "vca" => run_vca(&mut out),
        "unity" => run_unity(rate, &mut out),
        _ => panic!("unknown mode"),
    }
}
