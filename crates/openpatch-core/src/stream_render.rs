//! Bounded-memory offline PCM16 rendering. No live engine or external events.
use crate::Patch;
use serde::Serialize;
use std::fs::{self, File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::Path;

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct RenderStats {
    pub frames: u64,
    pub channels: u16,
    pub sample_rate: u32,
    /// Peak and RMS before PCM clipping, across all emitted channels.
    pub peak: f64,
    pub rms: f64,
    pub clipped_samples: u64,
}

/// Render a fresh compiled graph for 0.001 through 300 seconds. Duration rounds
/// to the nearest frame. Uses fixed 128-frame scratch storage and an 8 KiB writer;
/// audio storage does not grow with duration. Existing paths are never replaced.
pub fn render_wav_stream(patch: &Patch, path: &Path, seconds: f64) -> Result<RenderStats, String> {
    if !seconds.is_finite() || !(0.001..=300.0).contains(&seconds) {
        return Err("seconds must be finite and in 0.001..=300".into());
    }
    // Compilation and all header validation precede creation of the destination.
    let mut engine = patch.compile()?;
    let frames = (seconds * f64::from(patch.sample_rate)).round() as u64;
    let channels = engine.output_channels();
    if !(1..=2).contains(&channels) {
        return Err("WAV requires mono or stereo output".into());
    }
    let block_align = channels * 2;
    let byte_rate = patch
        .sample_rate
        .checked_mul(u32::from(block_align))
        .filter(|_| patch.sample_rate > 0)
        .ok_or("invalid WAV sample rate")?;
    let data_size = frames
        .checked_mul(u64::from(block_align))
        .and_then(|n| u32::try_from(n).ok())
        .filter(|n| *n <= u32::MAX - 36)
        .ok_or("render exceeds RIFF WAV size limit")?;
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| format!("cannot create render WAV: {e}"))?;
    // Keep the original handle alive until cleanup so its file identity cannot
    // be recycled while deciding whether the path still names our new file.
    let mut writer = BufWriter::with_capacity(8192, &file);
    let result = (|| -> Result<RenderStats, String> {
        let mut header = [0_u8; 44];
        header[0..4].copy_from_slice(b"RIFF");
        header[4..8].copy_from_slice(&(data_size + 36).to_le_bytes());
        header[8..16].copy_from_slice(b"WAVEfmt ");
        header[16..20].copy_from_slice(&16_u32.to_le_bytes());
        header[20..22].copy_from_slice(&1_u16.to_le_bytes());
        header[22..24].copy_from_slice(&channels.to_le_bytes());
        header[24..28].copy_from_slice(&patch.sample_rate.to_le_bytes());
        header[28..32].copy_from_slice(&byte_rate.to_le_bytes());
        header[32..34].copy_from_slice(&block_align.to_le_bytes());
        header[34..36].copy_from_slice(&16_u16.to_le_bytes());
        header[36..40].copy_from_slice(b"data");
        header[40..44].copy_from_slice(&data_size.to_le_bytes());
        writer
            .write_all(&header)
            .map_err(|e| format!("cannot write WAV header: {e}"))?;
        let mut stats = RenderStats {
            frames,
            channels,
            sample_rate: patch.sample_rate,
            peak: 0.0,
            rms: 0.0,
            clipped_samples: 0,
        };
        let mut audio = [[0.0_f32; 2]; 128];
        let mut pcm = [0_u8; 128 * 2 * 2];
        let mut rendered = 0;
        let mut squares = 0.0_f64;
        while rendered < frames {
            let length = (frames - rendered).min(128) as usize;
            engine.process_audio_block(&mut audio[..length]);
            let mut offset = 0;
            for pair in &audio[..length] {
                for &value in &pair[..usize::from(channels)] {
                    if !value.is_finite() {
                        return Err(format!(
                            "nonfinite output in render block starting at frame {rendered}"
                        ));
                    }
                    let v = f64::from(value);
                    stats.peak = stats.peak.max(v.abs());
                    squares += v * v;
                    stats.clipped_samples += u64::from(value.abs() > 1.0);
                    let limited = value.clamp(-1.0, 1.0);
                    let integer = if limited < 0.0 {
                        (limited * 32768.0).round() as i16
                    } else {
                        (limited * 32767.0).round() as i16
                    };
                    pcm[offset..offset + 2].copy_from_slice(&integer.to_le_bytes());
                    offset += 2;
                }
            }
            writer
                .write_all(&pcm[..offset])
                .map_err(|e| format!("cannot write WAV audio: {e}"))?;
            rendered += length as u64;
        }
        writer
            .flush()
            .map_err(|e| format!("cannot flush WAV: {e}"))?;
        stats.rms = (squares / (frames * u64::from(channels)) as f64).sqrt();
        Ok(stats)
    })();
    // Discard buffered bytes on failure; do not retry writes from Drop.
    let _ = writer.into_parts();
    if let Err(error) = &result {
        if owns_path(&file, path) {
            if let Err(cleanup) = fs::remove_file(path) {
                return Err(format!(
                    "{error}; could not remove incomplete WAV: {cleanup}"
                ));
            }
        }
    }
    result
}

fn owns_path(file: &File, path: &Path) -> bool {
    let (Ok(original), Ok(current)) = (file.metadata(), fs::symlink_metadata(path)) else {
        return false;
    };
    if !current.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        original.dev() == current.dev() && original.ino() == current.ino()
    }
    #[cfg(not(unix))]
    {
        // On platforms without a stable std file ID, be conservative when a
        // replacement cannot be ruled out; never delete an unverified file.
        original
            .created()
            .ok()
            .is_some_and(|created| current.created().ok() == Some(created))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    struct Directory(std::path::PathBuf);
    impl Directory {
        fn new() -> Self {
            let dir = std::env::temp_dir().join(format!(
                "openpatch-stream-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&dir).unwrap();
            Self(dir)
        }
        fn file(&self, name: &str) -> std::path::PathBuf {
            self.0.join(name)
        }
    }
    impl Drop for Directory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn patch(stereo: bool) -> Patch {
        serde_json::from_value(if stereo {json!({"version":2,"sample_rate":8000,"nodes":[
            {"id":"a","kind":"constant","params":{"value":2.5}},
            {"id":"b","kind":"constant","params":{"value":-5}},
            {"id":"out","kind":"audio_stereo"}],
            "cables":[{"from":"a","to":"out","input":"left"},{"from":"b","to":"out","input":"right"}],"output":"out"})
        } else {json!({"sample_rate":8000,"nodes":[{"id":"a","kind":"constant","params":{"value":0.25}}],"output":"a"})}).unwrap()
    }
    #[test]
    fn streamed_mono_and_stereo_pcm_match_existing_short_renderer() {
        let dir = Directory::new();
        for stereo in [false, true] {
            let patch = patch(stereo);
            let audio = crate::render_audio(&patch, 0.03125).unwrap();
            let reference = dir.file(if stereo {
                "reference2.wav"
            } else {
                "reference1.wav"
            });
            let output = dir.file(if stereo { "stream2.wav" } else { "stream1.wav" });
            crate::write_wav_interleaved(
                &reference,
                &audio.samples,
                audio.sample_rate,
                audio.channels,
            )
            .unwrap();
            let stats = render_wav_stream(&patch, &output, 0.03125).unwrap();
            assert_eq!(fs::read(reference).unwrap(), fs::read(output).unwrap());
            assert_eq!(stats.frames, 250);
            assert_eq!(stats.channels, if stereo { 2 } else { 1 });
            let sum: f64 = audio.samples.iter().map(|&v| f64::from(v).powi(2)).sum();
            assert!((stats.rms - (sum / audio.samples.len() as f64).sqrt()).abs() < 1e-12);
            assert_eq!(stats.clipped_samples, 0);
        }
    }
    #[test]
    fn invalid_requests_create_nothing_and_existing_file_is_untouched() {
        let dir = Directory::new();
        let path = dir.file("out.wav");
        let patch = patch(false);
        for seconds in [f64::NAN, f64::INFINITY, 0.0, 0.0001, 300.1] {
            assert!(render_wav_stream(&patch, &path, seconds).is_err());
            assert!(!path.exists());
        }
        let mut bad = patch.clone();
        bad.nodes[0].kind = "not_a_module".into();
        assert!(render_wav_stream(&bad, &path, 1.0).is_err());
        assert!(!path.exists());
        fs::write(&path, b"keep me").unwrap();
        assert!(render_wav_stream(&patch, &path, 1.0).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"keep me");
    }
    #[test]
    fn long_render_uses_same_fixed_scratch_and_counts_clipping() {
        let dir = Directory::new();
        let path = dir.file("long.wav");
        let mut patch = patch(false);
        patch.nodes[0].params.insert("value".into(), 1.5);
        let stats = render_wav_stream(&patch, &path, 120.0).unwrap();
        assert_eq!(stats.frames, 960000);
        assert_eq!(stats.channels, 1);
        assert_eq!(stats.clipped_samples, stats.frames);
        assert_eq!(stats.peak, 1.5);
        assert_eq!(stats.rms, 1.5);
        assert_eq!(fs::metadata(path).unwrap().len(), 44 + stats.frames * 2);
    }
    #[cfg(unix)]
    #[test]
    fn cleanup_identity_does_not_claim_replaced_path() {
        let dir = Directory::new();
        let path = dir.file("owned.wav");
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .unwrap();
        assert!(owns_path(&file, &path));
        fs::rename(&path, dir.file("moved.wav")).unwrap();
        fs::write(&path, b"replacement").unwrap();
        assert!(!owns_path(&file, &path));
    }
}
