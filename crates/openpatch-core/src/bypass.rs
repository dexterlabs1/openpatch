//! Explicit host bypass routes, resolved once when compiling a graph.
use serde_json::{json, Value};
pub(crate) fn routes(kind: &str) -> &'static [(&'static str, &'static str)] {
    match kind {
        "stereo_delay" | "stereo_reverb" | "sidechain_compressor" => {
            &[("left", "left"), ("right", "right")]
        }
        "modal_resonator" => &[("excite", "out")],
        "scope" => &[("x", "x"), ("y", "y")],
        "svf_filter" => &[("signal", "low")],
        "stock_vca" => &[("signal", "out")],
        "stock_vcf" => &[("signal", "lowpass"), ("signal", "highpass")],
        "stock_delay" => &[("signal", "mix"), ("signal", "wet")],
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
        "octave" | "stock_octave" | "quantizer" => &[("pitch", "out")],
        "stock_quantizer" => &[("pitch", "pitch")],
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
    if kind == "stock_quantizer" {
        module["bypass"]["output_policy"] = json!("actual connected context: raw active pitch slots, no offset/quantization; connected empty input gives mono+0, disconnected output stays0 channels; inactive storage is zero padded");
        module["bypass"]["state_policy"] =
            json!("skip quantization; retain last playing-note observation and durable bank");
        module["bypass"]["module_actions"] = json!(
            "no performance actions; durable bank edits remain normal revisioned state assignments"
        );
    }
    if kind == "stock_octave" {
        module["bypass"]["output_policy"] = json!("Module::processBypass copies each active raw pitch lane then calls SDK setChannels with raw pitch width, including zero; disconnected output retains raw storage");
        module["bypass"]["state_policy"] = json!("skip Octave process and freeze channel-zero lastOctave; canonical host output transition lifecycle still applies");
        module["bypass"]["compatibility"] = json!("Fundamental configBypass pitch-to-output route with Rack Module::processBypass raw port copy semantics");
    }
    if kind == "stock_vca_mix" {
        module["bypass"]["output_policy"] = json!("No configured bypass routes or output writes; connected outputs follow actual common Rack-style bypass-transition lifecycle, disconnected raw storage is retained");
        module["bypass"]["state_policy"] = json!("skip VCMixer processing; four meters, ClockDivider and four module lamps freeze while bypassed");
        module["bypass"]["compatibility"] = json!("Fundamental VCMixer has no configBypass route; common native host lifecycle handles physical output transitions");
    }
    if kind == "midi_gate" {
        module["bypass"]["state_policy"] = json!("ignores MIDI and freezes DSP; transitions clear cell gates/timers and armed Learn, retaining owned completion and velocity");
        module["bypass"]["module_actions"] = json!("midi_panic remains available while bypassed and clears cell gates/timers and armed Learn immediately");
    }
    if crate::content::is_annotation(&kind) {
        module["bypass"]["output_policy"] = json!("no outputs or bypass routes");
        module["bypass"]["state_policy"] = json!("no DSP state");
    }
    if kind == "poly_viz" {
        module["bypass"]["output_policy"] =
            json!("input-only module has no outputs or bypass routes");
        module["bypass"]["state_policy"] =
            json!("lights, channel count, and divider freeze while bypassed");
    }
}
