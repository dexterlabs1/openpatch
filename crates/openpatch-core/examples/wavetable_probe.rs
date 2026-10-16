// SPDX-License-Identifier: MIT
//! Binary OWT1 adapter for the actual-class wavetable qualification runner.
//!
//! Build this example separately, then pass its executable to
//! `scripts/wavetable_reference/run.py --sdk-only --rust-probe <executable>`.
//! The adapter uses production wavetable preparation and oscillators through a
//! doc-hidden control-thread probe. It contains no fixture-specific DSP.

use std::io::{self, BufReader, BufWriter, Read, Write};

use openpatch_core::{wavetable_probe::Oracle, PortFrame, MAX_CHANNELS};

const MAGIC: u32 = 0x3154_574f;
const MAX_BANK_SAMPLES: u32 = 1 << 20;
const MAX_WAVE_LENGTH: u32 = 1 << 14;

fn read_u32(input: &mut impl Read) -> Result<u32, String> {
    let mut bytes = [0; 4];
    input
        .read_exact(&mut bytes)
        .map_err(|error| format!("reading input: {error}"))?;
    Ok(u32::from_le_bytes(bytes))
}

fn read_f32(input: &mut impl Read) -> Result<f32, String> {
    Ok(f32::from_bits(read_u32(input)?))
}

fn read_finite(input: &mut impl Read, description: &str) -> Result<f32, String> {
    let value = read_f32(input)?;
    if value.is_finite() {
        Ok(value)
    } else {
        Err(format!("nonfinite {description}"))
    }
}

fn read_bank(input: &mut impl Read, length: u32, count: u32) -> Result<Vec<f32>, String> {
    if count > MAX_BANK_SAMPLES || length > MAX_WAVE_LENGTH {
        return Err("bank exceeds fixture limits".into());
    }
    let mut samples = Vec::with_capacity(count as usize);
    for _ in 0..count {
        samples.push(read_finite(input, "bank sample")?);
    }
    Ok(samples)
}

fn write_u32(output: &mut impl Write, value: u32) -> Result<(), String> {
    output
        .write_all(&value.to_le_bytes())
        .map_err(|error| format!("writing output: {error}"))
}

fn write_f32(output: &mut impl Write, value: f32) -> Result<(), String> {
    write_u32(output, value.to_bits())
}

fn run() -> Result<(), String> {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut input = BufReader::new(stdin.lock());
    let mut output = BufWriter::new(stdout.lock());
    if read_u32(&mut input)? != MAGIC {
        return Err("bad OWT1 magic".into());
    }
    let kind = read_u32(&mut input)?;
    let sample_rate = read_finite(&mut input, "sample rate")?;
    let frames = read_u32(&mut input)?;
    let flags = read_u32(&mut input)?;
    let wave_length = read_u32(&mut input)?;
    let sample_count = read_u32(&mut input)?;
    let default_bank = read_u32(&mut input)?;
    if kind > 1 || flags > 1 || default_bank > 1 || sample_rate <= 0.0 {
        return Err("invalid header".into());
    }
    // Production engines use integral sample rates. All qualification fixture
    // rates are integral; rejecting an unsupported header avoids silent casts.
    if sample_rate.fract() != 0.0 || f64::from(sample_rate) > f64::from(u32::MAX) {
        return Err("sample rate must fit a positive integral production rate".into());
    }
    if default_bank != 0 && sample_count != 0 {
        return Err("default bank has sample bytes".into());
    }
    let mut oracle = Oracle::new(kind, sample_rate as u32)?;
    if default_bank == 0 {
        let samples = read_bank(&mut input, wave_length, sample_count)?;
        oracle.replace_bank(wave_length, samples)?;
    }

    for _ in 0..frames {
        match read_u32(&mut input)? {
            0 => (),
            1 => oracle.reset_source()?,
            2 => {
                let length = read_u32(&mut input)?;
                let count = read_u32(&mut input)?;
                let samples = read_bank(&mut input, length, count)?;
                oracle.replace_bank(length, samples)?;
            }
            3 => oracle.set_loading(true),
            4 => oracle.set_loading(false),
            5 => oracle.replace_bank(0, Vec::new())?,
            6 => oracle.reset_bank()?,
            7 => oracle.set_wave_length(read_u32(&mut input)?)?,
            8 => oracle.adopt_current_state()?,
            _ => return Err("unknown event".into()),
        }

        let parameter_count = if kind == 0 { 7 } else { 6 };
        let mut parameters = [0.0; 7];
        for value in &mut parameters[..parameter_count] {
            *value = read_finite(&mut input, "parameter")?;
        }
        oracle.set_source_parameters(&parameters[..parameter_count])?;

        let mut inputs = [PortFrame::default(); 4];
        for port in &mut inputs {
            let width = read_u32(&mut input)?;
            if width > MAX_CHANNELS as u32 {
                return Err("invalid input width".into());
            }
            port.channels = width as u8;
            for lane in 0..MAX_CHANNELS {
                let value = read_finite(&mut input, "port voltage")?;
                // Read every storage lane, but never deliver the fixture's
                // poison values outside the declared Rack port width.
                port.values[lane] = if lane < width as usize { value } else { 0.0 };
            }
        }

        let frame = oracle.process(&inputs);
        if !(1..=MAX_CHANNELS as u8).contains(&frame.channels) {
            return Err("production oscillator returned an invalid output width".into());
        }
        write_u32(&mut output, u32::from(frame.channels))?;
        for lane in 0..MAX_CHANNELS {
            write_f32(
                &mut output,
                if lane < usize::from(frame.channels) {
                    frame.values[lane]
                } else {
                    0.0
                },
            )?;
        }
        if flags & 1 == 0 {
            continue;
        }

        let state = oracle.observation();
        for value in state.phases {
            write_f32(&mut output, value)?;
        }
        for value in state.directions {
            write_f32(&mut output, if kind == 0 { value } else { 0.0 })?;
        }
        for value in state.last_sync_values {
            write_f32(&mut output, if kind == 0 { value } else { 0.0 })?;
        }
        write_f32(
            &mut output,
            if kind == 1 {
                state.clock_frequency
            } else {
                0.0
            },
        )?;
        write_f32(&mut output, if kind == 1 { state.timer } else { 0.0 })?;
        write_f32(&mut output, state.last_position)?;
        for mask in state.reset_masks {
            write_u32(&mut output, if kind == 1 { mask } else { 0 })?;
        }
        write_u32(&mut output, if kind == 1 { state.clock_trigger } else { 0 })?;
        write_u32(&mut output, u32::from(state.loading))?;
        write_u32(&mut output, state.wave_length)?;
        write_u32(&mut output, state.sample_count)?;
    }

    let mut trailing = [0; 1];
    if input
        .read(&mut trailing)
        .map_err(|error| format!("reading stream end: {error}"))?
        != 0
    {
        return Err("trailing input bytes".into());
    }
    output
        .flush()
        .map_err(|error| format!("flushing output: {error}"))
}

fn main() {
    if let Err(error) = run() {
        eprintln!("wavetable Rust oracle: {error}");
        std::process::exit(2);
    }
}
