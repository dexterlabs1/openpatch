//! Native OpenGL visualizer. All GL work stays on the UI/render thread.
use eframe::{egui, egui_glow, glow};
use glow::HasContext;
use std::sync::{Arc, Mutex};

/// Built-in modes in display order. Ids are the `u_mode` values in
/// `visuals.frag`; the original four keep ids 0-3, and the studio set (4-8)
/// comes first in the picker. Every studio mode draws the signal itself as
/// thin lines: live waveform sweeps, or the recent spectrum history.
pub const MODES: [(i32, &str); 9] = [
    (5, "Horizon"),
    (4, "Phosphor"),
    (8, "Ridges"),
    (6, "Pulsar"),
    (7, "Vector"),
    (0, "Aurora"),
    (1, "Spectrum"),
    (2, "Waveform"),
    (3, "Ferrofluid"),
];
/// The first mode shown when the Visuals tab opens.
pub const DEFAULT_MODE: i32 = 5;
/// Modes before this display index are the newer studio set; a thin divider
/// separates them from the original four.
pub const STUDIO_MODE_COUNT: usize = 5;

/// Signal history uploaded as a small single-channel float texture
/// (`u_history`): rows `0..SPECTRUM_ROWS` hold the log-frequency spectrum, newest
/// first (row 0 is live); the next `SWEEP_ROWS` rows hold trigger-aligned,
/// level-normalised waveform sweeps, newest first (also live in its first row).
pub const HISTORY_WIDTH: usize = 128;
pub const SPECTRUM_ROWS: usize = 32;
pub const SWEEP_ROWS: usize = 16;
const HISTORY_ROWS: usize = SPECTRUM_ROWS + SWEEP_ROWS;
/// Spectrum rows per second; slow enough that each ridge reads as one line.
const SPECTRUM_RATE: f32 = 5.0;
/// Sweeps per second kept for phosphor persistence.
const SWEEP_RATE: f32 = 24.0;

pub const DEFAULT_SHADER: &str = include_str!("../assets/shaders/visuals.frag");
const VERTEX: &str = "#version 330 core\nout vec2 v_uv;\nvoid main(){ vec2 p=vec2(float((gl_VertexID << 1) & 2),float(gl_VertexID & 2)); v_uv=p; gl_Position=vec4(p*2.0-1.0,0.0,1.0); }";

struct Program {
    program: glow::Program,
    vao: glow::VertexArray,
    time: Option<glow::UniformLocation>,
    resolution: Option<glow::UniformLocation>,
    level: Option<glow::UniformLocation>,
    wave: Option<glow::UniformLocation>,
    spectrum: Option<glow::UniformLocation>,
    mode: Option<glow::UniformLocation>,
    ferro: Option<glow::UniformLocation>,
    history: Option<glow::UniformLocation>,
    history_info: Option<glow::UniformLocation>,
}

impl Program {
    fn new(gl: &glow::Context, fragment: &str) -> Result<Self, String> {
        // SAFETY: caller owns the current GL context on the native UI thread.
        unsafe {
            let program = gl.create_program()?;
            let mut shaders = Vec::new();
            let result = (|| {
                for (kind, source) in [
                    (glow::VERTEX_SHADER, VERTEX),
                    (glow::FRAGMENT_SHADER, fragment),
                ] {
                    let shader = gl.create_shader(kind)?;
                    shaders.push(shader);
                    gl.shader_source(shader, source);
                    gl.compile_shader(shader);
                    if !gl.get_shader_compile_status(shader) {
                        return Err(gl.get_shader_info_log(shader));
                    }
                    gl.attach_shader(program, shader);
                }
                gl.link_program(program);
                if !gl.get_program_link_status(program) {
                    return Err(gl.get_program_info_log(program));
                }
                gl.create_vertex_array()
            })();
            for shader in shaders {
                gl.delete_shader(shader);
            }
            let vao = match result {
                Ok(vao) => vao,
                Err(err) => {
                    gl.delete_program(program);
                    return Err(err);
                }
            };
            Ok(Self {
                time: gl.get_uniform_location(program, "u_time"),
                resolution: gl.get_uniform_location(program, "u_resolution"),
                level: gl.get_uniform_location(program, "u_level"),
                wave: gl.get_uniform_location(program, "u_wave[0]"),
                spectrum: gl.get_uniform_location(program, "u_spectrum[0]"),
                mode: gl.get_uniform_location(program, "u_mode"),
                ferro: gl.get_uniform_location(program, "u_ferro"),
                history: gl.get_uniform_location(program, "u_history"),
                history_info: gl.get_uniform_location(program, "u_history_info"),
                program,
                vao,
            })
        }
    }
    fn destroy(&self, gl: &glow::Context) {
        // SAFETY: called once with the same current context that created these objects.
        unsafe {
            gl.delete_program(self.program);
            gl.delete_vertex_array(self.vao);
        }
    }
}

pub struct Visualizer {
    program: Arc<Mutex<Program>>,
    dynamics: Mutex<FerroDynamics>,
    history: Mutex<SignalHistory>,
    history_texture: glow::Texture,
}
impl Visualizer {
    pub fn new(gl: &glow::Context) -> Result<Self, String> {
        let program = Program::new(gl, DEFAULT_SHADER)?;
        // SAFETY: caller owns the current GL context on the native UI thread.
        let history_texture = unsafe {
            let texture = match gl.create_texture() {
                Ok(texture) => texture,
                Err(err) => {
                    program.destroy(gl);
                    return Err(err);
                }
            };
            gl.bind_texture(glow::TEXTURE_2D, Some(texture));
            for (parameter, value) in [
                (glow::TEXTURE_MIN_FILTER, glow::NEAREST),
                (glow::TEXTURE_MAG_FILTER, glow::NEAREST),
                (glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE),
                (glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE),
            ] {
                gl.tex_parameter_i32(glow::TEXTURE_2D, parameter, value as i32);
            }
            let zeros = vec![0_u8; HISTORY_WIDTH * HISTORY_ROWS * 4];
            gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 4);
            gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                glow::R32F as i32,
                HISTORY_WIDTH as i32,
                HISTORY_ROWS as i32,
                0,
                glow::RED,
                glow::FLOAT,
                glow::PixelUnpackData::Slice(Some(&zeros)),
            );
            gl.bind_texture(glow::TEXTURE_2D, None);
            texture
        };
        Ok(Self {
            program: Arc::new(Mutex::new(program)),
            dynamics: Mutex::new(FerroDynamics::default()),
            history: Mutex::new(SignalHistory::default()),
            history_texture,
        })
    }
    pub fn compile(&mut self, gl: &glow::Context, source: &str) -> Result<(), String> {
        if source.len() > 65_536 {
            return Err("Shader exceeds 64 KiB".into());
        }
        let replacement = Program::new(gl, source)?;
        let mut current = self.program.lock().unwrap();
        current.destroy(gl);
        *current = replacement;
        Ok(())
    }
    pub fn paint(
        &self,
        ui: &mut egui::Ui,
        size: egui::Vec2,
        time: f32,
        level: f32,
        wave: [f32; 256],
        mode: i32,
    ) {
        let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
        let program = self.program.clone();
        let spectrum = spectrum(&wave);
        let ferro = self.dynamics.lock().unwrap().update(time, &spectrum, level);
        let (texels, info) = {
            let mut history = self.history.lock().unwrap();
            let info = history.update(time, &wave, &spectrum);
            (history.texel_bytes(), info)
        };
        let texture = self.history_texture;
        ui.painter().add(egui::PaintCallback {
            rect,
            callback: Arc::new(egui_glow::CallbackFn::new(move |_, painter| {
                let gl = painter.gl();
                let p = program.lock().unwrap();
                // SAFETY: eframe invokes the callback on its current render context;
                // egui_glow restores its program, VAO and texture unit 0 afterwards.
                unsafe {
                    gl.use_program(Some(p.program));
                    gl.bind_vertex_array(Some(p.vao));
                    gl.active_texture(glow::TEXTURE0);
                    gl.bind_texture(glow::TEXTURE_2D, Some(texture));
                    gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 4);
                    gl.tex_sub_image_2d(
                        glow::TEXTURE_2D,
                        0,
                        0,
                        0,
                        HISTORY_WIDTH as i32,
                        HISTORY_ROWS as i32,
                        glow::RED,
                        glow::FLOAT,
                        glow::PixelUnpackData::Slice(Some(&texels)),
                    );
                    gl.uniform_1_i32(p.history.as_ref(), 0);
                    gl.uniform_4_f32(p.history_info.as_ref(), info[0], info[1], info[2], info[3]);
                    gl.uniform_1_f32(p.time.as_ref(), time);
                    gl.uniform_2_f32(p.resolution.as_ref(), rect.width(), rect.height());
                    gl.uniform_1_f32(p.level.as_ref(), level);
                    gl.uniform_1_f32_slice(p.wave.as_ref(), &wave);
                    gl.uniform_1_f32_slice(p.spectrum.as_ref(), &spectrum);
                    gl.uniform_1_i32(p.mode.as_ref(), mode);
                    gl.uniform_4_f32(p.ferro.as_ref(), ferro[0], ferro[1], ferro[2], ferro[3]);
                    gl.draw_arrays(glow::TRIANGLES, 0, 3);
                }
            })),
        });
    }
    pub fn destroy(&self, gl: &glow::Context) {
        self.program.lock().unwrap().destroy(gl);
        // SAFETY: same context that created the texture in `new`.
        unsafe { gl.delete_texture(self.history_texture) };
    }
}

/// Visual-thread signal history behind the line-art modes. Everything is
/// sanitised and bounded before upload, so the shader never sees NaN or Inf.
struct SignalHistory {
    previous_time: Option<f32>,
    texels: Vec<f32>,
    /// Live per-column spectrum envelope (fast attack, gentle release).
    envelope: [f32; HISTORY_WIDTH],
    spectrum_gain: f32,
    sweep_gain: f32,
    spectrum_clock: f32,
    sweep_clock: f32,
    rows_pushed: u32,
    delay: f32,
}
impl Default for SignalHistory {
    fn default() -> Self {
        Self {
            previous_time: None,
            texels: vec![0.0; HISTORY_WIDTH * HISTORY_ROWS],
            envelope: [0.0; HISTORY_WIDTH],
            spectrum_gain: 0.25,
            sweep_gain: 0.25,
            spectrum_clock: 0.0,
            sweep_clock: 0.0,
            rows_pushed: 0,
            delay: 16.0,
        }
    }
}
impl SignalHistory {
    /// Advances the history and returns `u_history_info`: spectrum scroll
    /// phase, sweep scroll phase, XY delay in samples and a wrapped row count.
    fn update(&mut self, time: f32, wave: &[f32; 256], spectrum: &[f32; 64]) -> [f32; 4] {
        let finite = |x: f32| if x.is_finite() { x } else { 0.0 };
        let dt = self
            .previous_time
            .map_or(1.0 / 60.0, |old| finite(time - old).clamp(0.0, 0.1));
        if time.is_finite() {
            self.previous_time = Some(time);
        }
        let follow = |current: &mut f32, target: f32, attack: f32, release: f32| {
            let tau = if target > *current { attack } else { release };
            *current += (target - *current) * (1.0 - (-dt / tau).exp());
        };

        // Spectrum: log-like frequency axis, compressed, with slow auto-gain so
        // quiet patches still draw ridges and loud ones do not clip.
        let mut target = [0.0_f32; HISTORY_WIDTH];
        let mut loudest = 0.0_f32;
        for (column, value) in target.iter_mut().enumerate() {
            let x = column as f32 / (HISTORY_WIDTH - 1) as f32;
            let bin = 1.0 + x.powf(1.7) * 61.0;
            let index = (bin as usize).min(61);
            let a = finite(spectrum[index]).clamp(0.0, 4.0);
            let b = finite(spectrum[index + 1]).clamp(0.0, 4.0);
            *value = (a + (b - a) * (bin - index as f32)).max(0.0).sqrt() * (1.0 + x * 1.6);
            loudest = loudest.max(*value);
        }
        follow(&mut self.spectrum_gain, loudest, 0.25, 4.0);
        let spectrum_scale = 0.9 / self.spectrum_gain.max(0.06);
        for (column, value) in target.iter().enumerate() {
            follow(
                &mut self.envelope[column],
                (value * spectrum_scale).min(1.3),
                0.02,
                0.22,
            );
        }
        self.spectrum_clock += dt * SPECTRUM_RATE;
        if self.spectrum_clock >= 1.0 {
            self.spectrum_clock = self.spectrum_clock.fract().min(0.999);
            self.texels
                .copy_within(0..(SPECTRUM_ROWS - 1) * HISTORY_WIDTH, HISTORY_WIDTH);
            self.rows_pushed = (self.rows_pushed + 1) % 4096;
        }
        // Row 0 is live; the shift above moved its last state into row 1, so a
        // ridge leaves the front exactly as it was drawn.
        self.texels[..HISTORY_WIDTH].copy_from_slice(&self.envelope);

        // Sweeps: a hysteresis trigger on a rising crossing keeps periodic
        // signals standing still, like a real oscilloscope.
        let peak = wave.iter().fold(0.0_f32, |p, &v| p.max(finite(v).abs()));
        follow(&mut self.sweep_gain, peak, 0.05, 2.5);
        let gain = (0.85 / self.sweep_gain.max(0.02)).min(40.0);
        let sample = |i: usize| finite(wave[i]) * gain;
        let mut armed = false;
        let mut trigger = None;
        let mut first_crossing = None;
        let mut last_crossing = 0;
        let mut crossings = 0_usize;
        for i in 1..wave.len() {
            if sample(i) < -0.08 {
                armed = true;
            } else if armed && sample(i - 1) < 0.0 && sample(i) >= 0.0 {
                armed = false;
                crossings += 1;
                first_crossing.get_or_insert(i);
                last_crossing = i;
                if trigger.is_none() && i + HISTORY_WIDTH <= wave.len() {
                    trigger = Some(i);
                }
            }
        }
        let start = trigger.unwrap_or(0);
        // A quarter period turns a pure tone into a circle on the XY display;
        // richer signals fold into their own figures.
        if let (Some(first), true) = (first_crossing, crossings > 1) {
            let period = (last_crossing - first) as f32 / (crossings - 1) as f32;
            follow(
                &mut self.delay,
                (period * 0.25).clamp(3.0, 48.0),
                0.15,
                0.15,
            );
        }
        self.sweep_clock += dt * SWEEP_RATE;
        let sweeps = SPECTRUM_ROWS * HISTORY_WIDTH;
        if self.sweep_clock >= 1.0 {
            self.sweep_clock = self.sweep_clock.fract().min(0.999);
            self.texels.copy_within(
                sweeps..sweeps + (SWEEP_ROWS - 1) * HISTORY_WIDTH,
                sweeps + HISTORY_WIDTH,
            );
        }
        for i in 0..HISTORY_WIDTH {
            self.texels[sweeps + i] = sample(start + i).clamp(-1.4, 1.4);
        }
        [
            self.spectrum_clock,
            self.sweep_clock,
            self.delay,
            self.rows_pushed as f32,
        ]
    }

    fn texel_bytes(&self) -> Vec<u8> {
        self.texels.iter().flat_map(|v| v.to_ne_bytes()).collect()
    }
}

// UI-only envelopes keep rapid transients smooth without changing the audio path.
#[derive(Default)]
struct FerroDynamics {
    previous_time: Option<f32>,
    bands: [f32; 4],
    slow_level: f32,
}
impl FerroDynamics {
    fn update(&mut self, time: f32, spectrum: &[f32; 64], level: f32) -> [f32; 4] {
        let finite = |x: f32| if x.is_finite() { x.max(0.0) } else { 0.0 };
        let energy = |range: std::ops::Range<usize>, gain: f32| {
            let strength = (range
                .map(|i| finite(spectrum[i]).min(4.0).powi(2))
                .sum::<f32>()
                .sqrt()
                * gain)
                .max(0.0);
            strength / (1.0 + strength)
        };
        let peak = finite(level).min(1.0);
        self.slow_level += (peak - self.slow_level) * 0.025;
        let pulse =
            ((peak - self.slow_level * 0.82).max(0.0) / (self.slow_level + 0.015) * 1.8).min(1.0);
        let target = [
            energy(1..5, 35.0),
            energy(5..19, 55.0),
            energy(19..56, 90.0),
            pulse,
        ];
        let dt = self
            .previous_time
            .map_or(1.0 / 60.0, |old| finite(time - old).min(0.1));
        if time.is_finite() {
            self.previous_time = Some(time);
        }
        for (current, target) in self.bands.iter_mut().zip(target) {
            let tau = if target > *current { 0.008 } else { 0.085 };
            *current += (target - *current) * (1.0 - (-dt / tau).exp());
        }
        self.bands
    }
}

// Small Hann-windowed DFT runs on the visual/UI thread, never in the audio callback.
fn spectrum(wave: &[f32; 256]) -> [f32; 64] {
    std::array::from_fn(|bin| {
        let mut re = 0.0;
        let mut im = 0.0;
        for (i, &sample) in wave.iter().enumerate() {
            let phase = std::f32::consts::TAU * i as f32 / 256.0;
            let value = sample * (0.5 - 0.5 * phase.cos());
            re += value * (phase * bin as f32).cos();
            im += value * (phase * bin as f32).sin();
        }
        (re * re + im * im).sqrt() / 64.0
    })
}
#[cfg(test)]
mod tests {
    #[test]
    fn ferrofluid_separates_bands_and_releases_to_rest() {
        let tone = |bin: f32| {
            std::array::from_fn(|i| 0.1 * (std::f32::consts::TAU * i as f32 * bin / 256.0).sin())
        };
        let mut low = super::FerroDynamics::default();
        let mut high = super::FerroDynamics::default();
        for frame in 0..60 {
            let t = frame as f32 / 60.0;
            low.update(t, &super::spectrum(&tone(2.0)), 0.1);
            high.update(t, &super::spectrum(&tone(32.0)), 0.1);
        }
        assert!(low.bands[0] > 0.3 && low.bands[2] < 0.001);
        assert!(high.bands[2] > 0.5 && high.bands[0] < 0.001);
        for frame in 60..300 {
            low.update(frame as f32 / 60.0, &[0.0; 64], 0.0);
        }
        assert!(low.bands.iter().all(|&v| v < 0.0001));
    }

    #[test]
    fn ferrofluid_envelopes_stay_bounded_across_bad_inputs_and_clock_jumps() {
        let mut dynamics = super::FerroDynamics::default();
        for time in [0.0, 0.016, f32::NAN, -10.0, 10000.0, f32::INFINITY] {
            for value in [f32::NAN, f32::INFINITY, -100.0, f32::MAX, 1.0] {
                let bands = dynamics.update(time, &[value; 64], value);
                assert!(bands
                    .iter()
                    .all(|v| v.is_finite() && (0.0..=1.0).contains(v)));
            }
        }
    }

    #[test]
    fn built_in_modes_are_unique_and_dispatched_by_the_bundled_shader() {
        let mut ids: Vec<i32> = super::MODES.iter().map(|(id, _)| *id).collect();
        ids.sort_unstable();
        assert_eq!(ids, (0..super::MODES.len() as i32).collect::<Vec<_>>());
        assert!(super::MODES
            .iter()
            .any(|(id, _)| *id == super::DEFAULT_MODE));
        let shader = super::DEFAULT_SHADER;
        assert!(shader.starts_with("#version 330 core\n"));
        assert!(
            shader.len() < 65_536,
            "editor reset path rejects larger sources"
        );
        for id in 4..super::MODES.len() as i32 {
            assert!(
                shader.contains(&format!("u_mode == {id}")),
                "mode {id} has no shader branch"
            );
        }
        assert!(shader.contains("if (u_mode >= 4)"));
    }

    #[test]
    fn signal_history_triggers_sweeps_and_scrolls_rows_without_jumps() {
        use super::{HISTORY_WIDTH, SPECTRUM_ROWS};
        let mut history = super::SignalHistory::default();
        // A 50-sample period: every live sweep must start on the same rising
        // crossing regardless of where the ring buffer happened to be read.
        let mut info = [0.0; 4];
        for frame in 0..240 {
            let offset = frame as f32 * 7.3;
            let wave = std::array::from_fn(|i| {
                0.4 * (std::f32::consts::TAU * (i as f32 + offset) / 50.0).sin()
            });
            let before: Vec<f32> = history.texels[..HISTORY_WIDTH].to_vec();
            let pushed = info[3];
            info = history.update(frame as f32 / 60.0, &wave, &super::spectrum(&wave));
            if info[3] != pushed {
                // A row was just pushed: it leaves the front exactly as drawn.
                assert_eq!(
                    &history.texels[HISTORY_WIDTH..2 * HISTORY_WIDTH],
                    &before[..]
                );
            }
            let sweep = &history.texels[SPECTRUM_ROWS * HISTORY_WIDTH..];
            if frame > 30 {
                assert!(
                    sweep[0].abs() < 0.12 && sweep[3] > sweep[0],
                    "sweep starts rising at zero"
                );
                assert!((sweep[50] - sweep[0]).abs() < 0.12, "next period lines up");
            }
        }
        assert!(
            (info[2] - 12.5).abs() < 1.0,
            "XY delay is a quarter period: {}",
            info[2]
        );
        assert!(info[3] >= 15.0, "spectrum rows were pushed");
        assert!(history
            .texels
            .iter()
            .all(|v| v.is_finite() && v.abs() <= 1.4));
    }

    #[test]
    fn signal_history_stays_bounded_across_bad_inputs_and_clock_jumps() {
        let mut history = super::SignalHistory::default();
        for time in [0.0, 0.016, f32::NAN, -10.0, 10000.0, f32::INFINITY, 10000.5] {
            for value in [f32::NAN, f32::INFINITY, -100.0, f32::MAX, 1.0, 0.0] {
                let info = history.update(time, &[value; 256], &[value; 64]);
                assert!(info.iter().all(|v| v.is_finite()));
                assert!((0.0..1.0).contains(&info[0]) && (0.0..1.0).contains(&info[1]));
                assert!(history
                    .texels
                    .iter()
                    .all(|v| v.is_finite() && v.abs() <= 1.4));
            }
        }
    }

    #[test]
    fn spectrum_finds_tone() {
        let wave = std::array::from_fn(|i| (std::f32::consts::TAU * i as f32 * 8.0 / 256.0).sin());
        let bins = super::spectrum(&wave);
        assert!((bins[8] - 1.0).abs() < 0.001);
        assert!(bins[2] < 0.001);
        assert!(super::spectrum(&[0.0; 256]).iter().all(|&v| v == 0.0));
    }
}
