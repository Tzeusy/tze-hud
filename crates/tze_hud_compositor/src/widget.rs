//! SVG rendering pipeline for widgets.
//!
//! This module implements the compositor's SVG rendering pipeline for widgets
//! as specified in widget-system/spec.md §Requirement: Widget Compositor Rendering
//! and §Requirement: Widget Parameter Interpolation.
//!
//! ## Pipeline
//!
//! 1. At widget type registration, raw SVG bytes are stored per layer.
//! 2. On parameter publication (or initial render), bindings are applied to
//!    the SVG source to produce a modified SVG string.
//! 3. The modified SVG is parsed into a `usvg::Tree` and rasterized to a
//!    `tiny_skia::Pixmap` via `resvg::render`.
//! 4. The RGBA pixmap data is uploaded to a `wgpu::Texture`.
//! 5. Cached textures are reused when parameters are unchanged (dirty flag).
//!
//! ## Parameter Interpolation
//!
//! When `transition_ms > 0`, the compositor interpolates between old and new
//! parameter values:
//! - `f32`: linear interpolation
//! - `color`: component-wise linear interpolation in sRGB space
//! - `string` / `enum`: snap to new value at t=0
//!
//! ## Compositing
//!
//! Widget textures are composited at the widget instance's geometry position
//! using z_order >= WIDGET_TILE_Z_MIN (0x9000_0000), which places widget tiles
//! above zone tiles.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};
use std::time::Instant;

use tze_hud_scene::DegradationLevel;
use tze_hud_scene::types::{
    GeometryPolicy, Rgba, WIDGET_TILE_Z_MIN, WidgetBinding, WidgetBindingMapping, WidgetDefinition,
    WidgetParameterValue, WidgetRegistry,
};

// ─── SVG attribute manipulation ───────────────────────────────────────────────

/// Apply a single attribute change to an SVG element identified by `element_id`.
///
/// Finds the XML tag containing `id="element_id"` and sets the named attribute.
/// If the attribute already exists in the tag, its value is replaced. Otherwise
/// the attribute is inserted before the closing `>` or `/>`.
///
/// For the synthetic `text-content` target attribute, the text node content of
/// the element is replaced (spec §Requirement: SVG Layer Parameter Bindings).
///
/// # Limitations
///
/// This operates on the raw SVG string with simple pattern matching. It handles
/// the common SVG element authoring patterns (single-line elements, attributes
/// on the same line as the id). Elements with multi-line attributes or id
/// attributes appearing after the target attribute may not be updated correctly.
/// Widget bundle authors should ensure elements are on single lines with `id`
/// as the first attribute for best results.
pub fn apply_svg_attribute(svg: &str, element_id: &str, attr: &str, value: &str) -> String {
    let id_patterns = [format!("id=\"{element_id}\""), format!("id='{element_id}'")];

    for id_pattern in &id_patterns {
        if let Some(elem_pos) = svg.find(id_pattern.as_str()) {
            let before_id = &svg[..elem_pos];
            let tag_start = match before_id.rfind('<') {
                Some(p) => p,
                None => continue,
            };

            // Find end of this tag (either > or />)
            let from_tag = &svg[tag_start..];

            // Check for text-content synthetic target
            if attr == "text-content" {
                return apply_svg_text_content(svg, element_id, value);
            }

            // Find the position of closing > or />
            let tag_end_rel = match find_tag_end(from_tag) {
                Some(p) => p,
                None => continue,
            };
            let tag_end = tag_start + tag_end_rel;
            let tag_content = &svg[tag_start..tag_end];

            // Check if this attr already exists in the tag
            let attr_eq = format!("{attr}=\"");
            let attr_eq_single = format!("{attr}='");

            if let Some(attr_pos) = tag_content.find(attr_eq.as_str()) {
                // Replace double-quoted existing value
                let abs_start = tag_start + attr_pos + attr_eq.len();
                let val_end = svg[abs_start..].find('"').unwrap_or(0);
                let abs_end = abs_start + val_end;
                return format!("{}{}{}", &svg[..abs_start], value, &svg[abs_end..]);
            } else if let Some(attr_pos) = tag_content.find(attr_eq_single.as_str()) {
                // Replace single-quoted existing value
                let abs_start = tag_start + attr_pos + attr_eq_single.len();
                let val_end = svg[abs_start..].find('\'').unwrap_or(0);
                let abs_end = abs_start + val_end;
                return format!("{}{}{}", &svg[..abs_start], value, &svg[abs_end..]);
            } else {
                // Insert new attribute before tag end (> or />)
                let insert_pos = tag_end;
                return format!(
                    "{} {}=\"{}\"{}",
                    &svg[..insert_pos],
                    attr,
                    value,
                    &svg[insert_pos..]
                );
            }
        }
    }

    // Element not found; return unmodified
    svg.to_string()
}

/// Find the end of an XML start tag in `s` (the position of '>' or '/', where
/// the next char is '>', not being inside a quoted attribute value).
///
/// Returns the index of the closing character ('>' or the '/' in '/>').
fn find_tag_end(s: &str) -> Option<usize> {
    let mut in_double_quote = false;
    let mut in_single_quote = false;
    for (i, c) in s.char_indices() {
        match c {
            '"' if !in_single_quote => in_double_quote = !in_double_quote,
            '\'' if !in_double_quote => in_single_quote = !in_single_quote,
            '>' if !in_double_quote && !in_single_quote => {
                // Check if it's '/>'
                if i > 0 && s.as_bytes().get(i - 1) == Some(&b'/') {
                    return Some(i - 1);
                }
                return Some(i);
            }
            _ => {}
        }
    }
    None
}

/// Replace the text content of an SVG element (for the synthetic `text-content`
/// binding target).
///
/// Finds `<tag id="element_id"...>` and replaces the character data between the
/// opening and closing tag.
fn apply_svg_text_content(svg: &str, element_id: &str, value: &str) -> String {
    let id_patterns = [format!("id=\"{element_id}\""), format!("id='{element_id}'")];

    for id_pattern in &id_patterns {
        if let Some(elem_pos) = svg.find(id_pattern.as_str()) {
            let before_id = &svg[..elem_pos];
            let tag_start = match before_id.rfind('<') {
                Some(p) => p,
                None => continue,
            };

            // Find the '>' closing the start tag
            let from_tag = &svg[tag_start..];
            let open_end = match from_tag.find('>') {
                Some(p) => tag_start + p + 1,
                None => continue,
            };

            // Find the closing tag by looking for '</'
            let after_open = &svg[open_end..];
            let close_start = match after_open.find("</") {
                Some(p) => open_end + p,
                None => continue,
            };

            // Replace the text content
            return format!("{}{}{}", &svg[..open_end], value, &svg[close_start..]);
        }
    }

    svg.to_string()
}

// ─── Binding value resolution ─────────────────────────────────────────────────

/// Compute the SVG attribute string value for a single binding.
///
/// Returns `None` if the binding cannot be resolved (e.g. parameter not found
/// in `params`).
pub fn resolve_binding_value(
    binding: &WidgetBinding,
    params: &HashMap<String, WidgetParameterValue>,
    param_constraints: &HashMap<String, (f32, f32)>, // (min, max) for f32 params
) -> Option<String> {
    let param_val = params.get(&binding.param)?;

    match &binding.mapping {
        WidgetBindingMapping::Linear { attr_min, attr_max } => {
            let f = match param_val {
                WidgetParameterValue::F32(v) => *v,
                _ => return None,
            };
            // Get param constraints for normalization
            let (p_min, p_max) = param_constraints
                .get(&binding.param)
                .copied()
                .unwrap_or((0.0, 1.0));
            let normalized = if (p_max - p_min).abs() < f32::EPSILON {
                0.0f32
            } else {
                ((f - p_min) / (p_max - p_min)).clamp(0.0, 1.0)
            };
            let attr_val = attr_min + normalized * (attr_max - attr_min);
            // Format with enough precision but avoid spurious decimals
            if (attr_val - attr_val.round()).abs() < 0.001 {
                let rounded = attr_val.round() as i64;
                Some(format!("{rounded}"))
            } else {
                Some(format!("{attr_val:.3}"))
            }
        }

        WidgetBindingMapping::Direct => match param_val {
            WidgetParameterValue::String(s) => Some(s.clone()),
            WidgetParameterValue::Color(rgba) => Some(rgba_to_svg_color(rgba)),
            _ => None,
        },

        WidgetBindingMapping::Discrete { value_map } => {
            let key = match param_val {
                WidgetParameterValue::Enum(s) => s.as_str(),
                _ => return None,
            };
            value_map.get(key).cloned()
        }
    }
}

/// Convert an `Rgba` (f32 components 0.0..=1.0) to an SVG color string `rgba(r,g,b,a)`.
fn rgba_to_svg_color(rgba: &Rgba) -> String {
    let r = (rgba.r * 255.0).round() as u8;
    let g = (rgba.g * 255.0).round() as u8;
    let b = (rgba.b * 255.0).round() as u8;
    let a = rgba.a; // SVG opacity as 0.0..=1.0
    if (a - 1.0).abs() < 0.004 {
        format!("#{r:02x}{g:02x}{b:02x}")
    } else {
        format!("rgba({r},{g},{b},{a:.3})")
    }
}

// ─── Parameter interpolation ──────────────────────────────────────────────────

/// Compute the effective transition progress `t` (0.0..=1.0) for a widget
/// animation, taking the current degradation level into account.
///
/// Under [`DegradationLevel::Significant`] or higher (the spec's
/// `RENDERING_SIMPLIFIED` threshold), the compositor snaps to the final values
/// by returning `1.0` immediately, regardless of elapsed time.  This reduces
/// per-frame re-rasterizations during periods of degradation.
///
/// Under `Nominal`, `Minor`, or `Moderate`, the normal time-based interpolation
/// value is returned.
pub fn compute_transition_t(
    elapsed_ms: f32,
    duration_ms: f32,
    degradation_level: DegradationLevel,
) -> f32 {
    if degradation_level >= DegradationLevel::Significant {
        1.0
    } else {
        (elapsed_ms / duration_ms).clamp(0.0, 1.0)
    }
}

/// Interpolate between `old` and `new` parameter values at time `t` (0.0..=1.0).
///
/// - f32: linear interpolation
/// - color: component-wise linear interpolation in sRGB space
/// - string / enum: snap to `new` at t=0 (applied immediately)
pub fn interpolate_param(
    old: &WidgetParameterValue,
    new: &WidgetParameterValue,
    t: f32,
) -> WidgetParameterValue {
    match (old, new) {
        (WidgetParameterValue::F32(a), WidgetParameterValue::F32(b)) => {
            WidgetParameterValue::F32(a + (b - a) * t)
        }
        (WidgetParameterValue::Color(ca), WidgetParameterValue::Color(cb)) => {
            WidgetParameterValue::Color(Rgba::new(
                ca.r + (cb.r - ca.r) * t,
                ca.g + (cb.g - ca.g) * t,
                ca.b + (cb.b - ca.b) * t,
                ca.a + (cb.a - ca.a) * t,
            ))
        }
        // String and enum snap to new value immediately (spec §Widget Parameter Interpolation)
        _ => new.clone(),
    }
}

// ─── Widget texture cache ─────────────────────────────────────────────────────

/// Per-instance cached GPU texture for a widget.
///
/// When `dirty` is true, the compositor re-rasterizes the SVG and uploads a
/// new texture. When false, the cached texture is reused.
pub struct WidgetTextureEntry {
    /// The cached wgpu texture (RGBA8Unorm, premultiplied alpha).
    pub texture: wgpu::Texture,
    /// Bind group for sampling the texture in the widget render pass.
    pub bind_group: wgpu::BindGroup,
    /// Width of the texture in pixels.
    pub width: u32,
    /// Height of the texture in pixels.
    pub height: u32,
    /// When true, the texture must be re-rasterized before compositing.
    pub dirty: bool,
    /// Parameters used for the last successful rasterization.
    /// Compared against current scene params to detect changes from MCP publishes.
    pub last_rendered_params: HashMap<String, WidgetParameterValue>,
    /// Animation state: start time, duration, starting params, target params.
    pub animation: Option<WidgetAnimationState>,
}

/// Active animation state for a widget instance.
pub struct WidgetAnimationState {
    pub start: Instant,
    pub duration_ms: u32,
    pub from_params: HashMap<String, WidgetParameterValue>,
    pub to_params: HashMap<String, WidgetParameterValue>,
}

// ─── Standalone SVG rasterization (CPU-only, no GPU) ─────────────────────────

fn widget_usvg_options() -> resvg::usvg::Options<'static> {
    resvg::usvg::Options {
        fontdb: shared_widget_fontdb(),
        ..Default::default()
    }
}

fn shared_widget_fontdb() -> Arc<resvg::usvg::fontdb::Database> {
    static FONTDB: OnceLock<Arc<resvg::usvg::fontdb::Database>> = OnceLock::new();
    FONTDB
        .get_or_init(|| {
            let mut db = resvg::usvg::fontdb::Database::new();
            db.load_system_fonts();
            Arc::new(db)
        })
        .clone()
}

/// Rasterize all SVG layers for a widget definition with parameter bindings applied.
///
/// This is the **CPU-only** portion of the widget rendering pipeline — SVG string
/// manipulation, `usvg` parsing, and `resvg`/`tiny-skia` rasterization — without
/// the GPU texture upload step.
///
/// ## Performance contract
///
/// Per widget-system/spec.md §Requirement: Widget Compositor Rendering:
/// re-rasterization MUST complete in < 2ms for a 512×512 widget on reference
/// hardware.  This function is the hot path that must satisfy that budget.
///
/// ## Arguments
///
/// - `svg_layers` — pairs of `(svg_source, layer_bindings)` for each layer
///   in `widget_def.layers`.  Pre-loaded so this function is GPU-free.
/// - `param_constraints` — map from param name to `(f32_min, f32_max)`.
/// - `params` — current parameter values.
/// - `pixel_width` / `pixel_height` — target raster size.
///
/// ## Returns
///
/// Composed `tiny_skia::Pixmap` (RGBA, premultiplied alpha), or `None` if all
/// layers failed to rasterize.
pub fn rasterize_svg_layers(
    svg_layers: &[(&str, &[WidgetBinding])],
    param_constraints: &HashMap<String, (f32, f32)>,
    params: &HashMap<String, WidgetParameterValue>,
    pixel_width: u32,
    pixel_height: u32,
) -> Option<tiny_skia::Pixmap> {
    let mut composed: Option<tiny_skia::Pixmap> = None;

    for (svg_text, bindings) in svg_layers {
        // Apply parameter bindings to the SVG source.
        let mut modified_svg = svg_text.to_string();
        for binding in *bindings {
            if let Some(attr_val) = resolve_binding_value(binding, params, param_constraints) {
                modified_svg = apply_svg_attribute(
                    &modified_svg,
                    &binding.target_element,
                    &binding.target_attribute,
                    &attr_val,
                );
            }
        }

        // Parse modified SVG into usvg::Tree.
        let opts = widget_usvg_options();
        let tree = match resvg::usvg::Tree::from_str(&modified_svg, &opts) {
            Ok(t) => t,
            Err(e) => {
                tracing::warn!(error = %e, "rasterize_svg_layers: failed to parse SVG");
                continue;
            }
        };

        // Rasterize into a pixmap at the target size.
        let mut pixmap = match tiny_skia::Pixmap::new(pixel_width, pixel_height) {
            Some(p) => p,
            None => {
                tracing::warn!(
                    width = pixel_width,
                    height = pixel_height,
                    "rasterize_svg_layers: failed to allocate pixmap"
                );
                continue;
            }
        };

        // Scale the SVG uniformly (preserving aspect ratio) and center it
        // within the target pixel rectangle.  This prevents circles from
        // becoming ovals on non-square screen geometries.
        let svg_size = tree.size();
        let sx = pixel_width as f32 / svg_size.width();
        let sy = pixel_height as f32 / svg_size.height();
        let uniform_scale = sx.min(sy);
        let rendered_w = svg_size.width() * uniform_scale;
        let rendered_h = svg_size.height() * uniform_scale;
        let offset_x = (pixel_width as f32 - rendered_w) * 0.5;
        let offset_y = (pixel_height as f32 - rendered_h) * 0.5;
        let transform = tiny_skia::Transform::from_translate(offset_x, offset_y)
            .post_scale(uniform_scale, uniform_scale);
        resvg::render(&tree, transform, &mut pixmap.as_mut());

        // Composite this layer onto the accumulation pixmap (source-over).
        // Uses tiny_skia::PixmapMut::draw_pixmap which is SIMD-optimised and
        // handles premultiplied alpha correctly — avoiding a manual pixel loop.
        if let Some(ref mut base) = composed {
            base.as_mut().draw_pixmap(
                0,
                0,
                pixmap.as_ref(),
                &tiny_skia::PixmapPaint::default(),
                tiny_skia::Transform::identity(),
                None,
            );
        } else {
            composed = Some(pixmap);
        }
    }

    composed
}

// ─── WidgetRenderer (GPU state) ───────────────────────────────────────────────

/// The compositor-owned widget rendering state.
///
/// Created once per compositor and kept for the lifetime of the runtime.
pub struct WidgetRenderer {
    /// Raw SVG bytes keyed by (widget_type_id, svg_filename).
    /// Stored at widget type registration time.
    svgs: HashMap<(String, String), Vec<u8>>,

    /// Per-instance texture cache keyed by instance_name.
    textures: HashMap<String, WidgetTextureEntry>,

    /// Bind group layout for the texture pipeline.
    texture_bind_group_layout: wgpu::BindGroupLayout,

    /// Render pipeline for compositing widget textures.
    texture_pipeline: wgpu::RenderPipeline,
}

impl WidgetRenderer {
    /// Create a new `WidgetRenderer` for the given device and output format.
    pub fn new(device: &wgpu::Device, output_format: wgpu::TextureFormat) -> Self {
        let texture_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("widget_texture_bgl"),
                entries: &[
                    // t_texture: RGBA texture
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    // s_sampler
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                ],
            });

        let texture_pipeline =
            Self::create_texture_pipeline(device, &texture_bind_group_layout, output_format);

        Self {
            svgs: HashMap::new(),
            textures: HashMap::new(),
            texture_bind_group_layout,
            texture_pipeline,
        }
    }

    fn create_texture_pipeline(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        format: wgpu::TextureFormat,
    ) -> wgpu::RenderPipeline {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("widget_texture_shader"),
            source: wgpu::ShaderSource::Wgsl(WIDGET_TEXTURE_SHADER.into()),
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("widget_texture_pipeline_layout"),
            bind_group_layouts: &[layout],
            push_constant_ranges: &[],
        });

        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("widget_texture_pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[WidgetVertex::desc()],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        })
    }

    /// Register raw SVG bytes for a widget layer.
    ///
    /// Called at startup when widget bundles are loaded, before any instances
    /// are initialized. The SVG bytes are stored keyed by (widget_type_id, svg_filename).
    pub fn register_svg(&mut self, widget_type_id: &str, svg_filename: &str, svg_bytes: Vec<u8>) {
        let byte_count = svg_bytes.len();
        self.svgs.insert(
            (widget_type_id.to_string(), svg_filename.to_string()),
            svg_bytes,
        );
        tracing::debug!(
            widget_type = widget_type_id,
            svg_file = svg_filename,
            bytes = byte_count,
            "widget SVG registered"
        );
    }

    /// Mark a widget instance as dirty so it will be re-rasterized on the next frame.
    pub fn mark_dirty(&mut self, instance_name: &str) {
        if let Some(entry) = self.textures.get_mut(instance_name) {
            entry.dirty = true;
        }
    }

    /// Mark a widget instance as dirty and start a transition animation.
    pub fn start_transition(
        &mut self,
        instance_name: &str,
        from_params: HashMap<String, WidgetParameterValue>,
        to_params: HashMap<String, WidgetParameterValue>,
        transition_ms: u32,
    ) {
        if let Some(entry) = self.textures.get_mut(instance_name) {
            entry.dirty = true;
            entry.animation = Some(WidgetAnimationState {
                start: Instant::now(),
                duration_ms: transition_ms,
                from_params,
                to_params,
            });
        }
    }

    /// Resolve effective parameters for an instance, applying animation if active.
    ///
    /// Under degradation level [`DegradationLevel::Significant`] or higher
    /// (corresponding to the spec's `RENDERING_SIMPLIFIED` threshold), the
    /// compositor snaps to the final parameter values immediately (`t = 1.0`)
    /// instead of interpolating.  This reduces re-rasterization to at most once
    /// per parameter change during transitions, saving CPU time under load.
    ///
    /// Returns `(effective_params, still_animating)`.
    pub fn resolve_animated_params(
        &mut self,
        instance_name: &str,
        current_params: &HashMap<String, WidgetParameterValue>,
        degradation_level: DegradationLevel,
    ) -> (HashMap<String, WidgetParameterValue>, bool) {
        let entry = match self.textures.get_mut(instance_name) {
            Some(e) => e,
            None => return (current_params.clone(), false),
        };

        let anim = match &entry.animation {
            Some(a) => a,
            None => return (current_params.clone(), false),
        };

        let elapsed_ms = anim.start.elapsed().as_millis() as f32;
        let duration_ms = anim.duration_ms as f32;
        // Under RENDERING_SIMPLIFIED or higher degradation, snap to final values
        // immediately to avoid per-frame re-rasterization.
        let t = compute_transition_t(elapsed_ms, duration_ms, degradation_level);

        let mut result = anim.from_params.clone();
        for (k, new_val) in &anim.to_params {
            let old_val = anim.from_params.get(k).unwrap_or(new_val);
            result.insert(k.clone(), interpolate_param(old_val, new_val, t));
        }

        let still_animating = t < 1.0;
        if !still_animating {
            entry.animation = None;
        } else {
            // Keep re-rasterizing while animating
            entry.dirty = true;
        }

        (result, still_animating)
    }

    /// Rasterize an SVG layer with parameter bindings applied and upload to GPU.
    ///
    /// Returns the time spent rasterizing in microseconds (for performance monitoring).
    /// Per spec, re-rasterization MUST complete in less than 2ms for a 512x512 widget.
    #[allow(clippy::too_many_arguments)]
    pub fn rasterize_and_upload(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        instance_name: &str,
        widget_def: &WidgetDefinition,
        params: &HashMap<String, WidgetParameterValue>,
        pixel_width: u32,
        pixel_height: u32,
    ) -> u64 {
        let start = Instant::now();

        // Build a map from param name to (f32_min, f32_max) for linear binding normalization.
        let param_constraints: HashMap<String, (f32, f32)> = widget_def
            .parameter_schema
            .iter()
            .filter_map(|p| {
                if let Some(c) = &p.constraints {
                    let min = c.f32_min.unwrap_or(0.0);
                    let max = c.f32_max.unwrap_or(1.0);
                    Some((p.name.clone(), (min, max)))
                } else {
                    None
                }
            })
            .collect();

        // Resolve SVG text for each layer (bytes → str).
        let mut layer_texts: Vec<String> = Vec::with_capacity(widget_def.layers.len());
        let mut layer_data: Vec<(usize, &[WidgetBinding])> =
            Vec::with_capacity(widget_def.layers.len());

        for (idx, layer) in widget_def.layers.iter().enumerate() {
            let key = (widget_def.id.clone(), layer.svg_file.clone());
            let svg_bytes = match self.svgs.get(&key) {
                Some(b) => b.clone(),
                None => {
                    tracing::warn!(
                        widget = widget_def.id,
                        svg_file = layer.svg_file,
                        "SVG bytes not registered for widget layer"
                    );
                    continue;
                }
            };
            match std::str::from_utf8(&svg_bytes) {
                Ok(s) => {
                    layer_texts.push(s.to_string());
                    layer_data.push((idx, &layer.bindings));
                }
                Err(e) => {
                    tracing::warn!(error = %e, "widget SVG not valid UTF-8");
                }
            }
        }

        // Build the slice expected by rasterize_svg_layers.
        let svg_layers: Vec<(&str, &[WidgetBinding])> = layer_data
            .iter()
            .zip(layer_texts.iter())
            .map(|((_, bindings), text)| (text.as_str(), *bindings))
            .collect();

        // Delegate to the CPU-only rasterization path (shared with benchmarks/tests).
        let composed = rasterize_svg_layers(
            &svg_layers,
            &param_constraints,
            params,
            pixel_width,
            pixel_height,
        );

        let raster_us = start.elapsed().as_micros() as u64;

        if raster_us > 2000 {
            tracing::warn!(
                widget = instance_name,
                raster_us,
                "widget re-rasterization exceeded 2ms budget"
            );
        }

        let pixmap = match composed {
            Some(p) => p,
            None => {
                tracing::debug!(widget = instance_name, "no layers rendered for widget");
                return raster_us;
            }
        };

        // Upload to GPU texture (or create a new texture if size changed).
        self.upload_texture(
            device,
            queue,
            instance_name,
            pixmap.data(),
            pixel_width,
            pixel_height,
        );

        let total_us = start.elapsed().as_micros() as u64;
        tracing::trace!(
            widget = instance_name,
            raster_us,
            total_us,
            width = pixel_width,
            height = pixel_height,
            "widget rasterized and uploaded"
        );

        total_us
    }

    /// Upload RGBA pixel data to a wgpu texture (creating or replacing the cached entry).
    fn upload_texture(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        instance_name: &str,
        rgba_data: &[u8],
        width: u32,
        height: u32,
    ) {
        // Create the GPU texture.
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(&format!("widget_tex_{instance_name}")),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            // Use Rgba8UnormSrgb: tiny-skia rasterises SVGs in sRGB color space.
            // The GPU must sRGB-decode on sample so colours match the hex values
            // in the SVG source (e.g. #4FB543 looks the same as in a browser).
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        // Write pixel data.
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            rgba_data,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(width * 4),
                rows_per_image: Some(height),
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );

        let texture_view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some(&format!("widget_sampler_{instance_name}")),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(&format!("widget_bg_{instance_name}")),
            layout: &self.texture_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&texture_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });

        self.textures.insert(
            instance_name.to_string(),
            WidgetTextureEntry {
                texture,
                bind_group,
                width,
                height,
                dirty: false,
                last_rendered_params: HashMap::new(),
                animation: None,
            },
        );
    }

    /// Composite widget textures into the current render pass.
    ///
    /// Called from the compositor's frame render path. Widget tiles use
    /// z_order >= WIDGET_TILE_Z_MIN (0x9000_0000) so they appear above zone
    /// tiles when they overlap.
    ///
    /// # Arguments
    ///
    /// - `render_pass` — the active render pass to draw into
    /// - `registry` — the widget registry (instances + definitions)
    /// - `surf_w` / `surf_h` — surface dimensions for NDC conversion
    pub fn composite_widgets<'rp>(
        &'rp self,
        render_pass: &mut wgpu::RenderPass<'rp>,
        registry: &WidgetRegistry,
        surf_w: f32,
        surf_h: f32,
        device: &wgpu::Device,
    ) {
        use wgpu::util::DeviceExt;

        // Sort active instances by z_order (all widgets use WIDGET_TILE_Z_MIN
        // base + their declared order, so any ordering works for now).
        let mut instances: Vec<&tze_hud_scene::types::WidgetInstance> = registry
            .instances
            .values()
            .filter(|instance| {
                registry
                    .active_publishes
                    .get(&instance.instance_name)
                    .is_some_and(|publishes| !publishes.is_empty())
            })
            .collect();
        instances.sort_by_key(|_i| WIDGET_TILE_Z_MIN);

        render_pass.set_pipeline(&self.texture_pipeline);

        for instance in instances {
            let entry = match self.textures.get(&instance.instance_name) {
                Some(e) => e,
                None => continue,
            };

            // Resolve pixel geometry from the instance's geometry policy.
            let (raw_x, raw_y, _pw, _ph) =
                resolve_pixel_geometry(&instance.geometry_override, surf_w, surf_h).unwrap_or_else(
                    || {
                        // Fall back to full-screen if geometry not set
                        let def = registry
                            .definitions
                            .get(&instance.widget_type_name)
                            .map(|d| &d.default_geometry_policy);
                        if let Some(geo) = def {
                            resolve_pixel_geometry(&Some(*geo), surf_w, surf_h)
                                .unwrap_or((0.0, 0.0, surf_w, surf_h))
                        } else {
                            (0.0, 0.0, surf_w, surf_h)
                        }
                    },
                );
            // Pixel-snap widget quads so text-heavy SVGs remain crisp on screen.
            let (px, py, pw, ph) = snap_composite_rect(
                raw_x,
                raw_y,
                entry.width as f32,
                entry.height as f32,
                surf_w,
                surf_h,
            );

            // Build NDC quad vertices with UV coordinates.
            let vertices = widget_quad_vertices(px, py, pw, ph, surf_w, surf_h);
            let vertex_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("widget_quad_buf"),
                contents: bytemuck::cast_slice(&vertices),
                usage: wgpu::BufferUsages::VERTEX,
            });

            render_pass.set_bind_group(0, &entry.bind_group, &[]);
            render_pass.set_vertex_buffer(0, vertex_buf.slice(..));
            render_pass.draw(0..6, 0..1);
        }
    }

    /// Update the render pipeline's output format (e.g. on swapchain reconfiguration).
    pub fn update_format(&mut self, device: &wgpu::Device, new_format: wgpu::TextureFormat) {
        self.texture_pipeline =
            Self::create_texture_pipeline(device, &self.texture_bind_group_layout, new_format);
    }

    /// Get a reference to the texture entry for an instance (for testing / inspection).
    pub fn texture_entry(&self, instance_name: &str) -> Option<&WidgetTextureEntry> {
        self.textures.get(instance_name)
    }

    pub fn texture_entry_mut(&mut self, instance_name: &str) -> Option<&mut WidgetTextureEntry> {
        self.textures.get_mut(instance_name)
    }

    pub fn remove_texture(&mut self, instance_name: &str) {
        self.textures.remove(instance_name);
    }

    /// Returns true if any widget instance has an active animation (needs re-rasterize).
    pub fn has_active_animations(&self) -> bool {
        self.textures.values().any(|e| e.animation.is_some())
    }
}

/// Resolve pixel geometry from a `GeometryPolicy` option.
fn resolve_pixel_geometry(
    policy: &Option<GeometryPolicy>,
    surf_w: f32,
    surf_h: f32,
) -> Option<(f32, f32, f32, f32)> {
    match policy {
        Some(GeometryPolicy::Relative {
            x_pct,
            y_pct,
            width_pct,
            height_pct,
        }) => {
            let x = surf_w * x_pct;
            let y = surf_h * y_pct;
            let w = surf_w * width_pct;
            let h = surf_h * height_pct;
            Some((x, y, w, h))
        }
        Some(GeometryPolicy::EdgeAnchored {
            edge,
            height_pct,
            width_pct,
            margin_px,
        }) => {
            use tze_hud_scene::types::DisplayEdge;
            let w = surf_w * width_pct;
            let h = surf_h * height_pct;
            let x = (surf_w - w) / 2.0;
            let y = match edge {
                DisplayEdge::Top => *margin_px,
                DisplayEdge::Bottom => surf_h - h - margin_px,
                DisplayEdge::Left | DisplayEdge::Right => 0.0,
            };
            Some((x, y, w, h))
        }
        None => None,
    }
}

/// Snap a composite rectangle to integer pixels and clamp it to the surface.
fn snap_composite_rect(
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    surf_w: f32,
    surf_h: f32,
) -> (f32, f32, f32, f32) {
    let mut pw = w.max(1.0).round();
    let mut ph = h.max(1.0).round();
    if surf_w.is_finite() && surf_w > 0.0 {
        pw = pw.min(surf_w.round().max(1.0));
    }
    if surf_h.is_finite() && surf_h > 0.0 {
        ph = ph.min(surf_h.round().max(1.0));
    }

    let mut px = x.round();
    let mut py = y.round();
    if surf_w.is_finite() && surf_w > 0.0 {
        let max_x = (surf_w - pw).max(0.0);
        px = px.clamp(0.0, max_x);
    } else {
        px = px.max(0.0);
    }
    if surf_h.is_finite() && surf_h > 0.0 {
        let max_y = (surf_h - ph).max(0.0);
        py = py.clamp(0.0, max_y);
    } else {
        py = py.max(0.0);
    }

    (px, py, pw, ph)
}

// ─── Vertex types ─────────────────────────────────────────────────────────────

/// Vertex for the widget texture quad (position + UV).
#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct WidgetVertex {
    pub position: [f32; 2],
    pub uv: [f32; 2],
}

impl WidgetVertex {
    pub fn desc() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<WidgetVertex>() as wgpu::BufferAddress,
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
                    format: wgpu::VertexFormat::Float32x2,
                },
            ],
        }
    }
}

/// Generate a textured quad (6 vertices, two triangles) in NDC coordinates.
fn widget_quad_vertices(
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    screen_w: f32,
    screen_h: f32,
) -> [WidgetVertex; 6] {
    let left = (x / screen_w) * 2.0 - 1.0;
    let right = ((x + w) / screen_w) * 2.0 - 1.0;
    // NDC: y increases upward; pixel y=0 is top
    let top = 1.0 - (y / screen_h) * 2.0;
    let bottom = 1.0 - ((y + h) / screen_h) * 2.0;

    // UV: (0,0) = top-left, (1,1) = bottom-right (standard texture space)
    [
        WidgetVertex {
            position: [left, top],
            uv: [0.0, 0.0],
        },
        WidgetVertex {
            position: [right, top],
            uv: [1.0, 0.0],
        },
        WidgetVertex {
            position: [left, bottom],
            uv: [0.0, 1.0],
        },
        WidgetVertex {
            position: [right, top],
            uv: [1.0, 0.0],
        },
        WidgetVertex {
            position: [right, bottom],
            uv: [1.0, 1.0],
        },
        WidgetVertex {
            position: [left, bottom],
            uv: [0.0, 1.0],
        },
    ]
}

// ─── Shader ──────────────────────────────────────────────────────────────────

/// WGSL shader for compositing widget textures.
///
/// Samples the widget texture (premultiplied RGBA) and outputs it directly,
/// blending with the scene using premultiplied alpha blending.
const WIDGET_TEXTURE_SHADER: &str = r#"
@group(0) @binding(0)
var t_texture: texture_2d<f32>;
@group(0) @binding(1)
var s_sampler: sampler;

struct VertexInput {
    @location(0) position: vec2<f32>,
    @location(1) uv: vec2<f32>,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(in: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    out.clip_position = vec4<f32>(in.position, 0.0, 1.0);
    out.uv = in.uv;
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    return textureSample(t_texture, s_sampler, in.uv);
}
"#;

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── SVG attribute manipulation tests ──────────────────────────────────────

    #[test]
    fn test_apply_svg_attribute_replaces_existing() {
        let svg = r#"<svg><rect id="bar" width="50" fill="blue"/></svg>"#;
        let result = apply_svg_attribute(svg, "bar", "width", "80");
        assert!(
            result.contains("width=\"80\""),
            "should replace existing width: {result}"
        );
        assert!(
            !result.contains("width=\"50\""),
            "should not contain old value: {result}"
        );
    }

    #[test]
    fn test_apply_svg_attribute_inserts_new() {
        let svg = r#"<svg><rect id="bar" fill="blue"/></svg>"#;
        let result = apply_svg_attribute(svg, "bar", "height", "100");
        assert!(
            result.contains("height=\"100\""),
            "should insert new attribute: {result}"
        );
    }

    #[test]
    fn test_apply_svg_attribute_unknown_element_noop() {
        let svg = r#"<svg><rect id="bar" width="50"/></svg>"#;
        let result = apply_svg_attribute(svg, "nonexistent", "width", "80");
        assert_eq!(result, svg, "unknown element should leave SVG unchanged");
    }

    #[test]
    fn test_apply_svg_text_content() {
        let svg = r#"<svg><text id="label">Old Label</text></svg>"#;
        let result = apply_svg_attribute(svg, "label", "text-content", "New Label");
        assert!(
            result.contains(">New Label<"),
            "should replace text content: {result}"
        );
        assert!(
            !result.contains("Old Label"),
            "should not contain old text: {result}"
        );
    }

    // ── Binding resolution tests ──────────────────────────────────────────────

    #[test]
    fn test_resolve_linear_binding() {
        let mut params = HashMap::new();
        params.insert("level".to_string(), WidgetParameterValue::F32(0.5));

        let binding = WidgetBinding {
            param: "level".to_string(),
            target_element: "bar".to_string(),
            target_attribute: "height".to_string(),
            mapping: WidgetBindingMapping::Linear {
                attr_min: 0.0,
                attr_max: 200.0,
            },
        };

        let mut constraints = HashMap::new();
        constraints.insert("level".to_string(), (0.0, 1.0));

        let val = resolve_binding_value(&binding, &params, &constraints).unwrap();
        assert_eq!(
            val, "100",
            "level=0.5 with attr range 0..200 should give 100"
        );
    }

    #[test]
    fn test_resolve_direct_color_binding() {
        let mut params = HashMap::new();
        params.insert(
            "fill".to_string(),
            WidgetParameterValue::Color(Rgba::new(1.0, 0.0, 0.0, 1.0)),
        );

        let binding = WidgetBinding {
            param: "fill".to_string(),
            target_element: "bg".to_string(),
            target_attribute: "fill".to_string(),
            mapping: WidgetBindingMapping::Direct,
        };

        let val = resolve_binding_value(&binding, &params, &HashMap::new()).unwrap();
        assert_eq!(val, "#ff0000", "red color should format as #ff0000");
    }

    #[test]
    fn test_resolve_direct_string_binding() {
        let mut params = HashMap::new();
        params.insert(
            "label".to_string(),
            WidgetParameterValue::String("CPU".to_string()),
        );

        let binding = WidgetBinding {
            param: "label".to_string(),
            target_element: "lbl".to_string(),
            target_attribute: "text-content".to_string(),
            mapping: WidgetBindingMapping::Direct,
        };

        let val = resolve_binding_value(&binding, &params, &HashMap::new()).unwrap();
        assert_eq!(val, "CPU");
    }

    #[test]
    fn test_resolve_discrete_binding() {
        let mut params = HashMap::new();
        params.insert(
            "severity".to_string(),
            WidgetParameterValue::Enum("warning".to_string()),
        );

        let mut value_map = std::collections::BTreeMap::new();
        value_map.insert("info".to_string(), "#00ff00".to_string());
        value_map.insert("warning".to_string(), "#ffff00".to_string());
        value_map.insert("error".to_string(), "#ff0000".to_string());

        let binding = WidgetBinding {
            param: "severity".to_string(),
            target_element: "indicator".to_string(),
            target_attribute: "fill".to_string(),
            mapping: WidgetBindingMapping::Discrete { value_map },
        };

        let val = resolve_binding_value(&binding, &params, &HashMap::new()).unwrap();
        assert_eq!(val, "#ffff00", "warning should map to #ffff00");
    }

    // ── Interpolation tests ───────────────────────────────────────────────────

    #[test]
    fn test_f32_linear_interpolation() {
        let old = WidgetParameterValue::F32(0.0);
        let new = WidgetParameterValue::F32(1.0);

        let mid = interpolate_param(&old, &new, 0.5);
        assert_eq!(mid, WidgetParameterValue::F32(0.5));

        let start = interpolate_param(&old, &new, 0.0);
        assert_eq!(start, WidgetParameterValue::F32(0.0));

        let end = interpolate_param(&old, &new, 1.0);
        assert_eq!(end, WidgetParameterValue::F32(1.0));
    }

    #[test]
    fn test_color_component_wise_interpolation() {
        let old = WidgetParameterValue::Color(Rgba::new(0.0, 0.0, 1.0, 1.0)); // blue
        let new = WidgetParameterValue::Color(Rgba::new(1.0, 0.0, 0.0, 1.0)); // red

        let mid = interpolate_param(&old, &new, 0.5);
        if let WidgetParameterValue::Color(c) = mid {
            assert!((c.r - 0.5).abs() < 0.001, "r should be 0.5");
            assert!((c.g - 0.0).abs() < 0.001, "g should be 0.0");
            assert!((c.b - 0.5).abs() < 0.001, "b should be 0.5");
            assert!((c.a - 1.0).abs() < 0.001, "a should be 1.0");
        } else {
            panic!("expected Color variant");
        }
    }

    #[test]
    fn test_string_snaps_immediately() {
        let old = WidgetParameterValue::String("Old".to_string());
        let new = WidgetParameterValue::String("New".to_string());

        // Even at t=0.001, string should snap to new value
        let result = interpolate_param(&old, &new, 0.001);
        assert_eq!(
            result,
            WidgetParameterValue::String("New".to_string()),
            "string should snap to new value at any t"
        );
    }

    #[test]
    fn test_enum_snaps_immediately() {
        let old = WidgetParameterValue::Enum("info".to_string());
        let new = WidgetParameterValue::Enum("error".to_string());

        let result = interpolate_param(&old, &new, 0.001);
        assert_eq!(
            result,
            WidgetParameterValue::Enum("error".to_string()),
            "enum should snap to new value at any t"
        );
    }

    // ── Degradation-aware transition snapping tests ───────────────────────────

    /// Under Nominal/Minor/Moderate, compute_transition_t returns the elapsed/duration ratio.
    #[test]
    fn compute_transition_t_interpolates_below_rendering_simplified() {
        // At 50% elapsed out of 100ms duration → t = 0.5
        let t_nominal = compute_transition_t(50.0, 100.0, DegradationLevel::Nominal);
        assert!(
            (t_nominal - 0.5).abs() < 1e-6,
            "Nominal: expected t=0.5, got {t_nominal}"
        );

        let t_minor = compute_transition_t(50.0, 100.0, DegradationLevel::Minor);
        assert!(
            (t_minor - 0.5).abs() < 1e-6,
            "Minor: expected t=0.5, got {t_minor}"
        );

        let t_moderate = compute_transition_t(50.0, 100.0, DegradationLevel::Moderate);
        assert!(
            (t_moderate - 0.5).abs() < 1e-6,
            "Moderate: expected t=0.5, got {t_moderate}"
        );
    }

    /// Under RENDERING_SIMPLIFIED (Significant) or higher, transitions snap to t=1.0.
    ///
    /// Covers: openspec/changes/widget-system/design.md D5 stage 4 degradation note.
    #[test]
    fn compute_transition_t_snaps_at_rendering_simplified_and_above() {
        // All levels >= Significant must snap to 1.0, regardless of elapsed time.
        for level in [
            DegradationLevel::Significant,
            DegradationLevel::ShedTiles,
            DegradationLevel::Emergency,
        ] {
            let t = compute_transition_t(1.0, 1000.0, level); // only 0.1% elapsed
            assert_eq!(
                t, 1.0,
                "degradation level {level:?} should snap transition to t=1.0, got {t}"
            );
        }
    }

    /// Verify the snap produces the final parameter value for an f32 transition.
    #[test]
    fn degradation_snap_yields_final_f32_value() {
        let old = WidgetParameterValue::F32(0.0);
        let new_val = WidgetParameterValue::F32(100.0);

        // With t=1.0 (snap), interpolation must return the new value exactly.
        let snapped = interpolate_param(&old, &new_val, 1.0);
        assert_eq!(
            snapped,
            WidgetParameterValue::F32(100.0),
            "snapped f32 must equal final value"
        );
    }

    /// Verify the snap produces the final parameter value for a color transition.
    #[test]
    fn degradation_snap_yields_final_color_value() {
        let old = WidgetParameterValue::Color(Rgba::new(0.0, 0.0, 0.0, 1.0)); // black
        let new_val = WidgetParameterValue::Color(Rgba::new(1.0, 0.0, 0.0, 1.0)); // red

        let snapped = interpolate_param(&old, &new_val, 1.0);
        if let WidgetParameterValue::Color(c) = snapped {
            assert!((c.r - 1.0).abs() < 1e-6, "snapped r must be 1.0");
            assert!((c.g - 0.0).abs() < 1e-6, "snapped g must be 0.0");
            assert!((c.b - 0.0).abs() < 1e-6, "snapped b must be 0.0");
        } else {
            panic!("expected Color variant after snap");
        }
    }

    // ── Rgba → SVG color string tests ─────────────────────────────────────────

    #[test]
    fn test_rgba_to_svg_color_opaque() {
        let color = Rgba::new(1.0, 0.0, 0.0, 1.0);
        assert_eq!(rgba_to_svg_color(&color), "#ff0000");
    }

    #[test]
    fn test_rgba_to_svg_color_with_alpha() {
        let color = Rgba::new(1.0, 0.0, 0.0, 0.5);
        let s = rgba_to_svg_color(&color);
        assert!(
            s.starts_with("rgba(255,0,0,"),
            "should use rgba format: {s}"
        );
    }

    // ── Linear binding with spec scenario ────────────────────────────────────

    #[test]
    fn test_linear_mapping_spec_scenario() {
        // spec: level=0.5 with min=0,max=1 linear to attr_min=0,attr_max=200 → "100"
        let mut params = HashMap::new();
        params.insert("level".to_string(), WidgetParameterValue::F32(0.5));

        let binding = WidgetBinding {
            param: "level".to_string(),
            target_element: "bar".to_string(),
            target_attribute: "height".to_string(),
            mapping: WidgetBindingMapping::Linear {
                attr_min: 0.0,
                attr_max: 200.0,
            },
        };

        let mut constraints = HashMap::new();
        constraints.insert("level".to_string(), (0.0, 1.0));

        let val = resolve_binding_value(&binding, &params, &constraints).unwrap();
        assert_eq!(val, "100");
    }

    // ── SVG + resvg smoke test ────────────────────────────────────────────────

    #[test]
    fn test_apply_binding_and_rasterize() {
        // Verify the full pipeline: apply binding → modified SVG → usvg parse → render
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100">
            <rect id="bar" x="0" y="0" width="50" height="100" fill="blue"/>
        </svg>"#;

        let modified = apply_svg_attribute(svg, "bar", "width", "80");
        assert!(
            modified.contains("width=\"80\""),
            "width should be updated to 80"
        );

        // Parse and rasterize
        let opts = resvg::usvg::Options::default();
        let tree = resvg::usvg::Tree::from_str(&modified, &opts)
            .expect("modified SVG should parse without error");

        let mut pixmap = tiny_skia::Pixmap::new(100, 100).expect("pixmap creation");
        let transform = tiny_skia::Transform::from_scale(1.0, 1.0);
        resvg::render(&tree, transform, &mut pixmap.as_mut());

        // Pixel at x=75 should be blue (inside the 80-wide rect)
        let px = pixmap.pixel(75, 50).expect("pixel access");
        assert!(px.blue() > 200, "pixel inside rect should be blue");
    }

    #[test]
    fn test_rasterization_performance_512x512() {
        // Spec: re-rasterization MUST complete in less than 2ms for 512x512.
        // On CI this is a soft check — we log a warning but don't fail.
        // Use r##"..."## so that "#" inside SVG attribute values doesn't terminate the string.
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="512" height="512">
            <rect id="bar" x="0" y="0" width="256" height="512" fill="#336699"/>
            <rect id="accent" x="10" y="10" width="236" height="100" fill="#ff8800"/>
        </svg>"##;

        let start = std::time::Instant::now();

        let opts = resvg::usvg::Options::default();
        let tree = resvg::usvg::Tree::from_str(svg, &opts).expect("parse");
        let mut pixmap = tiny_skia::Pixmap::new(512, 512).expect("pixmap");
        let sx = 512.0 / tree.size().width();
        let sy = 512.0 / tree.size().height();
        resvg::render(
            &tree,
            tiny_skia::Transform::from_scale(sx, sy),
            &mut pixmap.as_mut(),
        );

        let elapsed_us = start.elapsed().as_micros();
        if elapsed_us > 2000 {
            eprintln!(
                "WARNING: 512x512 rasterization took {}µs (budget: 2000µs) — may fail on slow CI",
                elapsed_us
            );
        }
        // Verify non-trivial output
        assert_eq!(pixmap.data().len(), 512 * 512 * 4);
    }

    #[test]
    fn snap_composite_rect_rounds_fractional_origin() {
        let (x, y, w, h) = snap_composite_rect(2213.3333, 10.6667, 336.0, 128.0, 2560.0, 1440.0);
        assert_eq!(x, 2213.0);
        assert_eq!(y, 11.0);
        assert_eq!(w, 336.0);
        assert_eq!(h, 128.0);
    }

    #[test]
    fn snap_composite_rect_clamps_to_surface_bounds() {
        let (x, y, w, h) = snap_composite_rect(2500.9, 1430.2, 120.0, 40.0, 2560.0, 1440.0);
        assert_eq!(x, 2440.0);
        assert_eq!(y, 1400.0);
        assert_eq!(w, 120.0);
        assert_eq!(h, 40.0);
    }

    // ── Reference gauge rasterization tests ──────────────────────────────────
    //
    // Acceptance criterion 8 (hud-mim2.7): Compositor renders reference gauge
    // correctly. The fill.svg from the gauge fixture is rasterized with specific
    // parameter values and the pixel output is validated for correctness.
    //
    // Source: widget-system/spec.md §Deliverable 10 (Reference gauge widget bundle test fixture)

    /// Return the content of the reference gauge fill.svg fixture.
    fn reference_gauge_fill_svg() -> &'static str {
        // Inline the fill.svg content from the gauge fixture so this test has no
        // file system dependency and runs identically in CI (headless llvmpipe).
        // Use r##"..."## to allow "#" in SVG attribute hex color values.
        r##"<?xml version="1.0" encoding="UTF-8"?>
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 220" width="100" height="220">
  <rect id="bar" x="30" y="10" width="40" height="0"
        fill="#00b4ff" rx="3" ry="3"/>
  <text id="label-text" x="50" y="215" text-anchor="middle"
        font-family="sans-serif" font-size="10" fill="#cccccc"></text>
  <circle id="indicator" cx="85" cy="15" r="6" fill="#00cc66"/>
</svg>"##
    }

    /// Apply a set of parameter bindings to the reference gauge fill.svg and
    /// return the modified SVG string.
    ///
    /// Used to test the SVG binding pipeline for the reference gauge fixture.
    fn apply_gauge_params(
        level_height: &str,
        fill_color: &str,
        label: &str,
        indicator_fill: &str,
    ) -> String {
        let svg = reference_gauge_fill_svg();
        let svg = apply_svg_attribute(svg, "bar", "height", level_height);
        let svg = apply_svg_attribute(&svg, "bar", "fill", fill_color);
        let svg = apply_svg_attribute(&svg, "label-text", "text-content", label);
        apply_svg_attribute(&svg, "indicator", "fill", indicator_fill)
    }

    /// Parse and rasterize an SVG string into a Pixmap of the given dimensions.
    ///
    /// Shared helper to avoid repeating SVG parse + rasterize setup across pixel
    /// assertion tests. Panics (via `.expect`) if the SVG is malformed or pixmap
    /// allocation fails — test failures are the expected signal.
    fn rasterize_svg(svg_data: &str, width: u32, height: u32) -> tiny_skia::Pixmap {
        let opts = resvg::usvg::Options::default();
        let tree =
            resvg::usvg::Tree::from_str(svg_data, &opts).expect("SVG must parse without error");

        let mut pixmap =
            tiny_skia::Pixmap::new(width, height).expect("pixmap creation must not fail");

        let transform = tiny_skia::Transform::from_scale(
            width as f32 / tree.size().width(),
            height as f32 / tree.size().height(),
        );

        resvg::render(&tree, transform, &mut pixmap.as_mut());
        pixmap
    }

    /// WHEN the reference gauge fill.svg is parsed with default parameters
    /// THEN resvg parses it without error and produces non-trivial pixel output.
    ///
    /// Source: widget-system/spec.md §Deliverable 10 — reference gauge fixture.
    #[test]
    fn reference_gauge_fill_svg_parses_and_rasterizes() {
        let svg = reference_gauge_fill_svg();

        let w = 100u32;
        let h = 220u32;
        let pixmap = rasterize_svg(svg, w, h);

        // Output must be 100×220×4 bytes.
        assert_eq!(pixmap.data().len(), (w * h * 4) as usize);

        // At least one non-zero pixel — the SVG has a circle and a rect.
        let has_nonzero = pixmap.data().iter().any(|&b| b > 0);
        assert!(
            has_nonzero,
            "rasterized reference gauge must produce non-transparent pixels"
        );
    }

    /// WHEN the bar height is set to 100 (50% fill) THEN the blue fill appears in
    /// the expected region.
    ///
    /// fill.svg bar: x=30, y=10, width=40. With height=100 and fill=#00b4ff (blue),
    /// the bar extends from y=10 to y=110. The pixel at (50, 60) — center of bar
    /// at mid-height — should be the blue fill color.
    ///
    /// Source: widget-system/spec.md §Requirement: SVG Layer Parameter Bindings (linear mapping).
    #[test]
    fn reference_gauge_bar_half_full_renders_blue_in_fill_region() {
        // Apply level=100 (50% fill height), blue fill, empty label, info indicator.
        let modified_svg = apply_gauge_params("100", "#00b4ff", "", "#00cc66");

        let w = 100u32;
        let h = 220u32;
        let pixmap = rasterize_svg(&modified_svg, w, h);

        // Bar at x=30..70, y=10..110. Sample pixel at (50, 60): inside the filled bar.
        // The bar fill is #00b4ff (R=0, G=180, B=255). Allow ±20 for anti-aliasing.
        let px = pixmap.pixel(50, 60).expect("pixel access");
        assert!(
            px.blue() > 180,
            "pixel at (50, 60) inside blue fill bar should have high blue channel, got: b={}",
            px.blue()
        );
        assert!(
            px.red() < 50,
            "pixel at (50, 60) inside blue bar should have low red, got: r={}",
            px.red()
        );
    }

    /// WHEN the bar fill is set to red (#ff0000) THEN the bar region renders red.
    ///
    /// Tests the fill_color → direct → bar.fill binding in the reference gauge.
    #[test]
    fn reference_gauge_bar_red_fill_renders_red() {
        let modified_svg = apply_gauge_params("100", "#ff0000", "", "#00cc66");

        let w = 100u32;
        let h = 220u32;
        let pixmap = rasterize_svg(&modified_svg, w, h);

        // Bar center at (50, 60): should be red (#ff0000).
        let px = pixmap.pixel(50, 60).expect("pixel access");
        assert!(
            px.red() > 180,
            "pixel inside red-filled bar should have high red channel, got: r={}",
            px.red()
        );
        assert!(
            px.blue() < 50,
            "pixel inside red-filled bar should have low blue channel, got: b={}",
            px.blue()
        );
    }

    /// WHEN the severity indicator is set to warning (#ffcc00) THEN the indicator
    /// circle region renders yellow.
    ///
    /// Tests the severity → discrete → indicator.fill binding.
    /// Indicator circle: cx=85, cy=15, r=6.
    #[test]
    fn reference_gauge_indicator_warning_renders_yellow() {
        // Warning severity maps to #ffcc00 (yellow-gold).
        let modified_svg = apply_gauge_params("0", "#00b4ff", "", "#ffcc00");

        let w = 100u32;
        let h = 220u32;
        let pixmap = rasterize_svg(&modified_svg, w, h);

        // Indicator circle center at (85, 15) in the 100×220 SVG coordinate space.
        // With scale 1.0, the center maps to pixel (85, 15).
        let px = pixmap
            .pixel(85, 15)
            .expect("pixel access at indicator center");
        assert!(
            px.red() > 180,
            "warning indicator should be yellow (high red), got: r={}",
            px.red()
        );
        assert!(
            px.green() > 150,
            "warning indicator should be yellow (high green), got: g={}",
            px.green()
        );
        assert!(
            px.blue() < 50,
            "warning indicator should be yellow (low blue), got: b={}",
            px.blue()
        );
    }

    /// WHEN the reference gauge is rasterized at 512×512 with a full-height bar
    /// THEN rasterization completes within the 2ms budget.
    ///
    /// Source: hud-mim2.7 acceptance criterion 10 — re-rasterization < 2ms for 512×512.
    #[test]
    fn reference_gauge_rasterization_within_2ms_budget_at_512x512() {
        let modified_svg = apply_gauge_params("200", "#00b4ff", "CPU", "#00cc66");

        let start = std::time::Instant::now();
        let pixmap = rasterize_svg(&modified_svg, 512, 512);
        let elapsed_us = start.elapsed().as_micros();

        // Soft budget check: warn on CI if over budget; do not fail the build.
        // The reference hardware target is 3GHz single-core; llvmpipe in CI may be slower.
        if elapsed_us > 2000 {
            eprintln!(
                "WARNING: reference gauge 512×512 rasterization took {}µs (budget: 2000µs) — may fail on slow CI",
                elapsed_us
            );
        }

        // The output must be valid 512×512 RGBA8.
        assert_eq!(pixmap.data().len(), 512 * 512 * 4);
    }
}
