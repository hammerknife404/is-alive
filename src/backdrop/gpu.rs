//! The Miami scene on the GPU: drawn at art resolution into an offscreen
//! framebuffer (a shader for sky, stars, sun and grid; triangles for the
//! palms), then scaled onto the window with hard edges. With glow on, two
//! blur passes at art resolution make a soft halo that's screened over it.
//! The CPU only uploads the static layers when the size changes and a few
//! thousand palm vertices a frame, so it keeps up with any refresh rate
//! without costing a core.

use eframe::egui::PaintCallbackInfo;
use eframe::egui_glow::ShaderVersion;
use eframe::glow::{self, HasContext};

/// Static layers in RGBA8, art resolution, top row first.
pub struct Layers {
    pub size: [usize; 2],
    pub back: Vec<u8>,
    pub mid: Vec<u8>,
}

/// One frame's worth of input from the UI thread.
pub struct Frame {
    /// Scene size in art pixels.
    pub art: [usize; 2],
    /// The window in art pixels (it can be a fraction of one short of `art`).
    pub view: [f32; 2],
    pub phase: f32,
    /// Sun centre x, y, radius and the horizon row.
    pub sun: [f32; 4],
    pub k: f32,
    /// Palm triangles, x y pairs in art pixels.
    pub palms: Vec<f32>,
    pub upload: Option<std::sync::Arc<Layers>>,
    /// Soft glow and haze on the bright parts.
    pub glow: bool,
}

pub struct Gpu {
    /// The renderer, as the driver names it.
    pub name: String,
    scene: glow::Program,
    palm: glow::Program,
    blit: glow::Program,
    vao: glow::VertexArray,
    vbo: glow::Buffer,
    back: glow::Texture,
    mid: glow::Texture,
    fbo: glow::Framebuffer,
    target: glow::Texture,
    /// Glow: the scene blurred across, then down (filtered when scaled up).
    blur: glow::Program,
    across: (glow::Framebuffer, glow::Texture),
    down: (glow::Framebuffer, glow::Texture),
    size: [usize; 2],
}

const FULLSCREEN_VS: &str = r#"
out vec2 v_uv;
void main() {
    vec2 p = vec2(float((gl_VertexID << 1) & 2), float(gl_VertexID & 2));
    v_uv = p;
    gl_Position = vec4(p * 2.0 - 1.0, 0.0, 1.0);
}
"#;

const SCENE_FS: &str = r#"
uniform sampler2D u_back;
uniform sampler2D u_mid;
uniform vec2 u_size;
uniform float u_ph;
uniform vec4 u_sun;
uniform float u_k;
out vec4 o_color;
const float TAU = 6.2831853;
vec3 sun_band(int i) {
    if (i == 0) return vec3(255.0, 226.0, 92.0);
    if (i == 1) return vec3(255.0, 196.0, 88.0);
    if (i == 2) return vec3(255.0, 160.0, 96.0);
    if (i == 3) return vec3(255.0, 120.0, 120.0);
    return vec3(255.0, 86.0, 150.0);
}
void main() {
    float x = floor(gl_FragCoord.x);
    float y = u_size.y - 1.0 - floor(gl_FragCoord.y);
    ivec2 at = ivec2(int(x), int(y));
    vec4 back = texelFetch(u_back, at, 0);
    vec3 c = back.rgb * 255.0;
    float hz = u_sun.w;
    if (back.a < 0.999) {
        float p = back.a * 255.0 / 254.0 * TAU;
        if (sin(p + u_ph * TAU * 2.0) > -0.3) c = vec3(255.0, 236.0, 255.0);
    }
    if (y < hz) {
        float cx = u_sun.x, cy = u_sun.y, r = u_sun.z, K = u_k;
        float ri = r + K, ro = r + 2.0 * K;
        float dx = x - cx, dy = y - cy, d2 = dx * dx + dy * dy;
        if (y >= cy - ro && y <= min(cy + ro, hz - 1.0) && abs(dx) <= ro
                && d2 >= ri * ri && d2 <= ro * ro) {
            c = sin(u_ph * TAU) > 0.0 ? vec3(255.0, 70.0, 150.0) : vec3(220.0, 56.0, 140.0);
        }
        if (y >= cy - r && y < min(cy + r, hz)) {
            float half_w = floor(sqrt(max(r * r - dy * dy, 0.0)));
            int band = int(floor((y - (cy - r)) / (2.0 * r) * 5.0));
            band = band - 5 * (band / 5);
            float stp = floor(u_ph * 8.0 * K);
            float off = mod(y - cy - stp, 8.0 * K);
            bool cut = y > cy - floor(r / 6.0) && off < K + floor(dy / 6.0);
            if (!cut && abs(dx) <= half_w) c = sun_band(band);
        }
    } else {
        for (int k = 0; k <= 12; k++) {
            float t = (float(k) + u_ph) / 12.0;
            if (y == hz + floor((u_size.y - hz) * t * t)) {
                c = t > 0.25 ? vec3(255.0, 60.0, 170.0) : vec3(150.0, 40.0, 120.0);
            }
        }
    }
    vec4 mid = texelFetch(u_mid, at, 0);
    if (mid.a > 0.5) c = mid.rgb * 255.0;
    o_color = vec4(c / 255.0, 1.0);
}
"#;

const PALM_VS: &str = r#"
in vec2 a_pos;
uniform vec2 u_size;
void main() {
    gl_Position = vec4(a_pos.x / u_size.x * 2.0 - 1.0, 1.0 - a_pos.y / u_size.y * 2.0, 0.0, 1.0);
}
"#;

const PALM_FS: &str = r#"
out vec4 o_color;
void main() {
    o_color = vec4(12.0, 4.0, 28.0, 255.0) / 255.0;
}
"#;

/// One direction of a 13-tap Gaussian (sigma 2.6 art pixels).
const BLUR_FS: &str = r#"
uniform sampler2D u_img;
uniform vec2 u_dir;
uniform vec2 u_size;
out vec4 o_color;
void main() {
    ivec2 p = ivec2(gl_FragCoord.xy);
    ivec2 last = ivec2(u_size) - 1;
    vec3 sum = vec3(0.0);
    float total = 0.0;
    for (int i = -6; i <= 6; i++) {
        float w = exp(-0.5 * float(i * i) / 6.76);
        sum += w * texelFetch(u_img, clamp(p + ivec2(u_dir) * i, ivec2(0), last), 0).rgb;
        total += w;
    }
    o_color = vec4(sum / total, 1.0);
}
"#;

/// The scene with hard edges; with glow, the blurred copy's highlights
/// screened on top and a little of it laid over everything (the haze).
const BLIT_FS: &str = r#"
uniform sampler2D u_img;
uniform sampler2D u_glow;
uniform float u_glow_on;
uniform vec2 u_size;
uniform vec2 u_view;
in vec2 v_uv;
out vec4 o_color;
void main() {
    float x = v_uv.x * u_view.x;
    float y = (1.0 - v_uv.y) * u_view.y;
    float ax = min(floor(x), u_size.x - 1.0);
    float ay = min(floor(y), u_size.y - 1.0);
    vec3 c = texelFetch(u_img, ivec2(int(ax), int(u_size.y - 1.0 - ay)), 0).rgb;
    if (u_glow_on > 0.5) {
        vec3 b = texture(u_glow, vec2(x / u_size.x, 1.0 - y / u_size.y)).rgb;
        float l = dot(b, vec3(0.299, 0.587, 0.114));
        vec3 g = b * clamp((l - 0.38) / 0.62, 0.0, 1.0);
        c = 1.0 - (1.0 - c) * (1.0 - g * 0.75);
        c = mix(c, max(c, b), 0.16);
    }
    o_color = vec4(c, 1.0);
}
"#;

/// Renderers that are really the CPU wearing a GPU's clothes.
const SOFTWARE: &[&str] = &[
    "llvmpipe",
    "softpipe",
    "swrast",
    "software",
    "swiftshader",
    "lavapipe",
    "basic render",
    "gdi generic",
];

unsafe fn program(
    gl: &glow::Context,
    prefix: &str,
    vs: &str,
    fs: &str,
    attrib: Option<&str>,
) -> Result<glow::Program, String> {
    unsafe {
        let p = gl.create_program()?;
        let mut shaders = Vec::new();
        for (kind, src) in [(glow::VERTEX_SHADER, vs), (glow::FRAGMENT_SHADER, fs)] {
            let s = gl.create_shader(kind)?;
            gl.shader_source(s, &format!("{prefix}{src}"));
            gl.compile_shader(s);
            if !gl.get_shader_compile_status(s) {
                let log = gl.get_shader_info_log(s);
                gl.delete_shader(s);
                gl.delete_program(p);
                return Err(format!("shader didn't compile: {}", log.trim()));
            }
            gl.attach_shader(p, s);
            shaders.push(s);
        }
        if let Some(a) = attrib {
            gl.bind_attrib_location(p, 0, a);
        }
        gl.link_program(p);
        for s in shaders {
            gl.detach_shader(p, s);
            gl.delete_shader(s);
        }
        if !gl.get_program_link_status(p) {
            let log = gl.get_program_info_log(p);
            gl.delete_program(p);
            return Err(format!("shader didn't link: {}", log.trim()));
        }
        Ok(p)
    }
}

unsafe fn texture(gl: &glow::Context) -> Result<glow::Texture, String> {
    unsafe {
        let t = gl.create_texture()?;
        gl.bind_texture(glow::TEXTURE_2D, Some(t));
        for (k, v) in [
            (glow::TEXTURE_MIN_FILTER, glow::NEAREST),
            (glow::TEXTURE_MAG_FILTER, glow::NEAREST),
            (glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE),
            (glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE),
        ] {
            gl.tex_parameter_i32(glow::TEXTURE_2D, k, v as i32);
        }
        Ok(t)
    }
}

unsafe fn store(gl: &glow::Context, t: glow::Texture, size: [usize; 2], data: Option<&[u8]>) {
    unsafe {
        gl.bind_texture(glow::TEXTURE_2D, Some(t));
        gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 4);
        gl.tex_image_2d(
            glow::TEXTURE_2D,
            0,
            glow::RGBA8 as i32,
            size[0] as i32,
            size[1] as i32,
            0,
            glow::RGBA,
            glow::UNSIGNED_BYTE,
            glow::PixelUnpackData::Slice(data),
        );
    }
}

impl Gpu {
    /// Set up, or say why not: a software renderer (unless `force`), an
    /// OpenGL without GLSL 1.40 / ES 3.0, or a shader the driver rejects.
    pub fn new(gl: &glow::Context, force: bool) -> Result<Self, String> {
        let name = unsafe { gl.get_parameter_string(glow::RENDERER) };
        let lower = name.to_lowercase();
        if !force && let Some(s) = SOFTWARE.iter().find(|s| lower.contains(*s)) {
            return Err(format!("{name} is a software renderer ({s})"));
        }
        let prefix = match ShaderVersion::get(gl) {
            ShaderVersion::Gl140 => "#version 140\n",
            ShaderVersion::Es300 => {
                "#version 300 es\nprecision highp float;\nprecision highp int;\n"
            }
            v => return Err(format!("{name} has only {v:?} shaders")),
        };
        unsafe {
            let scene = program(gl, prefix, FULLSCREEN_VS, SCENE_FS, None)?;
            let palm = program(gl, prefix, PALM_VS, PALM_FS, Some("a_pos"))?;
            let blit = program(gl, prefix, FULLSCREEN_VS, BLIT_FS, None)?;
            let blur = program(gl, prefix, FULLSCREEN_VS, BLUR_FS, None)?;
            let fbo = gl.create_framebuffer()?;
            let glow_tex = texture(gl)?;
            for f in [glow::TEXTURE_MIN_FILTER, glow::TEXTURE_MAG_FILTER] {
                gl.tex_parameter_i32(glow::TEXTURE_2D, f, glow::LINEAR as i32);
            }
            Ok(Self {
                name,
                scene,
                palm,
                blit,
                vao: gl.create_vertex_array()?,
                vbo: gl.create_buffer()?,
                back: texture(gl)?,
                mid: texture(gl)?,
                target: texture(gl)?,
                fbo,
                blur,
                across: (gl.create_framebuffer()?, texture(gl)?),
                down: (gl.create_framebuffer()?, glow_tex),
                size: [0, 0],
            })
        }
    }

    pub fn paint(&mut self, gl: &glow::Context, info: &PaintCallbackInfo, f: &Frame) {
        unsafe {
            if let Some(l) = &f.upload {
                store(gl, self.back, l.size, Some(&l.back));
                store(gl, self.mid, l.size, Some(&l.mid));
            }
            let [w, h] = f.art;
            let outer = gl.get_parameter_framebuffer(glow::FRAMEBUFFER_BINDING);
            if self.size != f.art {
                for (fbo, tex) in [(self.fbo, self.target), self.across, self.down] {
                    store(gl, tex, f.art, None);
                    gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
                    gl.framebuffer_texture_2d(
                        glow::FRAMEBUFFER,
                        glow::COLOR_ATTACHMENT0,
                        glow::TEXTURE_2D,
                        Some(tex),
                        0,
                    );
                }
                self.size = f.art;
            }
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(self.fbo));
            gl.viewport(0, 0, w as i32, h as i32);
            gl.disable(glow::SCISSOR_TEST);
            gl.disable(glow::BLEND);
            gl.bind_vertex_array(Some(self.vao));

            gl.use_program(Some(self.scene));
            let u = |p, n: &str| gl.get_uniform_location(p, n);
            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(glow::TEXTURE_2D, Some(self.back));
            gl.active_texture(glow::TEXTURE1);
            gl.bind_texture(glow::TEXTURE_2D, Some(self.mid));
            gl.uniform_1_i32(u(self.scene, "u_back").as_ref(), 0);
            gl.uniform_1_i32(u(self.scene, "u_mid").as_ref(), 1);
            gl.uniform_2_f32(u(self.scene, "u_size").as_ref(), w as f32, h as f32);
            gl.uniform_1_f32(u(self.scene, "u_ph").as_ref(), f.phase);
            let [a, b, c, d] = f.sun;
            gl.uniform_4_f32(u(self.scene, "u_sun").as_ref(), a, b, c, d);
            gl.uniform_1_f32(u(self.scene, "u_k").as_ref(), f.k);
            gl.draw_arrays(glow::TRIANGLES, 0, 3);

            if !f.palms.is_empty() {
                gl.use_program(Some(self.palm));
                gl.uniform_2_f32(u(self.palm, "u_size").as_ref(), w as f32, h as f32);
                gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.vbo));
                let bytes: Vec<u8> = f.palms.iter().flat_map(|v| v.to_ne_bytes()).collect();
                gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, &bytes, glow::STREAM_DRAW);
                gl.enable_vertex_attrib_array(0);
                gl.vertex_attrib_pointer_f32(0, 2, glow::FLOAT, false, 8, 0);
                gl.draw_arrays(glow::TRIANGLES, 0, (f.palms.len() / 2) as i32);
                gl.disable_vertex_attrib_array(0);
                gl.bind_buffer(glow::ARRAY_BUFFER, None);
            }

            if f.glow {
                gl.use_program(Some(self.blur));
                gl.active_texture(glow::TEXTURE0);
                gl.uniform_1_i32(u(self.blur, "u_img").as_ref(), 0);
                gl.uniform_2_f32(u(self.blur, "u_size").as_ref(), w as f32, h as f32);
                for ((fbo, _), src, dir) in [
                    (self.across, self.target, (1.0, 0.0)),
                    (self.down, self.across.1, (0.0, 1.0)),
                ] {
                    gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
                    gl.bind_texture(glow::TEXTURE_2D, Some(src));
                    gl.uniform_2_f32(u(self.blur, "u_dir").as_ref(), dir.0, dir.1);
                    gl.draw_arrays(glow::TRIANGLES, 0, 3);
                }
            }

            // Onto the window, cropped to it, hard edges.
            gl.bind_framebuffer(glow::FRAMEBUFFER, outer);
            let vp = info.viewport_in_pixels();
            gl.viewport(vp.left_px, vp.from_bottom_px, vp.width_px, vp.height_px);
            let clip = info.clip_rect_in_pixels();
            gl.enable(glow::SCISSOR_TEST);
            gl.scissor(
                clip.left_px,
                clip.from_bottom_px,
                clip.width_px,
                clip.height_px,
            );
            gl.use_program(Some(self.blit));
            gl.active_texture(glow::TEXTURE1);
            gl.bind_texture(glow::TEXTURE_2D, Some(self.down.1));
            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(glow::TEXTURE_2D, Some(self.target));
            gl.uniform_1_i32(u(self.blit, "u_img").as_ref(), 0);
            gl.uniform_1_i32(u(self.blit, "u_glow").as_ref(), 1);
            let on = if f.glow { 1.0 } else { 0.0 };
            gl.uniform_1_f32(u(self.blit, "u_glow_on").as_ref(), on);
            gl.uniform_2_f32(u(self.blit, "u_size").as_ref(), w as f32, h as f32);
            gl.uniform_2_f32(u(self.blit, "u_view").as_ref(), f.view[0], f.view[1]);
            gl.draw_arrays(glow::TRIANGLES, 0, 3);
            gl.bind_vertex_array(None);
            gl.active_texture(glow::TEXTURE0);
            // egui restores the rest of its own state after a callback.
        }
    }
}

/// Palm strokes as triangles: each point stamped as a square (as the CPU's
/// Bresenham does) and each segment as the parallelogram between them.
pub fn triangles(strokes: &[super::Stroke]) -> Vec<f32> {
    let mut v = Vec::new();
    let mut quad = |p: [(f32, f32); 4]| {
        for i in [0, 1, 2, 0, 2, 3] {
            v.extend([p[i].0, p[i].1]);
        }
    };
    for (pts, width) in strokes {
        let lo = -(width / 2) as f32;
        let hi = lo + *width as f32;
        let snapped: Vec<(f32, f32)> = pts.iter().map(|p| (p.0.round(), p.1.round())).collect();
        for &(x, y) in &snapped {
            quad([
                (x + lo, y + lo),
                (x + hi, y + lo),
                (x + hi, y + hi),
                (x + lo, y + hi),
            ]);
        }
        for s in snapped.windows(2) {
            let ((ax, ay), (bx, by)) = (s[0], s[1]);
            // The two square corners furthest from the line's direction.
            let (c1, c2) = if (bx - ax) * (by - ay) >= 0.0 {
                ((hi, lo), (lo, hi))
            } else {
                ((lo, lo), (hi, hi))
            };
            quad([
                (ax + c1.0, ay + c1.1),
                (bx + c1.0, by + c1.1),
                (bx + c2.0, by + c2.1),
                (ax + c2.0, ay + c2.1),
            ]);
        }
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn triangles_cover_a_stroke() {
        let v = triangles(&[(vec![(0.0, 0.0), (10.0, 5.0)], 4)]);
        // Two squares and one parallelogram, two triangles each.
        assert_eq!(v.len(), 3 * 6 * 2);
        let xs: Vec<f32> = v.chunks(2).map(|p| p[0]).collect();
        assert_eq!(xs.iter().copied().fold(f32::MAX, f32::min), -2.0);
        assert_eq!(xs.iter().copied().fold(f32::MIN, f32::max), 12.0);
    }
}
