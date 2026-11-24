//! OpenGL drawing for the 3D viewport (runs inside an egui paint callback).

use eframe::glow::{self, HasContext};
use glam::{Mat4, Vec2, Vec3};

/// Interleaved vertex: position, normal, uv, colour (12 floats).
pub const STRIDE: usize = 12;

#[derive(Default)]
pub struct Batch {
    pub data: Vec<f32>,
}

impl Batch {
    pub fn push(&mut self, p: Vec3, n: Vec3, uv: Vec2, c: [f32; 4]) {
        self.data.extend_from_slice(&[p.x, p.y, p.z, n.x, n.y, n.z, uv.x, uv.y, c[0], c[1], c[2], c[3]]);
    }
    pub fn line(&mut self, a: Vec3, b: Vec3, c: [f32; 4]) {
        self.push(a, Vec3::Y, Vec2::ZERO, c);
        self.push(b, Vec3::Y, Vec2::ZERO, c);
    }
    pub fn point(&mut self, p: Vec3, c: [f32; 4]) {
        self.push(p, Vec3::Y, Vec2::ZERO, c);
    }
    pub fn count(&self) -> i32 {
        (self.data.len() / STRIDE) as i32
    }
}

/// Geometry that rarely changes (the bulk of a big scene). It is uploaded to the
/// graphics card once and only again when `key` changes.
#[derive(Clone, Default)]
pub struct Layer {
    pub key: u64,
    pub tris: std::sync::Arc<Batch>,
    pub lines: std::sync::Arc<Batch>,
}

pub const LAYERS: usize = 2;

/// Everything needed to draw one frame; built on the UI thread.
pub struct FrameData {
    /// Cached layers: unselected objects, and selected objects.
    pub layers: [Layer; LAYERS],
    pub view_proj: Mat4,
    pub tris: Batch,
    /// Lines that should always be hidden by geometry (grid).
    pub grid: Batch,
    pub lines: Batch,
    pub points: Batch,
    /// (version, width, height, rgba) - re-uploaded when the version changes.
    pub texture: Option<(u64, u32, u32, std::sync::Arc<Vec<u8>>)>,
    /// See-through faces, with every edge and vertex showing.
    pub xray: bool,
    /// Draw the grid behind everything instead of letting it cut through the geometry.
    pub grid_behind: bool,
}

pub struct Renderer {
    program: glow::Program,
    vao: glow::VertexArray,
    vbo: glow::Buffer,
    texture: glow::Texture,
    white: glow::Texture,
    tex_version: u64,
    layers: [GpuLayer; LAYERS],
}

/// Vertex buffers for one cached layer.
struct GpuLayer {
    key: Option<u64>,
    tris: (glow::VertexArray, glow::Buffer, i32),
    lines: (glow::VertexArray, glow::Buffer, i32),
}

const VS: &str = r#"#version 330 core
layout(location = 0) in vec3 a_pos;
layout(location = 1) in vec3 a_normal;
layout(location = 2) in vec2 a_uv;
layout(location = 3) in vec4 a_color;
uniform mat4 u_view_proj;
uniform float u_point_size;
out vec3 v_normal;
out vec2 v_uv;
out vec4 v_color;
void main() {
    v_normal = a_normal;
    v_uv = a_uv;
    v_color = a_color;
    gl_Position = u_view_proj * vec4(a_pos, 1.0);
    gl_PointSize = u_point_size;
}
"#;

const FS: &str = r#"#version 330 core
in vec3 v_normal;
in vec2 v_uv;
in vec4 v_color;
uniform sampler2D u_tex;
uniform int u_mode; // 0 = lit + textured, 1 = flat colour
uniform float u_alpha; // below 1: x-ray
out vec4 o_color;
void main() {
    if (u_mode == 1) {
        o_color = v_color;
        return;
    }
    // Image rows are stored top-first, UV origin is bottom-left.
    vec4 t = texture(u_tex, vec2(v_uv.x, 1.0 - v_uv.y));
    if (t.a < 0.5) discard;
    float d = dot(normalize(v_normal), normalize(vec3(0.35, 0.85, 0.5)));
    // Back faces are drawn dark so flipped normals are easy to spot.
    float light = gl_FrontFacing ? 0.72 + 0.28 * d : (u_alpha < 1.0 ? 0.6 : 0.3);
    vec3 rgb = mix(t.rgb * light, v_color.rgb, v_color.a);
    o_color = vec4(rgb, u_alpha);
}
"#;

impl Renderer {
    pub fn new(gl: &glow::Context) -> Result<Self, String> {
        unsafe {
            let program = gl.create_program()?;
            let mut shaders = Vec::new();
            for (kind, src) in [(glow::VERTEX_SHADER, VS), (glow::FRAGMENT_SHADER, FS)] {
                let sh = gl.create_shader(kind)?;
                gl.shader_source(sh, src);
                gl.compile_shader(sh);
                if !gl.get_shader_compile_status(sh) {
                    return Err(format!("shader compile failed: {}", gl.get_shader_info_log(sh)));
                }
                gl.attach_shader(program, sh);
                shaders.push(sh);
            }
            gl.link_program(program);
            if !gl.get_program_link_status(program) {
                return Err(format!("shader link failed: {}", gl.get_program_info_log(program)));
            }
            for sh in shaders {
                gl.detach_shader(program, sh);
                gl.delete_shader(sh);
            }

            let (vao, vbo) = Self::make_buffer(gl)?;
            let mut layers = Vec::new();
            for _ in 0..LAYERS {
                let (tv, tb) = Self::make_buffer(gl)?;
                let (lv, lb) = Self::make_buffer(gl)?;
                layers.push(GpuLayer { key: None, tris: (tv, tb, 0), lines: (lv, lb, 0) });
            }
            let Ok(layers) = <[GpuLayer; LAYERS]>::try_from(layers) else { return Err("layer setup failed".into()) };

            let white = Self::make_texture(gl, 1, 1, &[205, 205, 210, 255])?;
            let texture = Self::make_texture(gl, 1, 1, &[255, 255, 255, 255])?;
            Ok(Renderer { program, vao, vbo, texture, white, tex_version: 0, layers })
        }
    }

    /// A vertex array + buffer set up for the interleaved vertex format.
    unsafe fn make_buffer(gl: &glow::Context) -> Result<(glow::VertexArray, glow::Buffer), String> {
        let vao = gl.create_vertex_array()?;
        let vbo = gl.create_buffer()?;
        gl.bind_vertex_array(Some(vao));
        gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));
        let stride = (STRIDE * 4) as i32;
        for (loc, size, offset) in [(0, 3, 0), (1, 3, 3), (2, 2, 6), (3, 4, 8)] {
            gl.enable_vertex_attrib_array(loc);
            gl.vertex_attrib_pointer_f32(loc, size, glow::FLOAT, false, stride, offset * 4);
        }
        gl.bind_vertex_array(None);
        Ok((vao, vbo))
    }

    unsafe fn make_texture(gl: &glow::Context, w: u32, h: u32, rgba: &[u8]) -> Result<glow::Texture, String> {
        let tex = gl.create_texture()?;
        Self::upload(gl, tex, w, h, rgba);
        Ok(tex)
    }

    unsafe fn upload(gl: &glow::Context, tex: glow::Texture, w: u32, h: u32, rgba: &[u8]) {
        gl.bind_texture(glow::TEXTURE_2D, Some(tex));
        gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 1);
        gl.tex_image_2d(
            glow::TEXTURE_2D, 0, glow::RGBA8 as i32, w as i32, h as i32, 0,
            glow::RGBA, glow::UNSIGNED_BYTE, glow::PixelUnpackData::Slice(Some(rgba)),
        );
        // Nearest filtering: pixel art must stay crisp.
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MIN_FILTER, glow::NEAREST as i32);
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MAG_FILTER, glow::NEAREST as i32);
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_S, glow::REPEAT as i32);
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_T, glow::REPEAT as i32);
    }

    pub fn paint(&mut self, gl: &glow::Context, frame: &FrameData) {
        unsafe {
            if let Some((version, w, h, rgba)) = &frame.texture {
                if *version != self.tex_version {
                    Self::upload(gl, self.texture, *w, *h, rgba);
                    self.tex_version = *version;
                }
            }

            gl.enable(glow::DEPTH_TEST);
            gl.depth_func(glow::LEQUAL);
            gl.depth_mask(true);
            gl.disable(glow::BLEND);
            gl.disable(glow::CULL_FACE);
            gl.clear_depth_f32(1.0);
            gl.clear(glow::DEPTH_BUFFER_BIT); // limited to the viewport by egui's scissor rect
            gl.enable(0x8642); // GL_PROGRAM_POINT_SIZE

            gl.use_program(Some(self.program));
            gl.uniform_matrix_4_f32_slice(
                gl.get_uniform_location(self.program, "u_view_proj").as_ref(), false, &frame.view_proj.to_cols_array(),
            );
            gl.uniform_1_i32(gl.get_uniform_location(self.program, "u_tex").as_ref(), 0);
            let mode = gl.get_uniform_location(self.program, "u_mode");
            let point_size = gl.get_uniform_location(self.program, "u_point_size");
            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(glow::TEXTURE_2D, Some(if frame.texture.is_some() { self.texture } else { self.white }));
            // Re-upload a cached layer only when its contents changed.
            let bytes_of = |batch: &Batch| std::slice::from_raw_parts(batch.data.as_ptr() as *const u8, batch.data.len() * 4);
            for (gpu, layer) in self.layers.iter_mut().zip(&frame.layers) {
                if gpu.key != Some(layer.key) {
                    for (target, batch) in [(&mut gpu.tris, &layer.tris), (&mut gpu.lines, &layer.lines)] {
                        gl.bind_buffer(glow::ARRAY_BUFFER, Some(target.1));
                        gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, bytes_of(batch), glow::STATIC_DRAW);
                        target.2 = batch.count();
                    }
                    gpu.key = Some(layer.key);
                }
            }

            // Per-frame geometry goes through one buffer that is refilled for each batch.
            let (vao, vbo) = (self.vao, self.vbo);
            let draw = |batch: &Batch, prim: u32| {
                if batch.count() > 0 {
                    gl.bind_vertex_array(Some(vao));
                    gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));
                    gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, bytes_of(batch), glow::STREAM_DRAW);
                    gl.draw_arrays(prim, 0, batch.count());
                }
            };
            let draw_cached = |buffer: &(glow::VertexArray, glow::Buffer, i32), prim: u32| {
                if buffer.2 > 0 {
                    gl.bind_vertex_array(Some(buffer.0));
                    gl.draw_arrays(prim, 0, buffer.2);
                }
            };

            let grid_first = frame.grid_behind || frame.xray;
            if grid_first {
                // The grid as a backdrop: nothing it touches is hidden by it.
                gl.disable(glow::DEPTH_TEST);
                gl.uniform_1_i32(mode.as_ref(), 1);
                gl.enable(glow::BLEND);
                gl.blend_func(glow::SRC_ALPHA, glow::ONE_MINUS_SRC_ALPHA);
                draw(&frame.grid, glow::LINES);
                gl.disable(glow::BLEND);
                gl.enable(glow::DEPTH_TEST);
            }

            // Faces, pushed back slightly so edges and vertices on top of them win the depth test.
            gl.uniform_1_i32(mode.as_ref(), 0);
            let alpha = gl.get_uniform_location(self.program, "u_alpha");
            gl.uniform_1_f32(alpha.as_ref(), if frame.xray { 0.38 } else { 1.0 });
            if frame.xray {
                // See-through: blended, and not hiding anything behind.
                gl.enable(glow::BLEND);
                gl.blend_func(glow::SRC_ALPHA, glow::ONE_MINUS_SRC_ALPHA);
                gl.depth_mask(false);
            }
            gl.enable(glow::POLYGON_OFFSET_FILL);
            gl.polygon_offset(1.0, 1.0);
            for layer in &self.layers {
                draw_cached(&layer.tris, glow::TRIANGLES);
            }
            draw(&frame.tris, glow::TRIANGLES);
            gl.disable(glow::POLYGON_OFFSET_FILL);
            if frame.xray {
                gl.depth_mask(true);
                // Edges and vertices show through everything.
                gl.disable(glow::DEPTH_TEST);
            }

            gl.uniform_1_i32(mode.as_ref(), 1);
            gl.enable(glow::BLEND);
            gl.blend_func(glow::SRC_ALPHA, glow::ONE_MINUS_SRC_ALPHA);
            if !grid_first {
                draw(&frame.grid, glow::LINES);
            }
            for layer in &self.layers {
                draw_cached(&layer.lines, glow::LINES);
            }
            draw(&frame.lines, glow::LINES);
            gl.uniform_1_f32(point_size.as_ref(), 7.0);
            draw(&frame.points, glow::POINTS);

            gl.bind_vertex_array(None);
            gl.use_program(None);
            gl.disable(glow::DEPTH_TEST);
            gl.disable(0x8642);
        }
    }

    pub fn destroy(&self, gl: &glow::Context) {
        unsafe {
            gl.delete_program(self.program);
            gl.delete_vertex_array(self.vao);
            gl.delete_buffer(self.vbo);
            gl.delete_texture(self.texture);
            gl.delete_texture(self.white);
            for layer in &self.layers {
                for (vao, vbo, _) in [layer.tris, layer.lines] {
                    gl.delete_vertex_array(vao);
                    gl.delete_buffer(vbo);
                }
            }
        }
    }
}
