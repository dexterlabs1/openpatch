//! Bounded wavetable file codecs for control-thread use. These functions allocate;
//! do not call them from an audio callback. No filesystem access is performed.
//!
//! WAV channels stay interleaved, as in the source wavetable loader. Integer
//! normalization was checked against Rack's `dsp/convert.hpp` (raw files) and
//! Fundamental's dr_wav 0.13.16 at commit
//! `10dd0160c664770910e5584b7b00498cc48d9ddd` (WAV files).
//! This is an original implementation;
//! no third-party codec source is incorporated.

/// Maximum number of scalar samples, including every channel of a WAV file.
pub const MAX_WAVETABLE_SAMPLES: usize = 524_288;
/// Limits metadata as well as sample data, before parsing or allocating samples.
const MAX_SOURCE_BYTES: usize = 16 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq)]
pub struct DecodedWavetable {
    pub samples: Vec<f32>,
    /// Raw files have no sample rate.
    pub source_sample_rate: Option<u32>,
    /// Raw files are treated as mono. WAV samples retain channel interleaving.
    pub source_channels: u16,
    /// A nonzero power-of-two WAV sample rate, matching Rack's length hint.
    /// This is a hint, not validation that an entire wave fits the sample data.
    pub suggested_wave_length: Option<usize>,
}

#[derive(Clone, Copy)]
enum SampleFormat {
    Unsigned8,
    Signed8,
    Signed16,
    Signed24,
    Signed32,
    Float32,
}

impl SampleFormat {
    fn width(self) -> usize {
        match self {
            Self::Unsigned8 | Self::Signed8 => 1,
            Self::Signed16 => 2,
            Self::Signed24 => 3,
            Self::Signed32 | Self::Float32 => 4,
        }
    }
}

/// Decode RIFF/WAVE PCM8/16/24/32 or float32, or little-endian raw signed
/// `.s8/.i8/.s16/.i16/.s24/.i24/.s32/.i32` and `.f32` samples.
/// `extension` is just the extension, with an optional leading dot; case is ignored.
///
/// Rejects empty, partial, oversized, nonfinite, unsupported, and malformed input
/// without changing any caller-owned table. WAV requires exact RIFF and data
/// lengths, a consistent byte rate/block alignment, and one format/data chunk.
/// Extensible WAV supports PCM/float GUIDs when valid bits equal container bits.
/// AIFF, RF64, big-endian RIFX, compressed formats, and packed valid-bit formats
/// are unsupported. Raw samples have no rate/length hint.
pub fn decode_wavetable(bytes: &[u8], extension: &str) -> Result<DecodedWavetable, String> {
    if bytes.len() > MAX_SOURCE_BYTES {
        return Err("Wavetable source exceeds 16 MiB".into());
    }
    let extension = extension.strip_prefix('.').unwrap_or(extension);
    // Avoid allocating proportional to an untrusted extension string.
    if extension.len() > 3 {
        return Err("Unsupported wavetable file extension".into());
    }
    let extension = extension.to_ascii_lowercase();
    if extension == "wav" {
        return decode_wav(bytes);
    }
    let format = match extension.as_str() {
        "s8" | "i8" => SampleFormat::Signed8,
        "s16" | "i16" => SampleFormat::Signed16,
        "s24" | "i24" => SampleFormat::Signed24,
        "s32" | "i32" => SampleFormat::Signed32,
        "f32" => SampleFormat::Float32,
        _ => return Err("Unsupported wavetable file extension".into()),
    };
    Ok(DecodedWavetable {
        samples: decode_samples(bytes, format)?,
        source_sample_rate: None,
        source_channels: 1,
        suggested_wave_length: None,
    })
}

fn decode_samples(bytes: &[u8], format: SampleFormat) -> Result<Vec<f32>, String> {
    let width = format.width();
    if !bytes.len().is_multiple_of(width) {
        return Err("Partial wavetable sample".into());
    }
    validate_sample_count(bytes.len() / width)?;
    let mut samples = Vec::with_capacity(bytes.len() / width);
    for sample in bytes.chunks_exact(width) {
        let value = match format {
            // dr_wav's default unsigned WAV8 conversion spans both endpoints;
            // signed raw i8 instead uses the usual power-of-two normalization.
            SampleFormat::Unsigned8 => f32::from(sample[0]) * (2.0_f32 / 255.0) - 1.0,
            SampleFormat::Signed8 => (sample[0] as i8) as f32 / 128.0,
            SampleFormat::Signed16 => i16::from_le_bytes([sample[0], sample[1]]) as f32 / 32768.0,
            SampleFormat::Signed24 => {
                let signed = i32::from_le_bytes([0, sample[0], sample[1], sample[2]]) >> 8;
                signed as f32 / 8_388_608.0
            }
            SampleFormat::Signed32 => {
                i32::from_le_bytes([sample[0], sample[1], sample[2], sample[3]]) as f32
                    / 2_147_483_648.0
            }
            SampleFormat::Float32 => {
                f32::from_le_bytes([sample[0], sample[1], sample[2], sample[3]])
            }
        };
        if !value.is_finite() {
            return Err("Wavetable contains a nonfinite sample".into());
        }
        samples.push(value);
    }
    Ok(samples)
}

fn validate_sample_count(count: usize) -> Result<(), String> {
    if count == 0 || count > MAX_WAVETABLE_SAMPLES {
        return Err(format!(
            "Wavetable must contain 1..={MAX_WAVETABLE_SAMPLES} samples"
        ));
    }
    Ok(())
}

fn u16_at(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([bytes[offset], bytes[offset + 1]])
}

fn u32_at(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ])
}

struct WaveFormat {
    samples: SampleFormat,
    channels: u16,
    rate: u32,
    block_align: usize,
}

fn parse_wave_format(bytes: &[u8]) -> Result<WaveFormat, String> {
    if bytes.len() < 16 {
        return Err("Truncated WAV format".into());
    }
    let mut tag = u16_at(bytes, 0);
    let channels = u16_at(bytes, 2);
    let rate = u32_at(bytes, 4);
    let byte_rate = u32_at(bytes, 8);
    let block_align = u16_at(bytes, 12);
    let bits = u16_at(bytes, 14);
    if channels == 0 || rate == 0 {
        return Err("WAV channel count and sample rate must be nonzero".into());
    }
    if bytes.len() != 16 {
        if bytes.len() < 18 || usize::from(u16_at(bytes, 16)) != bytes.len() - 18 {
            return Err("Inconsistent WAV format extension length".into());
        }
        if tag != 0xfffe && bytes.len() != 18 {
            return Err("Unsupported WAV format extension".into());
        }
    }
    if tag == 0xfffe {
        if bytes.len() != 40 || u16_at(bytes, 18) != bits {
            return Err("Unsupported extensible WAV valid-bit format".into());
        }
        let mask = u32_at(bytes, 20);
        if mask != 0 && mask.count_ones() != u32::from(channels) {
            return Err("WAV channel mask does not match channel count".into());
        }
        // The standard KSDATAFORMAT_SUBTYPE_PCM / IEEE_FLOAT GUID suffix.
        if bytes[26..40] != [0, 0, 0, 0, 0x10, 0, 0x80, 0, 0, 0xaa, 0, 0x38, 0x9b, 0x71] {
            return Err("Unsupported extensible WAV subformat".into());
        }
        tag = u16_at(bytes, 24);
    }
    let samples = match (tag, bits) {
        (1, 8) => SampleFormat::Unsigned8,
        (1, 16) => SampleFormat::Signed16,
        (1, 24) => SampleFormat::Signed24,
        (1, 32) => SampleFormat::Signed32,
        (3, 32) => SampleFormat::Float32,
        _ => return Err("Unsupported WAV sample format".into()),
    };
    let expected_align = u32::from(channels) * samples.width() as u32;
    if expected_align != u32::from(block_align)
        || expected_align.checked_mul(rate) != Some(byte_rate)
    {
        return Err("Inconsistent WAV block alignment or byte rate".into());
    }
    Ok(WaveFormat {
        samples,
        channels,
        rate,
        block_align: usize::from(block_align),
    })
}

fn decode_wav(bytes: &[u8]) -> Result<DecodedWavetable, String> {
    if bytes.len() < 12 || &bytes[..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err("Expected a RIFF/WAVE file".into());
    }
    if u64::from(u32_at(bytes, 4)) + 8 != bytes.len() as u64 {
        return Err("WAV RIFF length does not match source length".into());
    }
    let mut format = None;
    let mut data = None;
    let mut position = 12;
    while position < bytes.len() {
        if bytes.len() - position < 8 {
            return Err("Truncated WAV chunk header".into());
        }
        let id = &bytes[position..position + 4];
        let length = u32_at(bytes, position + 4) as usize;
        position += 8;
        // Check before addition/slicing, including the RIFF odd-chunk pad byte.
        if length > bytes.len() - position {
            return Err("Truncated WAV chunk data".into());
        }
        let chunk = &bytes[position..position + length];
        match id {
            b"fmt " => {
                if format.is_some() {
                    return Err("Duplicate WAV format chunk".into());
                }
                format = Some(parse_wave_format(chunk)?);
            }
            b"data" => {
                if data.is_some() {
                    return Err("Duplicate WAV data chunk".into());
                }
                data = Some(chunk);
            }
            _ => {}
        }
        position += length;
        if !length.is_multiple_of(2) {
            if position == bytes.len() {
                return Err("Missing WAV odd-chunk padding".into());
            }
            position += 1;
        }
    }
    let format = format.ok_or("Missing WAV format chunk")?;
    let data = data.ok_or("Missing WAV data chunk")?;
    if data.len() % format.block_align != 0 {
        return Err("Partial WAV sample frame".into());
    }
    Ok(DecodedWavetable {
        samples: decode_samples(data, format.samples)?,
        source_sample_rate: Some(format.rate),
        source_channels: format.channels,
        suggested_wave_length: format
            .rate
            .is_power_of_two()
            .then_some(format.rate as usize),
    })
}

/// Encode mono PCM16 RIFF/WAVE with sample rate equal to `wave_length`, as used
/// to convey the wavetable length by Rack. The length need not divide the sample
/// count. Finite samples outside [-1, 1] clip. Quantization matches dr_wav's
/// float-to-PCM16 conversion: zero encodes as -1, and +1 as 32767.
/// The caller retains its samples unchanged on every error.
pub fn encode_wavetable_wav(samples: &[f32], wave_length: usize) -> Result<Vec<u8>, String> {
    validate_sample_count(samples.len())?;
    let rate = u32::try_from(wave_length).map_err(|_| "WAV wave length exceeds u32")?;
    let byte_rate = rate
        .checked_mul(2)
        .filter(|_| rate != 0)
        .ok_or("WAV wave length must be nonzero and fit the PCM16 byte rate")?;
    if samples.iter().any(|sample| !sample.is_finite()) {
        return Err("Wavetable contains a nonfinite sample".into());
    }
    let data_length = (samples.len() * 2) as u32;
    let mut bytes = Vec::with_capacity(44 + data_length as usize);
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data_length).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16_u32.to_le_bytes());
    bytes.extend_from_slice(&1_u16.to_le_bytes());
    bytes.extend_from_slice(&1_u16.to_le_bytes());
    bytes.extend_from_slice(&rate.to_le_bytes());
    bytes.extend_from_slice(&byte_rate.to_le_bytes());
    bytes.extend_from_slice(&2_u16.to_le_bytes());
    bytes.extend_from_slice(&16_u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data_length.to_le_bytes());
    for &sample in samples {
        let quantized = ((sample.clamp(-1.0, 1.0) + 1.0) * 32767.5) as i32 - 32768;
        bytes.extend_from_slice(&(quantized as i16).to_le_bytes());
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fmt(tag: u16, channels: u16, rate: u32, bits: u16) -> Vec<u8> {
        let align = channels * (bits / 8);
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&tag.to_le_bytes());
        bytes.extend_from_slice(&channels.to_le_bytes());
        bytes.extend_from_slice(&rate.to_le_bytes());
        bytes.extend_from_slice(&(rate * u32::from(align)).to_le_bytes());
        bytes.extend_from_slice(&align.to_le_bytes());
        bytes.extend_from_slice(&bits.to_le_bytes());
        bytes
    }

    fn riff(chunks: &[(&[u8; 4], &[u8])]) -> Vec<u8> {
        let mut bytes = b"RIFF\0\0\0\0WAVE".to_vec();
        for (id, data) in chunks {
            bytes.extend_from_slice(*id);
            bytes.extend_from_slice(&(data.len() as u32).to_le_bytes());
            bytes.extend_from_slice(data);
            if data.len() % 2 != 0 {
                bytes.push(0);
            }
        }
        let size = (bytes.len() - 8) as u32;
        bytes[4..8].copy_from_slice(&size.to_le_bytes());
        bytes
    }

    #[test]
    fn raw_signed_endpoints_aliases_and_metadata() {
        for (extensions, bytes, expected) in [
            (
                ["s8", ".I8"],
                vec![128, 0, 127],
                vec![-1.0, 0.0, 127.0 / 128.0],
            ),
            (
                ["s16", "i16"],
                vec![0, 128, 0, 0, 255, 127],
                vec![-1.0, 0.0, 32767.0 / 32768.0],
            ),
            (
                ["s24", "i24"],
                vec![0, 0, 128, 0, 0, 0, 255, 255, 127],
                vec![-1.0, 0.0, 8388607.0 / 8388608.0],
            ),
            (
                ["s32", "i32"],
                vec![0, 0, 0, 128, 0, 0, 0, 0, 255, 255, 255, 127],
                vec![-1.0, 0.0, 1.0],
            ),
        ] {
            for extension in extensions {
                let decoded = decode_wavetable(&bytes, extension).unwrap();
                assert_eq!(decoded.samples, expected);
                assert_eq!(decoded.source_channels, 1);
                assert_eq!(decoded.source_sample_rate, None);
                assert_eq!(decoded.suggested_wave_length, None);
            }
        }
    }

    #[test]
    fn wav_endpoints_and_unsigned8_differ_from_signed_raw() {
        let fixtures = [
            (
                8,
                vec![0, 128, 255],
                vec![-1.0, 128.0 * (2.0_f32 / 255.0) - 1.0, 1.0],
            ),
            (
                16,
                vec![0, 128, 0, 0, 255, 127],
                vec![-1.0, 0.0, 32767.0 / 32768.0],
            ),
            (
                24,
                vec![0, 0, 128, 0, 0, 0, 255, 255, 127],
                vec![-1.0, 0.0, 8388607.0 / 8388608.0],
            ),
            (
                32,
                vec![0, 0, 0, 128, 0, 0, 0, 0, 255, 255, 255, 127],
                vec![-1.0, 0.0, 1.0],
            ),
        ];
        for (bits, data, expected) in fixtures {
            let bytes = riff(&[(b"fmt ", &fmt(1, 1, 44100, bits)), (b"data", &data)]);
            assert_eq!(decode_wavetable(&bytes, "wav").unwrap().samples, expected);
        }
    }

    #[test]
    fn stereo_is_interleaved_and_only_power_two_rates_suggest_length() {
        let data: Vec<u8> = [0_i16, 32767, -32768, 16384]
            .iter()
            .flat_map(|x| x.to_le_bytes())
            .collect();
        for rate in [1024, 44100, 1] {
            let bytes = riff(&[(b"fmt ", &fmt(1, 2, rate, 16)), (b"data", &data)]);
            let decoded = decode_wavetable(&bytes, ".WAV").unwrap();
            assert_eq!(decoded.samples, [0.0, 32767.0 / 32768.0, -1.0, 0.5]);
            assert_eq!(decoded.source_channels, 2);
            assert_eq!(decoded.source_sample_rate, Some(rate));
            assert_eq!(
                decoded.suggested_wave_length,
                rate.is_power_of_two().then_some(rate as usize)
            );
        }
    }

    #[test]
    fn float_preserves_finite_values_and_rejects_nonfinite_atomically() {
        let values = [-2.5_f32, -0.0, 0.0, 3.25, f32::MAX];
        let data: Vec<u8> = values.iter().flat_map(|x| x.to_le_bytes()).collect();
        assert_eq!(decode_wavetable(&data, "f32").unwrap().samples, values);
        let wav = riff(&[(b"fmt ", &fmt(3, 1, 2048, 32)), (b"data", &data)]);
        assert_eq!(decode_wavetable(&wav, "wav").unwrap().samples, values);
        for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let data: Vec<u8> = [0.5_f32, value]
                .iter()
                .flat_map(|x| x.to_le_bytes())
                .collect();
            assert!(decode_wavetable(&data, "f32").is_err());
            assert!(decode_wavetable(
                &riff(&[(b"fmt ", &fmt(3, 1, 1, 32)), (b"data", &data)]),
                "wav"
            )
            .is_err());
            assert!(encode_wavetable_wav(&[0.5, value], 1024).is_err());
        }
    }

    #[test]
    fn odd_unknown_chunks_and_data_before_format_work() {
        let bytes = riff(&[
            (b"JUNK", &[7]),
            (b"data", &[0, 128]),
            (b"fmt ", &fmt(1, 1, 32, 16)),
            (b"LIST", &[1, 2, 3]),
        ]);
        assert_eq!(decode_wavetable(&bytes, "wav").unwrap().samples, [-1.0]);
        let mut missing_pad = riff(&[(b"fmt ", &fmt(1, 1, 32, 8)), (b"data", &[128])]);
        missing_pad.pop();
        let size = (missing_pad.len() - 8) as u32;
        missing_pad[4..8].copy_from_slice(&size.to_le_bytes());
        assert!(decode_wavetable(&missing_pad, "wav").is_err());
    }

    #[test]
    fn extensible_pcm_and_float_validate_guid_mask_and_valid_bits() {
        for tag in [1_u16, 3] {
            let mut format = fmt(0xfffe, 2, 1024, 32);
            format.extend_from_slice(&22_u16.to_le_bytes());
            format.extend_from_slice(&32_u16.to_le_bytes());
            format.extend_from_slice(&3_u32.to_le_bytes());
            format.extend_from_slice(&u32::from(tag).to_le_bytes());
            format.extend_from_slice(&[0, 0, 0x10, 0, 0x80, 0, 0, 0xaa, 0, 0x38, 0x9b, 0x71]);
            let data = if tag == 1 {
                vec![0, 0, 0, 128, 0, 0, 0, 64]
            } else {
                [-1.0_f32, 0.5]
                    .iter()
                    .flat_map(|x| x.to_le_bytes())
                    .collect()
            };
            let decode = |format: &[u8]| {
                decode_wavetable(&riff(&[(b"fmt ", format), (b"data", &data)]), "wav")
            };
            assert_eq!(decode(&format).unwrap().samples, [-1.0, 0.5]);
            for (offset, value) in [(16, 21), (18, 24), (20, 1), (30, 0xff)] {
                let mut bad = format.clone();
                bad[offset] = value;
                assert!(decode(&bad).is_err());
            }
        }
    }

    #[test]
    fn malformed_riff_and_format_are_rejected() {
        let valid = riff(&[(b"fmt ", &fmt(1, 2, 1024, 16)), (b"data", &[0, 0, 0, 0])]);
        for (offset, value) in [
            (0, b'X'),
            (4, 0xff),
            (8, b'X'),
            (16, 0xff),
            (20, 2),
            (22, 0),
            (25, 0),
            (29, 0),
            (32, 1),
            (34, 12),
            (40, 0xff),
        ] {
            let mut bad = valid.clone();
            bad[offset] = value;
            assert!(decode_wavetable(&bad, "wav").is_err(), "offset {offset}");
        }
        for end in 0..valid.len() {
            assert!(decode_wavetable(&valid[..end], "wav").is_err());
        }
        let format = fmt(1, 1, 1024, 16);
        for chunks in [
            vec![(b"fmt ", format.as_slice())],
            vec![(b"data", &[0, 0][..])],
            vec![
                (b"fmt ", format.as_slice()),
                (b"fmt ", format.as_slice()),
                (b"data", &[0, 0][..]),
            ],
            vec![
                (b"fmt ", format.as_slice()),
                (b"data", &[0, 0][..]),
                (b"data", &[0, 0][..]),
            ],
        ] {
            assert!(decode_wavetable(&riff(&chunks), "wav").is_err());
        }
        assert!(decode_wavetable(
            &riff(&[(b"fmt ", &fmt(1, 2, 1024, 16)), (b"data", &[0, 0])]),
            "wav"
        )
        .is_err());
    }

    #[test]
    fn format_extensions_compression_and_rate_overflow() {
        let mut extended = fmt(1, 1, 1024, 16);
        extended.extend_from_slice(&0_u16.to_le_bytes());
        let make = |format: &[u8]| riff(&[(b"fmt ", format), (b"data", &[0, 0])]);
        assert!(decode_wavetable(&make(&extended), "wav").is_ok());
        extended[16] = 1;
        assert!(decode_wavetable(&make(&extended), "wav").is_err());
        for (tag, bits) in [(2, 16), (6, 8), (7, 8), (3, 64), (1, 12)] {
            assert!(decode_wavetable(&make(&fmt(tag, 1, 1024, bits)), "wav").is_err());
        }
        let mut bad_rate = fmt(1, 1, 1024, 16);
        bad_rate[4..8].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(decode_wavetable(&make(&bad_rate), "wav").is_err());
        let mut oversized_channels = fmt(1, 1, 1024, 16);
        oversized_channels[2..4].copy_from_slice(&u16::MAX.to_le_bytes());
        assert!(decode_wavetable(&make(&oversized_channels), "wav").is_err());
    }

    #[test]
    fn size_limits_and_partial_raw_are_enforced() {
        for extension in ["s8", "s16", "s24", "s32", "f32"] {
            assert!(decode_wavetable(&[], extension).is_err());
        }
        for extension in ["s16", "i16", "s24", "i24", "s32", "i32", "f32"] {
            assert!(decode_wavetable(&[0], extension).is_err());
        }
        assert!(decode_wavetable(&[0], "aif").is_err());
        assert!(decode_wavetable(&[0], "aiff").is_err());
        let maximum = vec![0; MAX_WAVETABLE_SAMPLES];
        assert_eq!(
            decode_wavetable(&maximum, "i8").unwrap().samples.len(),
            MAX_WAVETABLE_SAMPLES
        );
        assert!(decode_wavetable(&vec![0; MAX_WAVETABLE_SAMPLES + 1], "i8").is_err());
        assert!(decode_wavetable(&vec![0; MAX_SOURCE_BYTES + 1], "wav").is_err());
        let mut hostile = b"RIFF\x04\x00\x00\x00WAVEdata\xff\xff\xff\xff".to_vec();
        hostile[4..8].copy_from_slice(&12_u32.to_le_bytes());
        assert!(decode_wavetable(&hostile, "wav").is_err());
    }

    #[test]
    fn pcm16_save_clips_and_matches_drwav_zero_quantization() {
        let values = [-2.0, -1.0, -0.5, 0.0, 0.5, 1.0, 2.0];
        let bytes = encode_wavetable_wav(&values, 1024).unwrap();
        let pcm: Vec<i16> = bytes[44..]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|x| i16::from_le_bytes([x[0], x[1]]))
            .collect();
        assert_eq!(pcm, [-32768, -32768, -16385, -1, 16383, 32767, 32767]);
        let decoded = decode_wavetable(&bytes, "wav").unwrap();
        assert_eq!(decoded.source_channels, 1);
        assert_eq!(decoded.source_sample_rate, Some(1024));
        assert_eq!(decoded.suggested_wave_length, Some(1024));
        for (actual, source) in decoded.samples.iter().zip(values) {
            assert!((actual - source.clamp(-1.0, 1.0)).abs() <= 2.0 / 32768.0);
        }
        assert!(encode_wavetable_wav(&[], 1024).is_err());
        assert!(encode_wavetable_wav(&[0.0], 0).is_err());
        assert!(encode_wavetable_wav(&[0.0], u32::MAX as usize).is_err());
        assert!(encode_wavetable_wav(&vec![0.0; MAX_WAVETABLE_SAMPLES + 1], 1024).is_err());
    }
}
