//! Native OpenGL visualizer. All GL work stays on the UI/render thread.
use eframe::{egui, egui_glow, glow};
use glow::HasContext;
use std::sync::{Arc, Mutex};

pub const DEFAULT_SHADER: &str = r#"#version 330 core
in vec2 v_uv;
out vec4 frag_color;
uniform vec2 u_resolution;
uniform float u_time;
uniform float u_level;
uniform float u_wave[256];
uniform float u_spectrum[64];
uniform int u_mode; // 0 aurora, 1 spectrum, 2 oscilloscope
void main() {
    vec2 uv = v_uv;
    vec2 p = (uv - 0.5) * vec2(u_resolution.x/u_resolution.y, 1.0);
    vec3 col = vec3(0.019, 0.028, 0.054);
    float grid = pow(max(0.0, cos(p.x*48.0)), 32.0)*0.012
               + pow(max(0.0, cos(p.y*48.0)), 32.0)*0.012;
    col += vec3(0.2,0.55,0.7)*grid;
    if (u_mode == 0) {
        for (int i=0; i<7; i++) {
            float f = float(i);
            float y = sin(p.x*(1.5+f*0.16)+u_time*(0.2+f*0.045)+f*0.45)*0.15;
            y += sin(p.x*3.0-u_time*0.3+f)*0.045;
            y += u_wave[int(clamp(uv.x*255.0,0.0,255.0))]*0.30;
            float d = abs(p.y-y+(f-3.0)*0.025);
            vec3 tint = mix(vec3(0.04,0.8,0.74),vec3(0.48,0.25,0.97),f/6.0);
            col += tint*(0.0018/(d+0.004)+exp(-d*18.0)*0.08)*(0.38+u_level*3.0);
        }
    } else if (u_mode == 1) {
        float bin = clamp(floor(uv.x*64.0),0.0,63.0);
        float amplitude = clamp(sqrt(u_spectrum[int(bin)])*2.3,0.0,0.9);
        float bar = step(0.12,fract(uv.x*64.0))*step(uv.y,amplitude+0.045)*step(0.045,uv.y);
        col += mix(vec3(0.08,0.85,0.75),vec3(0.55,0.3,0.98),uv.x)*bar*(0.45+uv.y);
    } else {
        float index = clamp(uv.x*254.0,0.0,254.0);
        float wave = mix(u_wave[int(index)],u_wave[int(index)+1],fract(index));
        float d = abs(uv.y-0.5-wave*1.3);
        col += vec3(0.12,0.9,0.75)*(0.003/(d+0.004)+exp(-d*80.0)*0.1);
    }
    col *= 1.0-smoothstep(0.25,0.85,length(uv-0.5))*0.5;
    frag_color = vec4(col,1.0);
}
"#;
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
}
impl Visualizer {
    pub fn new(gl: &glow::Context) -> Result<Self, String> {
        Ok(Self {
            program: Arc::new(Mutex::new(Program::new(gl, DEFAULT_SHADER)?)),
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
                    gl.draw_arrays(glow::TRIANGLES, 0, 3);
                }
            })),
        });
    }
    pub fn destroy(&self, gl: &glow::Context) {
        self.program.lock().unwrap().destroy(gl);
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
    fn spectrum_finds_tone() {
        let wave = std::array::from_fn(|i| (std::f32::consts::TAU * i as f32 * 8.0 / 256.0).sin());
        let bins = super::spectrum(&wave);
        assert!((bins[8] - 1.0).abs() < 0.001);
        assert!(bins[2] < 0.001);
        assert!(super::spectrum(&[0.0; 256]).iter().all(|&v| v == 0.0));
    }
}
