//! The compositor — renders the scene graph to a surface.
//!
//! For the vertical slice, this renders tiles as colored rectangles
//! with hit-region highlighting for local feedback.

use crate::pipeline::{rect_vertices, RectVertex, RECT_SHADER};
use crate::surface::HeadlessSurface;
use tze_hud_scene::graph::SceneGraph;
use tze_hud_scene::types::*;
use tze_hud_telemetry::FrameTelemetry;

/// GPU state and render pipeline.
pub struct Compositor {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pipeline: wgpu::RenderPipeline,
    pub width: u32,
    pub height: u32,
    frame_number: u64,
}

impl Compositor {
    /// Create a new headless compositor.
    pub async fn new_headless(width: u32, height: u32) -> Result<Self, CompositorError> {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(),
            ..Default::default()
        });

        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::LowPower,
                compatible_surface: None,
                force_fallback_adapter: false,
            })
            .await
            .ok_or(CompositorError::NoAdapter)?;

        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("tze_hud_compositor"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::downlevel_defaults(),
                memory_hints: wgpu::MemoryHints::default(),
            }, None)
            .await
            .map_err(|e| CompositorError::DeviceCreation(e.to_string()))?;

        let pipeline = Self::create_pipeline(&device);

        Ok(Self {
            device,
            queue,
            pipeline,
            width,
            height,
            frame_number: 0,
        })
    }

    fn create_pipeline(device: &wgpu::Device) -> wgpu::RenderPipeline {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("rect_shader"),
            source: wgpu::ShaderSource::Wgsl(RECT_SHADER.into()),
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("rect_pipeline_layout"),
            bind_group_layouts: &[],
            push_constant_ranges: &[],
        });

        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("rect_pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[RectVertex::desc()],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Rgba8UnormSrgb,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                polygon_mode: wgpu::PolygonMode::Fill,
                unclipped_depth: false,
                conservative: false,
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        })
    }

    /// Render one frame of the scene to the headless surface.
    /// Returns telemetry for this frame.
    pub fn render_frame(
        &mut self,
        scene: &SceneGraph,
        surface: &HeadlessSurface,
    ) -> FrameTelemetry {
        let frame_start = std::time::Instant::now();
        self.frame_number += 1;

        let mut telemetry = FrameTelemetry::new(self.frame_number);

        // Collect visible tiles
        let tiles = scene.visible_tiles();
        telemetry.tile_count = tiles.len() as u32;
        telemetry.node_count = scene.node_count() as u32;
        telemetry.active_leases = scene.leases.len() as u32;

        // Build vertex buffer from scene
        let mut vertices: Vec<RectVertex> = Vec::new();
        let sw = self.width as f32;
        let sh = self.height as f32;

        for tile in &tiles {
            // Render tile background
            let bg_color = self.tile_background_color(tile, scene);
            let verts = rect_vertices(
                tile.bounds.x,
                tile.bounds.y,
                tile.bounds.width,
                tile.bounds.height,
                sw,
                sh,
                bg_color,
            );
            vertices.extend_from_slice(&verts);

            // Render nodes within the tile
            if let Some(root_id) = tile.root_node {
                self.render_node(root_id, tile, scene, &mut vertices, sw, sh);
            }
        }

        let encode_start = std::time::Instant::now();

        // Create vertex buffer
        let vertex_buffer = if vertices.is_empty() {
            None
        } else {
            let buffer = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("vertex_buffer"),
                contents: bytemuck::cast_slice(&vertices),
                usage: wgpu::BufferUsages::VERTEX,
            });
            Some(buffer)
        };

        // Encode render pass
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("frame_encoder"),
            });

        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("frame_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &surface.view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.05,
                            g: 0.05,
                            b: 0.1,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });

            render_pass.set_pipeline(&self.pipeline);

            if let Some(ref buffer) = vertex_buffer {
                render_pass.set_vertex_buffer(0, buffer.slice(..));
                render_pass.draw(0..vertices.len() as u32, 0..1);
            }
        }

        telemetry.render_encode_us = encode_start.elapsed().as_micros() as u64;

        // Copy to readback buffer
        surface.copy_to_buffer(&mut encoder);

        let submit_start = std::time::Instant::now();
        self.queue.submit(std::iter::once(encoder.finish()));
        self.device.poll(wgpu::Maintain::Wait);
        telemetry.gpu_submit_us = submit_start.elapsed().as_micros() as u64;

        telemetry.frame_time_us = frame_start.elapsed().as_micros() as u64;
        telemetry
    }

    /// Determine the background color for a tile based on its content.
    fn tile_background_color(&self, tile: &Tile, scene: &SceneGraph) -> [f32; 4] {
        if let Some(root_id) = tile.root_node
            && let Some(node) = scene.nodes.get(&root_id)
        {
            match &node.data {
                NodeData::SolidColor(sc) => return sc.color.to_array(),
                NodeData::TextMarkdown(tm) => {
                    if let Some(bg) = &tm.background {
                        return bg.to_array();
                    }
                    return [0.15, 0.15, 0.25, tile.opacity];
                }
                NodeData::HitRegion(_) => {
                    // Check local state for visual feedback
                    if let Some(state) = scene.hit_region_states.get(&root_id) {
                        if state.pressed {
                            return [0.4, 0.7, 1.0, tile.opacity]; // Active blue
                        } else if state.hovered {
                            return [0.3, 0.5, 0.8, tile.opacity]; // Hover blue
                        }
                    }
                    return [0.2, 0.3, 0.5, tile.opacity]; // Default hit region
                }
                NodeData::StaticImage(_) => {
                    // Tile background for image tiles: near-black with slight tint
                    return [0.05, 0.05, 0.05, tile.opacity];
                }
            }
        }
        [0.1, 0.1, 0.2, tile.opacity]
    }

    /// Render a node and its children within a tile.
    fn render_node(
        &self,
        node_id: SceneId,
        tile: &Tile,
        scene: &SceneGraph,
        vertices: &mut Vec<RectVertex>,
        sw: f32,
        sh: f32,
    ) {
        let node = match scene.nodes.get(&node_id) {
            Some(n) => n,
            None => return,
        };

        match &node.data {
            NodeData::SolidColor(sc) => {
                let verts = rect_vertices(
                    tile.bounds.x + sc.bounds.x,
                    tile.bounds.y + sc.bounds.y,
                    sc.bounds.width,
                    sc.bounds.height,
                    sw,
                    sh,
                    sc.color.to_array(),
                );
                vertices.extend_from_slice(&verts);
            }
            NodeData::TextMarkdown(tm) => {
                // For the vertical slice, render text as a colored rectangle
                // (actual text rendering deferred to post-vertical-slice)
                let bg = tm.background.unwrap_or(Rgba::new(0.15, 0.15, 0.25, 1.0));
                let verts = rect_vertices(
                    tile.bounds.x + tm.bounds.x,
                    tile.bounds.y + tm.bounds.y,
                    tm.bounds.width,
                    tm.bounds.height,
                    sw,
                    sh,
                    bg.to_array(),
                );
                vertices.extend_from_slice(&verts);

                // Render a smaller text area rectangle in the text color
                // to indicate text content is present
                let text_margin = 8.0;
                if tm.bounds.width > text_margin * 2.0 && tm.bounds.height > text_margin * 2.0 {
                    let verts = rect_vertices(
                        tile.bounds.x + tm.bounds.x + text_margin,
                        tile.bounds.y + tm.bounds.y + text_margin,
                        tm.bounds.width - text_margin * 2.0,
                        (tm.font_size_px * 1.2).min(tm.bounds.height - text_margin * 2.0),
                        sw,
                        sh,
                        tm.color.to_array(),
                    );
                    vertices.extend_from_slice(&verts);
                }
            }
            NodeData::HitRegion(hr) => {
                // Render hit region with local state feedback
                let color = if let Some(state) = scene.hit_region_states.get(&node_id) {
                    if state.pressed {
                        [0.4, 0.7, 1.0, 1.0]
                    } else if state.hovered {
                        [0.3, 0.5, 0.8, 1.0]
                    } else {
                        [0.2, 0.3, 0.5, 1.0]
                    }
                } else {
                    [0.2, 0.3, 0.5, 1.0]
                };

                let verts = rect_vertices(
                    tile.bounds.x + hr.bounds.x,
                    tile.bounds.y + hr.bounds.y,
                    hr.bounds.width,
                    hr.bounds.height,
                    sw,
                    sh,
                    color,
                );
                vertices.extend_from_slice(&verts);
            }
            NodeData::StaticImage(img) => {
                // Render a representative colored quad for the image bounds.
                //
                // Full GPU texture upload (wgpu::Texture from RGBA data) is deferred to a
                // follow-up iteration that adds a sampler pipeline. For the vertical slice this
                // placeholder renders a warm-gray background quad with a smaller accent strip
                // (mimicking the visual weight of an image) so that pixel-readback tests can
                // verify the node is composited into the frame at the correct position.
                let outer_color = [0.55_f32, 0.50, 0.45, 1.0]; // warm gray — "image placeholder"
                let verts = rect_vertices(
                    tile.bounds.x + img.bounds.x,
                    tile.bounds.y + img.bounds.y,
                    img.bounds.width,
                    img.bounds.height,
                    sw,
                    sh,
                    outer_color,
                );
                vertices.extend_from_slice(&verts);

                // Inner accent strip — a slightly brighter inset rectangle.
                let margin = 4.0_f32;
                if img.bounds.width > margin * 2.0 && img.bounds.height > margin * 2.0 {
                    let accent_color = [0.75_f32, 0.70, 0.65, 1.0];
                    let verts = rect_vertices(
                        tile.bounds.x + img.bounds.x + margin,
                        tile.bounds.y + img.bounds.y + margin,
                        img.bounds.width - margin * 2.0,
                        img.bounds.height - margin * 2.0,
                        sw,
                        sh,
                        accent_color,
                    );
                    vertices.extend_from_slice(&verts);
                }
            }
        }

        // Render children
        for child_id in &node.children {
            self.render_node(*child_id, tile, scene, vertices, sw, sh);
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum CompositorError {
    #[error("no suitable GPU adapter found")]
    NoAdapter,
    #[error("failed to create device: {0}")]
    DeviceCreation(String),
}

// Make buffer init descriptor available
use wgpu::util::DeviceExt;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::surface::HeadlessSurface;
    use tze_hud_scene::graph::SceneGraph;

    /// Convenience: build a minimal scene with one tile containing the given node.
    fn scene_with_node(node: Node) -> SceneGraph {
        let mut scene = SceneGraph::new(256.0, 256.0);
        let tab_id = scene.create_tab("test", 0).unwrap();
        let lease_id = scene.grant_lease("test", 60_000, vec![]);
        let tile_id = scene
            .create_tile(tab_id, "test", lease_id, Rect::new(0.0, 0.0, 256.0, 256.0), 1)
            .unwrap();
        scene.set_tile_root(tile_id, node).unwrap();
        scene
    }

    /// Create a headless compositor and surface pair for testing.
    async fn make_compositor_and_surface(w: u32, h: u32) -> (Compositor, HeadlessSurface) {
        let compositor = Compositor::new_headless(w, h).await.expect("headless compositor");
        let surface = HeadlessSurface::new(&compositor.device, w, h);
        (compositor, surface)
    }

    #[tokio::test]
    async fn test_static_image_node_renders_placeholder_quad() {
        // The static image placeholder renders a warm-gray outer quad ~[0.55, 0.50, 0.45].
        // In sRGB output the linear values are gamma-compressed.
        // We just verify that *some* non-background pixels appear in the expected warm range.
        let (mut compositor, surface) = make_compositor_and_surface(256, 256).await;

        let (img_data, hash) = {
            let data: Vec<u8> = (0..8 * 8).flat_map(|_| [255u8, 0, 0, 255]).collect();
            let sum: u64 = data.iter().map(|b| *b as u64).sum();
            (data, format!("{:016x}", sum))
        };

        let node = Node {
            id: SceneId::new(),
            children: vec![],
            data: NodeData::StaticImage(StaticImageNode {
                image_data: img_data,
                width: 8,
                height: 8,
                content_hash: hash,
                fit_mode: ImageFitMode::Contain,
                bounds: Rect::new(0.0, 0.0, 256.0, 256.0),
            }),
        };

        let scene = scene_with_node(node);
        compositor.render_frame(&scene, &surface);
        compositor.queue.submit(std::iter::empty());
        compositor.device.poll(wgpu::Maintain::Wait);

        let pixels = surface.read_pixels(&compositor.device);
        // The background clear color is ~[0.05, 0.05, 0.1] in linear; tile bg is [0.05,0.05,0.05].
        // The placeholder outer quad is warm gray [0.55, 0.50, 0.45] in linear.
        // In sRGB this is approximately [198, 188, 176]. We look for pixels brighter than 150 in
        // all three channels to confirm the quad was rendered (not just the dark background).
        let any_warm_pixel = pixels
            .chunks(4)
            .any(|p| p[0] > 150 && p[1] > 140 && p[2] > 130);
        assert!(any_warm_pixel, "expected warm-gray placeholder pixels from StaticImageNode");
    }

    #[tokio::test]
    async fn test_static_image_node_composited_with_other_nodes() {
        // Render a scene with both a SolidColor node and a StaticImage node in adjacent tiles.
        let (mut compositor, surface) = make_compositor_and_surface(512, 256).await;

        let mut scene = SceneGraph::new(512.0, 256.0);
        let tab_id = scene.create_tab("test", 0).unwrap();
        let lease_id = scene.grant_lease("agent", 60_000, vec![]);

        // Left tile: red solid color
        let left_tile_id = scene
            .create_tile(tab_id, "agent", lease_id, Rect::new(0.0, 0.0, 256.0, 256.0), 1)
            .unwrap();
        scene.set_tile_root(left_tile_id, Node {
            id: SceneId::new(),
            children: vec![],
            data: NodeData::SolidColor(SolidColorNode {
                color: Rgba::new(1.0, 0.0, 0.0, 1.0),
                bounds: Rect::new(0.0, 0.0, 256.0, 256.0),
            }),
        }).unwrap();

        // Right tile: static image
        let (img_data, hash) = {
            let data: Vec<u8> = (0..8 * 8).flat_map(|_| [0u8, 255, 0, 255]).collect();
            (data, "green-hash".to_string())
        };
        let right_tile_id = scene
            .create_tile(tab_id, "agent", lease_id, Rect::new(256.0, 0.0, 256.0, 256.0), 2)
            .unwrap();
        scene.set_tile_root(right_tile_id, Node {
            id: SceneId::new(),
            children: vec![],
            data: NodeData::StaticImage(StaticImageNode {
                image_data: img_data,
                width: 8,
                height: 8,
                content_hash: hash,
                fit_mode: ImageFitMode::Cover,
                bounds: Rect::new(0.0, 0.0, 256.0, 256.0),
            }),
        }).unwrap();

        compositor.render_frame(&scene, &surface);
        compositor.device.poll(wgpu::Maintain::Wait);

        let pixels = surface.read_pixels(&compositor.device);
        assert_eq!(pixels.len(), 512 * 256 * 4, "pixel buffer size mismatch");
        // Just verify the frame completed without panic and returned the expected buffer size.
    }
}
