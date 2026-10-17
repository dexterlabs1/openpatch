//! Standalone probe of the actual production kernel (no duplicate DSP).
#[path = "../../crates/openpatch-core/src/frame_scene/kernel.rs"]
mod kernel;
use kernel::{ChannelSettings, EasingCurve, FrameKernel};
use std::io::{self, Read, Write};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut input = String::new();
    io::stdin().read_to_string(&mut input)?;
    let mut words = input.split_whitespace();
    macro_rules! read {
        ($ty:ty) => {
            words.next().ok_or("missing argument")?.parse::<$ty>()?
        };
    }
    let mut k = FrameKernel::new();
    let mut record = 0;
    let mut out = io::BufWriter::new(io::stdout().lock());
    while let Some(op) = words.next() {
        match op {
            "S" => {
                let ch = read!(usize);
                let curve = read!(u8);
                let response = read!(u8);
                k.set_settings(
                    ch,
                    ChannelSettings {
                        easing_curve: EasingCurve::from_index(curve),
                        response,
                    },
                );
                continue;
            }
            "I" => {
                let ch = read!(usize);
                let value = read!(u16);
                k.set_immediate(ch, value);
                continue;
            }
            "C" => {
                k.clear();
                continue;
            }
            "A" => {
                let ts = read!(u16);
                let values = [read!(u16), read!(u16), read!(u16), read!(u16)];
                let success = k.add_keyframe(ts, values);
                writeln!(
                    out,
                    "{{\"record\":{record},\"op\":\"A\",\"success\":{},\"keys\":{}}}",
                    u8::from(success),
                    k.num_keyframes()
                )?;
            }
            "R" => {
                let ts = read!(u16);
                let success = k.remove_keyframe(ts);
                writeln!(
                    out,
                    "{{\"record\":{record},\"op\":\"R\",\"success\":{},\"keys\":{}}}",
                    u8::from(success),
                    k.num_keyframes()
                )?;
            }
            "E" => {
                let ts = read!(u16);
                let tolerance = read!(u16);
                let result = k.evaluate(ts);
                let near = k.find_nearest_keyframe(ts, tolerance);
                writeln!(out,"{{\"record\":{record},\"op\":\"E\",\"timestamp\":{ts},\"levels\":{:?},\"dac_code\":{:?},\"color\":{:?},\"position\":{},\"nearest_keyframe\":{},\"find_nearest\":{near}}}",result.levels,result.dac_code,result.color,result.position,result.nearest_keyframe)?;
            }
            "D" => {
                let gain = read!(u16);
                let response = read!(u8);
                writeln!(out,"{{\"record\":{record},\"op\":\"D\",\"gain\":{gain},\"response\":{response},\"dac_code\":{}}}",FrameKernel::convert_to_dac_code(gain,response))?;
            }
            "V" => {
                let ch = read!(usize);
                let tick = read!(u16);
                let easing = read!(u8) != 0;
                writeln!(
                    out,
                    "{{\"record\":{record},\"op\":\"V\",\"value\":{}}}",
                    k.sample_animation(ch, tick, easing)
                )?;
            }
            _ => return Err("unknown operation".into()),
        }
        record += 1;
    }
    Ok(())
}
