//! Render pipeline — vertex types and shader configuration.

use bytemuck::{Pod, Zeroable};

/// Vertex for rendering colored rectangles.
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct RectVertex {
    pub position: [f32; 2],
    pub color: [f32; 4],
}

impl RectVertex {
    pub fn desc() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<RectVertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &[
                wgpu::VertexAttribute {
                    offset: 0,
                    shader_location: 0,
                    format: wgpu::VertexFormat::Float32x2,
                },
                wgpu::VertexAttribute {
                    offset: std::mem::size_of::<[f32; 2]>() as wgpu::BufferAddress,
                    shader_location: 1,
                    format: wgpu::VertexFormat::Float32x4,
                },
            ],
        }
    }
}

/// Generate vertices for a filled rectangle.
/// Coordinates are in NDC: x in [-1, 1], y in [-1, 1].
pub fn rect_vertices(
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    screen_w: f32,
    screen_h: f32,
    color: [f32; 4],
) -> [RectVertex; 6] {
    // Convert from pixel coordinates to NDC
    let left = (x / screen_w) * 2.0 - 1.0;
    let right = ((x + w) / screen_w) * 2.0 - 1.0;
    let top = 1.0 - (y / screen_h) * 2.0;
    let bottom = 1.0 - ((y + h) / screen_h) * 2.0;

    [
        // Triangle 1
        RectVertex { position: [left, top], color },
        RectVertex { position: [right, top], color },
        RectVertex { position: [left, bottom], color },
        // Triangle 2
        RectVertex { position: [right, top], color },
        RectVertex { position: [right, bottom], color },
        RectVertex { position: [left, bottom], color },
    ]
}

/// The shader source for rendering colored rectangles.
pub const RECT_SHADER: &str = r#"
struct VertexInput {
    @location(0) position: vec2<f32>,
    @location(1) color: vec4<f32>,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec4<f32>,
};

@vertex
fn vs_main(in: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    out.clip_position = vec4<f32>(in.position, 0.0, 1.0);
    out.color = in.color;
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    return in.color;
}
"#;
