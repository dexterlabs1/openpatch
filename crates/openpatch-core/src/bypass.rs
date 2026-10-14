//! Explicit host bypass routes, resolved once when compiling a graph.
use serde_json::{json, Value};
pub(crate) fn routes(kind: &str) -> &'static [(&'static str, &'static str)] {
    match kind {
        "stereo_delay" | "stereo_reverb" | "sidechain_compressor" => {
            &[("left", "left"), ("right", "right")]
        }
        "scope" => &[("x", "x"), ("y", "y")],
        "svf_filter" => &[("signal", "low")],
        "mutes10" => &[
            ("in1", "out1"),
            ("in2", "out2"),
            ("in3", "out3"),
            ("in4", "out4"),
            ("in5", "out5"),
            ("in6", "out6"),
            ("in7", "out7"),
            ("in8", "out8"),
            ("in9", "out9"),
            ("in10", "out10"),
        ],
        "octave" | "quantizer" => &[("pitch", "out")],
        "gain" | "lowpass" | "ladder_vcf" | "analog_vca" | "attenuverter" | "rescale" => {
            &[("signal", "out")]
        }
        "crossfade" => &[("a", "out")],
        "fade" => &[("a", "out"), ("b", "complement")],
        _ => &[],
    }
}
pub(crate) fn annotate(module: &mut Value) {
    let kind = module["kind"].as_str().unwrap().to_owned();
    module["bypass"] = json!({"field":"bypassed","default":false,"routes":routes(&kind).iter().map(|(input,output)|json!({"input":input,"output":output})).collect::<Vec<_>>(),
        "output_policy":if kind=="scope" {"independent raw X/Y input frames preserving 0–16 channels"} else {"direct input frames without normaling, gain, clipping or mixing; empty routed input and all unrouted outputs produce mono zero"},
        "state_policy":if kind=="midi_cv" {"events are consumed and MIDI state/pulses continue processing in fixed scratch storage; published outputs are mono zero"} else {"DSP state freezes while bypassed; bypass transitions preserve DSP and incoming delayed frames but clear output/outgoing delayed frames to mono zero, including self-loops"},
        "module_actions":"bypassed sequencers reject actions; manual presses reject while matching releases remain accepted and clear held/pending controls immediately; transitions discard pending actions and release held manual controls while preserving DSP",
        "compatibility":"explicit OpenPatch host policy; generic utility/filter routes are chosen for existing OpenPatch kinds, not a claim of full Rack bypass parity"});
    if kind == "poly_viz" {
        module["bypass"]["output_policy"] =
            json!("input-only module has no outputs or bypass routes");
        module["bypass"]["state_policy"] =
            json!("lights, channel count, and divider freeze while bypassed");
    }
}
