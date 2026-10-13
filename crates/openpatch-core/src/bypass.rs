//! Explicit host bypass routes, resolved once when compiling a graph.
use serde_json::{json, Value};
pub(crate) fn routes(kind: &str) -> &'static [(&'static str, &'static str)] {
    match kind {
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
        "gain" | "lowpass" | "ladder_vcf" | "analog_vca" | "attenuverter" => &[("signal", "out")],
        "crossfade" => &[("a", "out")],
        _ => &[],
    }
}
pub(crate) fn annotate(module: &mut Value) {
    let kind = module["kind"].as_str().unwrap().to_owned();
    module["bypass"] = json!({"field":"bypassed","default":false,"routes":routes(&kind).iter().map(|(input,output)|json!({"input":input,"output":output})).collect::<Vec<_>>(),
        "output_policy":"direct input frames without normaling, gain, clipping or mixing; empty routed input and all unrouted outputs produce mono zero",
        "state_policy":if kind=="midi_cv" {"events are consumed and MIDI state/pulses continue processing in fixed scratch storage; published outputs are mono zero"} else {"DSP state freezes while bypassed; bypass transitions preserve DSP and incoming delayed frames but clear output/outgoing delayed frames to mono zero, including self-loops"},
        "module_actions":"bypassed sequencers reject action resolution/application; transitions discard internal queued sequencer actions",
        "compatibility":"explicit OpenPatch host policy; generic utility/filter routes are chosen for existing OpenPatch kinds, not a claim of full Rack bypass parity"});
}
