// SPDX-License-Identifier: GPL-3.0-or-later
// Adapted from RackMCP src/core/listen_analysis.{cpp,hpp} by Zachary Scheffler,
// commit 285e84c5ff2d3ecef25890da07f578af6a0c5ca6. See LICENSE-THIRD-PARTY.
// Offline analysis only: this module allocates and must never run in the audio callback.

use serde::Serialize;

pub const VOLTS_FULL_SCALE: f32 = 10.0;
const PI: f64 = std::f64::consts::PI;

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelLevels {
    pub present: bool,
    pub peak_volts: f32,
    pub rms_volts: f32,
    pub dc_volts: f32,
    pub clipped_samples: u32,
    pub loudness_valid: bool,
    pub loudness_lufs: f64,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Band {
    pub center_hz: f32,
    pub level_db: f32,
}
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ListenAnalysis {
    pub sample_rate: f32,
    pub frames: usize,
    pub channels: [ChannelLevels; 2],
    pub correlation_valid: bool,
    pub correlation: f32,
    pub loudness_valid: bool,
    pub integrated_lufs: f64,
    pub momentary_max_valid: bool,
    pub momentary_max_lufs: f64,
    pub bands: Vec<Band>,
    pub centroid_valid: bool,
    pub centroid_hz: f32,
    pub onset_count: u32,
    pub tempo_valid: bool,
    pub tempo_bpm: f32,
    pub tempo_confidence: f32,
    pub pitch_valid: bool,
    pub pitch_hz: f32,
    pub pitch_confidence: f32,
}

pub fn volts_to_dbfs(volts: f64) -> f64 {
    if volts > 0.0 {
        20.0 * (volts / f64::from(VOLTS_FULL_SCALE)).log10()
    } else {
        f64::NEG_INFINITY
    }
}

pub fn note_name(hz: f32) -> (String, f32) {
    if !hz.is_finite() || hz <= 0.0 {
        return (String::new(), 0.0);
    }
    const NAMES: [&str; 12] = [
        "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
    ];
    let midi = 69.0 + 12.0 * (f64::from(hz) / 440.0).log2();
    let nearest = midi.round() as i64;
    let cents = ((midi - nearest as f64) * 100.0) as f32;
    let pc = nearest.rem_euclid(12) as usize;
    let octave = (nearest as f64 / 12.0).floor() as i64 - 1;
    (format!("{}{octave}", NAMES[pc]), cents)
}

/// Source radix-2 FFT. Kept here instead of using rustfft to preserve operation order.
pub fn fft(re: &mut [f32], im: &mut [f32]) -> Result<(), String> {
    if re.len() != im.len() || !re.len().is_power_of_two() {
        return Err("FFT requires equal power-of-two buffers".into());
    }
    fft_inner(re, im);
    Ok(())
}
fn fft_inner(re: &mut [f32], im: &mut [f32]) {
    let n = re.len();
    let mut j = 0;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j ^= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let mut len = 2;
    while len <= n {
        let ang = -2.0 * PI / len as f64;
        let wr = ang.cos();
        let wi = ang.sin();
        for i in (0..n).step_by(len) {
            let (mut cr, mut ci) = (1.0, 0.0);
            for k in 0..len / 2 {
                let a = i + k;
                let b = i + k + len / 2;
                let tr = f64::from(re[b]) * cr - f64::from(im[b]) * ci;
                let ti = f64::from(re[b]) * ci + f64::from(im[b]) * cr;
                re[b] = (f64::from(re[a]) - tr) as f32;
                im[b] = (f64::from(im[a]) - ti) as f32;
                re[a] = (f64::from(re[a]) + tr) as f32;
                im[a] = (f64::from(im[a]) + ti) as f32;
                let ncr = cr * wr - ci * wi;
                ci = cr * wi + ci * wr;
                cr = ncr;
            }
        }
        len <<= 1;
    }
}
#[derive(Default)]
struct Biquad {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
    z1: f64,
    z2: f64,
}
impl Biquad {
    fn process(&mut self, x: f64) -> f64 {
        let y = self.b0 * x + self.z1;
        self.z1 = self.b1 * x - self.a1 * y + self.z2;
        self.z2 = self.b2 * x - self.a2 * y;
        y
    }
}
fn k_weighting(fs: f64) -> (Biquad, Biquad) {
    let k = (PI * 1681.974450955533 / fs).tan();
    let vh = 10.0_f64.powf(3.999843853973347 / 20.0);
    let vb = vh.powf(0.4996667741545416);
    let q = 0.7071752369554196;
    let a0 = 1.0 + k / q + k * k;
    let shelf = Biquad {
        b0: (vh + vb * k / q + k * k) / a0,
        b1: 2.0 * (k * k - vh) / a0,
        b2: (vh - vb * k / q + k * k) / a0,
        a1: 2.0 * (k * k - 1.0) / a0,
        a2: (1.0 - k / q + k * k) / a0,
        ..Default::default()
    };
    let k = (PI * 38.13547087602444 / fs).tan();
    let q = 0.5003270373238773;
    let a0 = 1.0 + k / q + k * k;
    let hp = Biquad {
        b0: 1.0,
        b1: -2.0,
        b2: 1.0,
        a1: 2.0 * (k * k - 1.0) / a0,
        a2: (1.0 - k / q + k * k) / a0,
        ..Default::default()
    };
    (shelf, hp)
}
fn block_powers(x: &[f32], fs: f64) -> Vec<f64> {
    let (mut shelf, mut hp) = k_weighting(fs);
    let mut sq = Vec::with_capacity(x.len());
    for &x in x {
        let y = hp.process(shelf.process(f64::from(x) / f64::from(VOLTS_FULL_SCALE)));
        sq.push(y * y);
    }
    let block = (0.4 * fs).round() as usize;
    let hop = (0.1 * fs).round() as usize;
    if x.len() < block {
        let mut s = 0.0;
        for v in sq {
            s += v;
        }
        return vec![s / x.len() as f64];
    }
    let mut prefix = vec![0.0; x.len() + 1];
    for i in 0..x.len() {
        prefix[i + 1] = prefix[i] + sq[i];
    }
    let mut out = Vec::new();
    for start in (0..=x.len() - block).step_by(hop) {
        out.push((prefix[start + block] - prefix[start]) / block as f64);
    }
    out
}
fn block_loudness(power: f64) -> f64 {
    -0.691 + 10.0 * power.log10()
}
fn gated_loudness(chans: &[&[f64]]) -> Option<f64> {
    if chans.is_empty() || chans[0].is_empty() {
        return None;
    }
    let mut sum = vec![0.0; chans[0].len()];
    for c in chans {
        for j in 0..sum.len() {
            sum[j] += c[j];
        }
    }
    let (mut abs_sum, mut abs_count) = (0.0, 0);
    for &s in &sum {
        if s > 0.0 && block_loudness(s) > -70.0 {
            abs_sum += s;
            abs_count += 1;
        }
    }
    if abs_count == 0 {
        return None;
    }
    let rel_gate = block_loudness(abs_sum / f64::from(abs_count)) - 10.0;
    let (mut rel_sum, mut rel_count) = (0.0, 0);
    for &s in &sum {
        if s > 0.0 && block_loudness(s) > -70.0 && block_loudness(s) > rel_gate {
            rel_sum += s;
            rel_count += 1;
        }
    }
    if rel_count == 0 {
        None
    } else {
        Some(block_loudness(rel_sum / f64::from(rel_count)))
    }
}
fn hann(n: usize) -> Vec<f32> {
    (0..n)
        .map(|i| (0.5 - 0.5 * (2.0 * PI * i as f64 / n as f64).cos()) as f32)
        .collect()
}
fn averaged_spectrum(x: &[f32], n: usize) -> Vec<f64> {
    let mut power = vec![0.0; n / 2 + 1];
    let w = hann(n);
    let (mut re, mut im) = (vec![0.0; n], vec![0.0; n]);
    let mut wsum2 = 0.0;
    for &v in &w {
        wsum2 += f64::from(v) * f64::from(v);
    }
    let mut frames = 0;
    for start in (0..=x.len() - n).step_by(n / 2) {
        for i in 0..n {
            re[i] = x[start + i] / VOLTS_FULL_SCALE * w[i];
            im[i] = 0.0;
        }
        fft_inner(&mut re, &mut im);
        for k in 0..=n / 2 {
            let mut p = f64::from(re[k]) * f64::from(re[k]) + f64::from(im[k]) * f64::from(im[k]);
            p /= n as f64 * wsum2;
            if k != 0 && k != n / 2 {
                p *= 2.0;
            }
            power[k] += p;
        }
        frames += 1;
    }
    if frames > 0 {
        for p in &mut power {
            *p /= f64::from(frames);
        }
    }
    power
}
fn median(mut v: Vec<f64>) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    let mid = v.len() / 2;
    v.select_nth_unstable_by(mid, |a, b| a.total_cmp(b));
    v[mid]
}
fn yin(
    x: &[f32],
    w: usize,
    tau_min: usize,
    tau_max: usize,
    fs: f64,
    d: &mut Vec<f64>,
    cmnd: &mut Vec<f64>,
) -> Option<(f32, f32)> {
    d.clear();
    d.resize(tau_max + 2, 0.0);
    cmnd.clear();
    cmnd.resize(tau_max + 2, 1.0);
    for tau in 1..=tau_max + 1 {
        let mut s = 0.0;
        for j in 0..w {
            let diff = f64::from(x[j]) - f64::from(x[j + tau]);
            s += diff * diff;
        }
        d[tau] = s;
    }
    let mut running = 0.0;
    for tau in 1..=tau_max + 1 {
        running += d[tau];
        cmnd[tau] = if running > 0.0 {
            d[tau] * tau as f64 / running
        } else {
            1.0
        };
    }
    let mut best = 0;
    let mut tau = tau_min;
    while tau <= tau_max {
        if cmnd[tau] < 0.15 {
            while tau < tau_max && cmnd[tau + 1] < cmnd[tau] {
                tau += 1;
            }
            best = tau;
            break;
        }
        tau += 1;
    }
    if best == 0 {
        let mut m = 2.0;
        for (tau, &value) in cmnd.iter().enumerate().take(tau_max + 1).skip(tau_min) {
            if value < m {
                m = value;
                best = tau;
            }
        }
    }
    if best == 0 || cmnd[best] > 0.3 {
        return None;
    }
    let mut t = best as f64;
    if best > 1 && best < tau_max + 1 {
        let (a, b, c) = (cmnd[best - 1], cmnd[best], cmnd[best + 1]);
        let den = a - 2.0 * b + c;
        if den > 0.0 {
            t += 0.5 * (a - c) / den;
        }
    }
    Some(((fs / t) as f32, (1.0 - cmnd[best]).max(0.0) as f32))
}

/// Analyze one finite capture off the audio thread. Connection flags control analysis,
/// but do not change the raw passthrough or imply a right-channel mono copy.
/// Native guards use the Core patch rate range (8000..=192000 Hz) and source
/// job duration limit (30 s); these are stricter than the standalone C++ API.
pub fn analyze(
    left: &[f32],
    right: &[f32],
    sample_rate: f32,
    left_present: bool,
    right_present: bool,
) -> Result<ListenAnalysis, String> {
    if left.len() != right.len() {
        return Err("Listen channel lengths differ".into());
    }
    if !sample_rate.is_finite() || !(8_000.0..=192_000.0).contains(&sample_rate) {
        return Err("Listen sample rate must be finite and between 8000 and 192000 Hz".into());
    }
    if left.is_empty() || left.len() as f64 > f64::from(sample_rate) * 30.0 {
        return Err("Listen capture must contain frames and be at most 30 seconds".into());
    }
    if left.iter().chain(right).any(|v| !v.is_finite()) {
        return Err("Listen capture contains a non-finite sample".into());
    }
    let frames = left.len();
    let fs = f64::from(sample_rate);
    let mut r = ListenAnalysis {
        sample_rate,
        frames,
        ..Default::default()
    };
    let chans = [left, right];
    let present = [left_present, right_present];
    let mut blocks: [Vec<f64>; 2] = Default::default();
    for c in 0..2 {
        let l = &mut r.channels[c];
        l.present = present[c];
        if !present[c] {
            continue;
        }
        let (mut sum, mut sum_sq) = (0.0, 0.0);
        let mut peak = 0.0_f32;
        let mut clipped = 0;
        for &v in chans[c] {
            let a = v.abs();
            if a > peak {
                peak = a;
            }
            if a >= VOLTS_FULL_SCALE {
                clipped += 1;
            }
            sum += f64::from(v);
            sum_sq += f64::from(v) * f64::from(v);
        }
        l.peak_volts = peak;
        l.rms_volts = (sum_sq / frames as f64).sqrt() as f32;
        l.dc_volts = (sum / frames as f64) as f32;
        l.clipped_samples = clipped;
        blocks[c] = block_powers(chans[c], fs);
        if let Some(lufs) = gated_loudness(&[&blocks[c]]) {
            l.loudness_valid = true;
            l.loudness_lufs = lufs;
        }
    }
    let all: Vec<&[f64]> = (0..2)
        .filter(|&c| present[c])
        .map(|c| blocks[c].as_slice())
        .collect();
    if let Some(lufs) = gated_loudness(&all) {
        r.loudness_valid = true;
        r.integrated_lufs = lufs;
    }
    if !all.is_empty() {
        let mut best = f64::NEG_INFINITY;
        for j in 0..all[0].len() {
            let mut s = 0.0;
            for c in &all {
                s += c[j];
            }
            if s > 0.0 {
                best = best.max(block_loudness(s));
            }
        }
        if best > f64::NEG_INFINITY {
            r.momentary_max_valid = true;
            r.momentary_max_lufs = best;
        }
    }
    if left_present && right_present {
        let ml = f64::from(r.channels[0].dc_volts);
        let mr = f64::from(r.channels[1].dc_volts);
        let (mut sxy, mut sxx, mut syy) = (0.0, 0.0, 0.0);
        for i in 0..frames {
            let a = f64::from(left[i]) - ml;
            let b = f64::from(right[i]) - mr;
            sxy += a * b;
            sxx += a * a;
            syy += b * b;
        }
        if sxx > 1e-12 * frames as f64 && syy > 1e-12 * frames as f64 {
            r.correlation_valid = true;
            r.correlation = (sxy / (sxx * syy).sqrt()) as f32;
        }
    }
    let n_present = usize::from(left_present) + usize::from(right_present);
    if n_present == 0 {
        return Ok(r);
    }
    let mut mono = vec![0.0; frames];
    for i in 0..frames {
        let mut s = 0.0_f32;
        if left_present {
            s += left[i];
        }
        if right_present {
            s += right[i];
        }
        mono[i] = s / n_present as f32;
    }
    // Source float mono summation can overflow even when each input is finite.
    if mono.iter().any(|v| !v.is_finite()) {
        return Err("Listen mono sum overflowed".into());
    }
    if frames >= 256 {
        let n = 8192.min(1 << (usize::BITS - 1 - frames.leading_zeros()));
        let power = averaged_spectrum(&mono, n);
        if power.iter().any(|v| !v.is_finite()) {
            return Err("Listen spectrum overflowed".into());
        }
        let bin_hz = fs / n as f64;
        let nyquist = fs / 2.0;
        for k in -16..=13 {
            let center = 1000.0 * 2.0_f64.powf(f64::from(k) / 3.0);
            let lo = center * 2.0_f64.powf(-1.0 / 6.0);
            let hi = center * 2.0_f64.powf(1.0 / 6.0);
            if lo >= nyquist {
                break;
            }
            let mut p = 0.0;
            for (b, &v) in power.iter().enumerate().skip(1) {
                let f = b as f64 * bin_hz;
                if f >= lo && f < hi {
                    p += v;
                }
            }
            r.bands.push(Band {
                center_hz: center as f32,
                level_db: if p > 1e-12 {
                    (10.0 * p.log10()) as f32
                } else {
                    -120.0
                },
            });
        }
        let (mut num, mut den) = (0.0, 0.0);
        for (b, &p) in power.iter().enumerate().skip(1) {
            let f = b as f64 * bin_hz;
            if f < 20.0 {
                continue;
            }
            num += f * p;
            den += p;
        }
        if den > 1e-14 {
            r.centroid_valid = true;
            r.centroid_hz = (num / den) as f32;
        }
    }
    analyze_rhythm(&mono, fs, &mut r)?;
    let w = (0.0213 * fs).round() as usize;
    let tau_min = (fs / 2000.0).floor() as usize;
    let tau_max = (fs / 40.0).ceil() as usize;
    let span = w + tau_max + 2;
    if frames >= span && tau_min >= 2 {
        let windows = 48.min(1.max((frames - span) / (w / 2) + 1));
        let (mut hzs, mut confs, mut d, mut cmnd) =
            (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        let mut considered = 0;
        for k in 0..windows {
            let start = if windows > 1 {
                (k as f64 * (frames - span) as f64 / (windows - 1) as f64) as usize
            } else {
                0
            };
            let mut ms = 0.0;
            for i in 0..w {
                let v = f64::from(mono[start + i] / VOLTS_FULL_SCALE);
                ms += v * v;
            }
            if ms / (w as f64) < 10.0_f64.powf(-50.0 / 10.0) {
                continue;
            }
            considered += 1;
            if let Some((hz, conf)) =
                yin(&mono[start..], w, tau_min, tau_max, fs, &mut d, &mut cmnd)
            {
                hzs.push(f64::from(hz));
                confs.push(f64::from(conf));
            }
        }
        if considered > 0 && !hzs.is_empty() {
            let voiced = hzs.len() as f64 / f64::from(considered);
            r.pitch_hz = median(hzs) as f32;
            r.pitch_confidence = (median(confs) * voiced) as f32;
            r.pitch_valid = voiced >= 0.3;
        }
    }
    let finite = r.channels.iter().all(|c| {
        c.peak_volts.is_finite()
            && c.rms_volts.is_finite()
            && c.dc_volts.is_finite()
            && c.loudness_lufs.is_finite()
    }) && r.correlation.is_finite()
        && r.integrated_lufs.is_finite()
        && r.momentary_max_lufs.is_finite()
        && r.centroid_hz.is_finite()
        && r.tempo_bpm.is_finite()
        && r.tempo_confidence.is_finite()
        && r.pitch_hz.is_finite()
        && r.pitch_confidence.is_finite()
        && r.bands
            .iter()
            .all(|b| b.center_hz.is_finite() && b.level_db.is_finite());
    if !finite {
        return Err("Listen analysis produced a non-finite metric".into());
    }
    Ok(r)
}

#[allow(clippy::needless_range_loop)] // Keep source lag-index selection and operation order visible.
fn analyze_rhythm(mono: &[f32], fs: f64, r: &mut ListenAnalysis) -> Result<(), String> {
    let frames = mono.len();
    let n = if fs > 50000.0 { 2048 } else { 1024 };
    let hop = n / 4;
    if frames < n * 2 {
        return Ok(());
    }
    let w = hann(n);
    let (mut re, mut im) = (vec![0.0; n], vec![0.0; n]);
    let (mut prev, mut cur) = (vec![0.0; n / 2 + 1], vec![0.0; n / 2 + 1]);
    let (mut flux, mut energy) = (Vec::new(), Vec::new());
    let gate_ms = 10.0_f64.powf(-60.0 / 10.0);
    let mut first = true;
    for start in (0..=frames - n).step_by(hop) {
        let mut ms = 0.0;
        for i in 0..n {
            let v = f64::from(mono[start + i] / VOLTS_FULL_SCALE);
            ms += v * v;
            re[i] = v as f32 * w[i];
            im[i] = 0.0;
        }
        ms /= n as f64;
        fft_inner(&mut re, &mut im);
        let mut f = 0.0;
        for k in 1..=n / 2 {
            cur[k] = (1000.0
                * (f64::from(re[k]) * f64::from(re[k]) + f64::from(im[k]) * f64::from(im[k]))
                    .sqrt())
            .ln_1p();
            if !cur[k].is_finite() {
                return Err("Listen onset spectrum overflowed".into());
            }
            let diff = cur[k] - prev[k];
            if diff > 0.0 {
                f += diff;
            }
        }
        f /= (n / 2) as f64;
        if first || ms < gate_ms {
            f = 0.0;
        }
        first = false;
        flux.push(f);
        energy.push(ms);
        std::mem::swap(&mut prev, &mut cur);
    }
    if flux.iter().chain(&energy).any(|v| !v.is_finite()) {
        return Err("Listen onset spectrum overflowed".into());
    }
    let mut env = vec![0.0; flux.len()];
    for (i, v) in env.iter_mut().enumerate() {
        let (mut s, mut wsum) = (0.0, 0.0);
        for k in -2_i32..=2 {
            let j = i as isize + k as isize;
            if j < 0 || j >= flux.len() as isize {
                continue;
            }
            let g = (-0.5 * f64::from(k) * f64::from(k)).exp();
            s += g * flux[j as usize];
            wsum += g;
        }
        *v = s / wsum;
    }
    let fps = fs / hop as f64;
    let min_gap = (0.05 * fps).ceil() as isize;
    let mut last_onset = -1_000_000_isize;
    for i in 0..flux.len() {
        let mut is_max = true;
        for k in -3_isize..=3 {
            let j = i as isize + k;
            if k != 0 && j >= 0 && j < flux.len() as isize && flux[j as usize] > flux[i] {
                is_max = false;
                break;
            }
        }
        if !is_max {
            continue;
        }
        let mut local = Vec::new();
        for k in -10_isize..=10 {
            let j = i as isize + k;
            if j >= 0 && j < flux.len() as isize {
                local.push(flux[j as usize]);
            }
        }
        let thr = 1.5 * median(local) + 0.05;
        let attack = i < 2
            || energy[i] > 2.0 * energy[i - 2]
            || (i + 1 < energy.len() && energy[i + 1] > 2.0 * energy[i - 2]);
        if attack && flux[i] > thr && i as isize - last_onset >= min_gap {
            r.onset_count += 1;
            last_onset = i as isize;
        }
    }
    if r.onset_count < 3 {
        return Ok(());
    }
    let mut mean = 0.0;
    for &v in &env {
        mean += v;
    }
    mean /= env.len() as f64;
    let e: Vec<f64> = env.iter().map(|v| v - mean).collect();
    let mut r0 = 0.0;
    for &v in &e {
        r0 += v * v;
    }
    r0 /= e.len() as f64;
    let lag_min = (60.0 / 220.0 * fps).floor() as usize;
    let lag_max = (e.len() / 2).min((60.0 / 50.0 * fps).ceil() as usize);
    if r0 <= 0.0 || lag_max <= lag_min + 2 {
        return Ok(());
    }
    let mut ac = vec![0.0; lag_max + 2];
    for lag in lag_min.saturating_sub(1)..=lag_max + 1 {
        if lag >= e.len() {
            break;
        }
        let mut s = 0.0;
        for i in 0..e.len() - lag {
            s += e[i] * e[i + lag];
        }
        ac[lag] = s / (e.len() - lag) as f64 / r0;
    }
    let mut best = 0;
    let mut best_score = f64::NEG_INFINITY;
    for (lag, &value) in ac.iter().enumerate().take(lag_max + 1).skip(lag_min.max(1)) {
        let bpm = 60.0 * fps / lag as f64;
        let o = (bpm / 120.0).log2();
        let score = value * (-0.5 * o * o).exp();
        if score > best_score {
            best_score = score;
            best = lag;
        }
    }
    while best > 0 {
        let mut half = 0;
        let mut half_ac = f64::NEG_INFINITY;
        for h in (best / 2).saturating_sub(1)..=best / 2 + 1 {
            if h >= lag_min.max(1) && h <= lag_max && ac[h] > half_ac {
                half_ac = ac[h];
                half = h;
            }
        }
        if half == 0 || half_ac < 0.8 * ac[best] {
            break;
        }
        best = half;
    }
    if best > 0 {
        let mut t = best as f64;
        if best + 1 < ac.len() {
            let (a, b, c) = (ac[best - 1], ac[best], ac[best + 1]);
            let den = a - 2.0 * b + c;
            if den < 0.0 {
                t += 0.5 * (a - c) / den;
            }
        }
        r.tempo_bpm = (60.0 * fps / t) as f32;
        r.tempo_confidence = ac[best].clamp(0.0, 1.0) as f32;
        r.tempo_valid = r.tempo_confidence > 0.1;
    }
    Ok(())
}

/// Source IEEE float WAV layout, explicitly little endian on every host. A missing
/// right buffer duplicates L for this export only, matching the source writer.
pub fn encode_wav_float_stereo(
    left: &[f32],
    right: Option<&[f32]>,
    sample_rate: u32,
) -> Result<Vec<u8>, String> {
    if sample_rate == 0 || sample_rate > u32::MAX / 8 {
        return Err("Invalid WAV sample rate".into());
    }
    if let Some(right) = right {
        if right.len() != left.len() {
            return Err("WAV channel lengths differ".into());
        }
    }
    if left
        .iter()
        .chain(right.unwrap_or(&[]))
        .any(|v| !v.is_finite())
    {
        return Err("WAV contains a non-finite sample".into());
    }
    let data_bytes = left
        .len()
        .checked_mul(8)
        .and_then(|n| u32::try_from(n).ok())
        .filter(|&n| n <= u32::MAX - 50)
        .ok_or("WAV exceeds RIFF size limit")?;
    let mut out = Vec::with_capacity(58 + data_bytes as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(50 + data_bytes).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&18_u32.to_le_bytes());
    out.extend_from_slice(&3_u16.to_le_bytes());
    out.extend_from_slice(&2_u16.to_le_bytes());
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.extend_from_slice(&(sample_rate * 8).to_le_bytes());
    out.extend_from_slice(&8_u16.to_le_bytes());
    out.extend_from_slice(&32_u16.to_le_bytes());
    out.extend_from_slice(&0_u16.to_le_bytes());
    out.extend_from_slice(b"fact");
    out.extend_from_slice(&4_u32.to_le_bytes());
    out.extend_from_slice(&(left.len() as u32).to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_bytes.to_le_bytes());
    for i in 0..left.len() {
        out.extend_from_slice(&(left[i] / VOLTS_FULL_SCALE).to_le_bytes());
        out.extend_from_slice(&(right.unwrap_or(left)[i] / VOLTS_FULL_SCALE).to_le_bytes());
    }
    Ok(out)
}
