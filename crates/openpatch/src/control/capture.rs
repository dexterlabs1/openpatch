//! Bounded offline waveform capture. Aggregate every active sample rather than
//! subsampling, so an isolated pulse remains visible in a bin's extrema.
use super::{optional_string, string, PerformanceEvents, Session};
use openpatch_core::{catalog, PortFrame, BLOCK_SIZE};
use serde_json::{json, Value};

#[derive(Clone, Copy, Debug)]
struct Statistics {
    count: usize,
    sum: f64,
    squares: f64,
    min: f32,
    max: f32,
    peak: f32,
}
impl Default for Statistics {
    fn default() -> Self {
        Self {
            count: 0,
            sum: 0.0,
            squares: 0.0,
            min: f32::INFINITY,
            max: f32::NEG_INFINITY,
            peak: 0.0,
        }
    }
}
impl Statistics {
    fn add(&mut self, value: f32) {
        self.count += 1;
        self.sum += f64::from(value);
        self.squares += f64::from(value).powi(2);
        self.min = self.min.min(value);
        self.max = self.max.max(value);
        self.peak = self.peak.max(value.abs());
    }
    fn mean(&self) -> Option<f64> {
        (self.count > 0).then(|| self.sum / self.count as f64)
    }
    fn min(&self) -> Option<f32> {
        (self.count > 0).then_some(self.min)
    }
    fn max(&self) -> Option<f32> {
        (self.count > 0).then_some(self.max)
    }
    fn summary(&self) -> Value {
        json!({"active_frames":self.count,"min":self.min(),"max":self.max(),"dc":self.mean(),
            "rms":(self.count>0).then(|| (self.squares/self.count as f64).sqrt()),
            "peak":(self.count>0).then_some(self.peak)})
    }
}
#[derive(Debug)]
struct Bin {
    start: usize,
    end: usize,
    statistics: Statistics,
}
fn integer(args: &Value, name: &str, default: u64, min: u64, max: u64) -> Result<usize, String> {
    let value = match args.get(name) {
        None => default,
        Some(value) => value
            .as_u64()
            .ok_or_else(|| format!("{name} must be an integer"))?,
    };
    if !(min..=max).contains(&value) {
        return Err(format!("{name} must be in {min}..={max}"));
    }
    Ok(value as usize)
}
impl Session {
    pub(super) fn capture_signal(&self, args: &Value) -> Result<Value, String> {
        let seconds = args
            .get("seconds")
            .and_then(Value::as_f64)
            .ok_or("seconds must be a number")?;
        if !seconds.is_finite() || !(0.001..=2.0).contains(&seconds) {
            return Err("capture seconds must be finite and in 0.001..=2".into());
        }
        let requested_points = integer(args, "points", 512, 64, 2048)?;
        let channel = integer(args, "channel", 1, 1, 16)?;
        let expected = args
            .get("expected_revision")
            .map(|value| {
                value
                    .as_u64()
                    .ok_or("expected_revision must be a nonnegative integer")
            })
            .transpose()?;
        self.check_revision(expected)?;
        let id = string(args, "id")?;
        let port = optional_string(args, "port", "out")?;
        let mut patch = self.patch.clone();
        patch.output = Some(id.into());
        patch.output_port = port.into();
        let mut engine = patch.compile()?;
        let frames = (seconds * f64::from(patch.sample_rate)).round() as usize;
        let mut events = PerformanceEvents::new(args, frames, &engine)?;
        let node = patch
            .nodes
            .iter()
            .find(|node| node.id == id)
            .ok_or("capture module does not exist")?;
        let catalog = catalog();
        let units = catalog["modules"]
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| entry["kind"] == node.kind)
            .and_then(|entry| entry["output_metadata"].as_array())
            .and_then(|ports| ports.iter().find(|entry| entry["name"] == port))
            .and_then(|entry| entry["units"].as_str())
            .ok_or("capture output has no catalog units")?
            .to_owned();
        let domain = match units.as_str() {
            "V" => "voltage",
            "normalized digital amplitude" => "normalized_audio",
            _ => "legacy_normalized",
        };
        let points = requested_points.min(frames);
        let mut bins: Vec<_> = (0..points)
            .map(|index| Bin {
                start: index * frames / points,
                end: (index + 1) * frames / points,
                statistics: Statistics::default(),
            })
            .collect();
        let mut summary = Statistics::default();
        let mut block = [PortFrame::default(); BLOCK_SIZE];
        let mut cursor = 0;
        let mut bin_index = 0;
        #[cfg(test)]
        {
            let hook = BEFORE_RENDER.with(|slot| slot.borrow_mut().take());
            if let Some(hook) = hook {
                hook();
            }
        }
        while cursor < frames {
            events.apply(&mut engine, cursor);
            let end = events.next_frame(frames);
            let count = (frames - cursor).min(BLOCK_SIZE).min(end - cursor);
            engine.process_poly_block(&mut block[..count]);
            for (offset, frame) in block[..count].iter().enumerate() {
                let absolute = cursor + offset;
                while absolute >= bins[bin_index].end {
                    bin_index += 1;
                }
                if channel <= usize::from(frame.channels) {
                    let value = frame.values[channel - 1];
                    if !value.is_finite() {
                        return Err("capture encountered a nonfinite signal".into());
                    }
                    bins[bin_index].statistics.add(value);
                    summary.add(value);
                }
            }
            cursor += count;
        }
        let bins: Vec<_> = bins.iter().map(|bin| json!({"start_frame":bin.start,"end_frame":bin.end,"active_frames":bin.statistics.count,"min":bin.statistics.min(),"max":bin.statistics.max(),"mean":bin.statistics.mean()})).collect();
        Ok(
            json!({"revision":self.revision,"id":id,"port":port,"channel":channel,"sample_rate":patch.sample_rate,"frames":frames,
            "requested_points":requested_points,"points":points,"units":units,"amplitude_domain":domain,
            "digital_full_scale":(domain=="normalized_audio").then_some(1.0),
            "state":"captured offline from reset","midi_event_count":events.midi.len(),"module_event_count":events.modules.len(),"solver_failures":engine.solver_failures(),
            "statistics":"active channel samples only; inactive channels excluded, not treated as zero",
            "amplitude_policy":"raw selected channel; no summing, normalization, or WAV clipping",
            "summary":summary.summary(),"bins":bins}),
        )
    }
}
#[cfg(test)]
thread_local! {
    static BEFORE_RENDER: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::control::{execute_shared, Session};
    use openpatch_core::{Node, Patch};
    fn node(id: &str, kind: &str, params: &[(&str, f32)]) -> Node {
        Node {
            score: None,
            bypassed: false,
            id: id.into(),
            kind: kind.into(),
            params: params
                .iter()
                .map(|(key, value)| ((*key).into(), *value))
                .collect(),
        }
    }
    fn session(nodes: Vec<Node>, cables: Vec<openpatch_core::Cable>) -> Session {
        let mut session = Session::new(&std::env::temp_dir()).unwrap();
        session
            .replace_patch(
                Patch {
                    nodes,
                    cables,
                    ..Patch::new_v2()
                },
                Some(0),
            )
            .unwrap();
        session
    }
    fn wire(from: &str, output: &str, to: &str, input: &str) -> openpatch_core::Cable {
        openpatch_core::Cable {
            input_order: None,
            output_order: None,
            color: None,
            id: None,
            from: from.into(),
            output: output.into(),
            to: to.into(),
            input: input.into(),
        }
    }
    fn capture(session: &mut Session, args: Value) -> Value {
        session.execute("capture_signal", args).unwrap()
    }
    #[test]
    fn calibrated_constant_and_nonempty_bins_cover_short_render_exactly() {
        let mut s = session(
            vec![node(
                "dc",
                "poly_constant",
                &[("channels", 1.0), ("v1", -2.0)],
            )],
            vec![],
        );
        let before = s.snapshot();
        let result = capture(&mut s, json!({"id":"dc","seconds":0.001}));
        assert_eq!(result["revision"], 1);
        assert_eq!(result["sample_rate"], 48000);
        assert_eq!(result["frames"], 48);
        assert_eq!(result["requested_points"], 512);
        assert_eq!(result["points"], 48);
        assert_eq!(result["channel"], 1);
        assert_eq!(result["units"], "V");
        assert_eq!(result["amplitude_domain"], "voltage");
        assert!(result["digital_full_scale"].is_null());
        assert_eq!(
            result["summary"],
            json!({"active_frames":48,"min":-2.0,"max":-2.0,"dc":-2.0,"rms":2.0,"peak":2.0})
        );
        for (index, bin) in result["bins"].as_array().unwrap().iter().enumerate() {
            assert_eq!(
                *bin,
                json!({"start_frame":index,"end_frame":index+1,"active_frames":1,"min":-2.0,"max":-2.0,"mean":-2.0})
            );
        }
        assert_eq!(s.snapshot(), before);
    }
    #[test]
    fn one_sample_midi_pulse_preserves_peak_and_exact_statistics_between_bin_boundaries() {
        let mut s = session(vec![node("m", "midi_cv", &[("channels", 1.0)])], vec![]);
        let args = json!({"id":"m","port":"gate","seconds":0.01,"points":64,"midi_events":[
            {"frame":101,"event":{"type":"note_on","channel":0,"note":60,"velocity":127}},
            {"frame":102,"event":{"type":"note_off","channel":0,"note":60,"velocity":0}}
        ]});
        let before = s.snapshot();
        let result = capture(&mut s, args.clone());
        assert_eq!(result["midi_event_count"], 2);
        assert_eq!(result["summary"]["active_frames"], 480);
        assert_eq!(result["summary"]["peak"], 10.0);
        assert_eq!(result["summary"]["min"], 0.0);
        assert_eq!(result["summary"]["dc"], 10.0 / 480.0);
        assert_eq!(result["summary"]["rms"], (100.0_f64 / 480.0).sqrt());
        let bins = result["bins"].as_array().unwrap();
        let pulses: Vec<_> = bins.iter().filter(|bin| bin["max"] == 10.0).collect();
        assert_eq!(pulses.len(), 1);
        let pulse = pulses[0];
        assert!(
            pulse["start_frame"].as_u64().unwrap() <= 101
                && pulse["end_frame"].as_u64().unwrap() > 101
        );
        assert_eq!(pulse["min"], 0.0);
        assert_eq!(
            pulse["mean"],
            10.0 / pulse["active_frames"].as_f64().unwrap()
        );
        assert_eq!(bins.first().unwrap()["start_frame"], 0);
        assert_eq!(bins.last().unwrap()["end_frame"], 480);
        for pair in bins.windows(2) {
            assert_eq!(pair[0]["end_frame"], pair[1]["start_frame"]);
        }
        assert_eq!(capture(&mut s, args), result);
        assert_eq!(s.snapshot(), before);
    }
    #[test]
    fn absent_channels_are_null_and_delayed_named_poly_outputs_exclude_inactive_samples() {
        let mut s = session(
            vec![
                node("empty", "poly_constant", &[("channels", 0.0)]),
                node("bus", "poly_constant", &[("channels", 2.0), ("v2", -4.0)]),
                node("copy", "mult", &[]),
            ],
            vec![wire("bus", "out", "copy", "signal")],
        );
        for (id, channel) in [("empty", 1), ("bus", 3), ("bus", 16)] {
            let result = capture(&mut s, json!({"id":id,"seconds":0.001,"channel":channel}));
            assert_eq!(
                result["summary"],
                json!({"active_frames":0,"min":null,"max":null,"dc":null,"rms":null,"peak":null})
            );
            assert!(result["bins"]
                .as_array()
                .unwrap()
                .iter()
                .all(|bin| bin["active_frames"] == 0
                    && bin["min"].is_null()
                    && bin["max"].is_null()
                    && bin["mean"].is_null()));
        }
        let result = capture(
            &mut s,
            json!({"id":"copy","port":"out8","channel":2,"seconds":0.001}),
        );
        assert_eq!(
            result["summary"],
            json!({"active_frames":47,"min":-4.0,"max":-4.0,"dc":-4.0,"rms":4.0,"peak":4.0})
        );
        assert_eq!(
            result["bins"][0],
            json!({"start_frame":0,"end_frame":1,"active_frames":0,"min":null,"max":null,"mean":null})
        );
    }
    #[test]
    fn digital_audio_and_legacy_units_are_distinct_and_capture_does_not_clip_or_sum() {
        let mut s = session(
            vec![
                node(
                    "v",
                    "poly_constant",
                    &[("channels", 2.0), ("v1", 16.0), ("v2", -4.0)],
                ),
                node("legacy", "constant", &[("value", 2.0)]),
                node("audio", "audio_stereo", &[("gain", 2.0)]),
            ],
            vec![wire("v", "out", "audio", "left")],
        );
        let voltage = capture(&mut s, json!({"id":"v","channel":2,"seconds":0.001}));
        assert_eq!(voltage["summary"]["dc"], -4.0);
        let legacy = capture(&mut s, json!({"id":"legacy","seconds":0.001}));
        assert_eq!(legacy["units"], "legacy normalized");
        assert_eq!(legacy["amplitude_domain"], "legacy_normalized");
        assert!(legacy["digital_full_scale"].is_null());
        let audio = capture(&mut s, json!({"id":"audio","channel":2,"seconds":0.001}));
        assert_eq!(audio["units"], "normalized digital amplitude");
        assert_eq!(audio["amplitude_domain"], "normalized_audio");
        assert_eq!(audio["digital_full_scale"], 1.0);
        assert!((audio["summary"]["peak"].as_f64().unwrap() - 2.4).abs() < 1e-6);
        // Explicit sink sums 12 V × gain2 / 10; capture retains >1 amplitude.
    }
    #[test]
    fn requests_validate_types_limits_endpoints_events_and_stale_revision_without_changes() {
        let mut s = session(vec![node("dc", "poly_constant", &[])], vec![]);
        let before = s.snapshot();
        for (field, value) in [
            ("seconds", json!(0.0)),
            ("seconds", json!(2.001)),
            ("seconds", json!("1")),
            ("points", json!(63)),
            ("points", json!(2049)),
            ("points", json!(64.0)),
            ("points", json!(true)),
            ("channel", json!(0)),
            ("channel", json!(17)),
            ("channel", json!(1.5)),
            ("channel", json!(null)),
            ("id", json!("missing")),
            ("id", json!(1)),
            ("port", json!("missing")),
            ("port", json!(1)),
            ("expected_revision", json!(0)),
            ("expected_revision", json!(-1)),
            ("unexpected", json!(1)),
            (
                "midi_events",
                json!([{"frame":48,"event":{"type":"clock"}}]),
            ),
            (
                "midi_events",
                json!([{"frame":1,"event":{"type":"clock"}},{"frame":0,"event":{"type":"stop"}}]),
            ),
            (
                "midi_events",
                json!([{"frame":0,"event":{"type":"clock","extra":1}}]),
            ),
        ] {
            let mut args = json!({"id":"dc","seconds":0.001});
            args[field] = value;
            assert!(
                s.execute("capture_signal", args).is_err(),
                "accepted {field}"
            );
            assert_eq!(s.snapshot(), before);
        }
        s.seal_for_close(s.revision()).unwrap();
        assert!(s
            .execute(
                "capture_signal",
                json!({"id":"dc","seconds":0.001,"expected_revision":1})
            )
            .is_ok());
    }
    #[test]
    fn capture_releases_live_session_lock_and_keeps_the_captured_revision() {
        let s = session(
            vec![node(
                "dc",
                "poly_constant",
                &[("channels", 1.0), ("v1", 1.0)],
            )],
            vec![],
        );
        let shared = s.into_shared();
        let edit_session = shared.clone();
        BEFORE_RENDER.with(|slot| {
            *slot.borrow_mut() = Some(Box::new(move || {
                let mut live = edit_session
                    .try_lock()
                    .expect("offline capture held the live session lock");
                live.execute(
                    "set_parameter",
                    json!({"id":"dc","parameter":"v1","value":2.0,"expected_revision":1}),
                )
                .unwrap();
            }))
        });
        let result = execute_shared(
            &shared,
            "capture_signal",
            json!({"id":"dc","seconds":0.001,"expected_revision":1}),
        )
        .unwrap();
        assert_eq!(result["revision"], 1);
        assert_eq!(result["summary"]["dc"], 1.0);
        assert_eq!(shared.lock().unwrap().revision(), 2);
        let new = execute_shared(
            &shared,
            "capture_signal",
            json!({"id":"dc","seconds":0.001,"expected_revision":2}),
        )
        .unwrap();
        assert_eq!(new["summary"]["dc"], 2.0);
        assert!(execute_shared(
            &shared,
            "capture_signal",
            json!({"id":"dc","seconds":0.001,"expected_revision":1})
        )
        .is_err());
    }
    #[test]
    fn maximum_capture_is_bounded_and_summary_is_independent_of_bin_count() {
        let mut patch = Patch::new_v2();
        patch.sample_rate = 192000;
        patch.nodes = vec![node(
            "dc",
            "poly_constant",
            &[("channels", 1.0), ("v1", 3.0)],
        )];
        let mut s = Session::new(&std::env::temp_dir()).unwrap();
        s.replace_patch(patch, Some(0)).unwrap();
        let high = capture(&mut s, json!({"id":"dc","seconds":2,"points":2048}));
        let low = capture(&mut s, json!({"id":"dc","seconds":2,"points":64}));
        assert_eq!(high["frames"], 384000);
        assert_eq!(high["points"], 2048);
        assert_eq!(high["summary"], low["summary"]);
        assert_eq!(high["bins"].as_array().unwrap().len(), 2048);
        assert!(high.to_string().len() < 524288);
        assert!(high["bins"]
            .as_array()
            .unwrap()
            .iter()
            .all(|bin| bin["end_frame"].as_u64() > bin["start_frame"].as_u64()));
    }
    #[test]
    fn same_frame_midi_order_and_impulses_on_bin_boundaries_are_preserved() {
        let mut s = session(vec![node("m", "midi_cv", &[("channels", 1.0)])], vec![]);
        let note_on =
            json!({"frame":97,"event":{"type":"note_on","channel":0,"note":60,"velocity":127}});
        let note_off =
            json!({"frame":97,"event":{"type":"note_off","channel":0,"note":60,"velocity":0}});
        let released = capture(
            &mut s,
            json!({"id":"m","port":"gate","seconds":0.01,"points":64,"midi_events":[note_on,note_off]}),
        );
        assert_eq!(released["summary"]["peak"], 0.0);
        let impulse = capture(
            &mut s,
            json!({"id":"m","port":"gate","seconds":0.01,"points":64,"midi_events":[note_off,note_on,{"frame":98,"event":{"type":"note_off","channel":0,"note":60,"velocity":0}}]}),
        );
        let bin = impulse["bins"]
            .as_array()
            .unwrap()
            .iter()
            .find(|bin| bin["start_frame"] == 97)
            .unwrap();
        assert_eq!(bin["max"], 10.0);
        assert_eq!(bin["min"], 0.0);
        assert_eq!(impulse["summary"]["dc"], 10.0 / 480.0);
    }
}
