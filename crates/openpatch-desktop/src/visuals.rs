//! Native OpenGL visualizer. All GL work stays on the UI/render thread.
use eframe::{egui, egui_glow, glow};
use glow::HasContext;
use std::sync::{Arc, Mutex};

pub const MODES: [(i32, &str); 4] = [
    (0, "Aurora"),
    (1, "Spectrum"),
    (2, "Waveform"),
    (3, "Ferrofluid"),
];

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
}
impl Visualizer {
    pub fn new(gl: &glow::Context) -> Result<Self, String> {
        Ok(Self {
            program: Arc::new(Mutex::new(Program::new(gl, DEFAULT_SHADER)?)),
            dynamics: Mutex::new(FerroDynamics::default()),
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
        ui.painter().add(egui::PaintCallback {
            rect,
            callback: Arc::new(egui_glow::CallbackFn::new(move |_, painter| {
                let gl = painter.gl();
                let p = program.lock().unwrap();
                // SAFETY: eframe invokes the callback on its current render context.
                unsafe {
                    gl.use_program(Some(p.program));
                    gl.bind_vertex_array(Some(p.vao));
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
    fn spectrum_finds_tone() {
        let wave = std::array::from_fn(|i| (std::f32::consts::TAU * i as f32 * 8.0 / 256.0).sin());
        let bins = super::spectrum(&wave);
        assert!((bins[8] - 1.0).abs() < 0.001);
        assert!(bins[2] < 0.001);
        assert!(super::spectrum(&[0.0; 256]).iter().all(|&v| v == 0.0));
    }
}
