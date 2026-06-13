//! Image texture cache, local composer state, and font/image loading methods
//! for the compositor.
//!
//! Moved from `renderer/mod.rs` banner 6 (`// ─── Image texture cache ──`)
//! and the font/image loading method cluster (formerly ~L1338–1735 pre-split),
//! by Step R-2 of the renderer module split (hud-fgryk).  No logic was changed;
//! only visibility modifiers were added where Rust's module-privacy rules
//! require them (listed in the PR body).

use std::collections::HashSet;
use std::sync::{Arc, Mutex as StdMutex};

use tze_hud_scene::resolve_token_placeholders;
use tze_hud_scene::types::*;

use crate::text::TextRasterizer;

// ─── Image texture cache ────────────────────────────────────────────────────

/// A cached GPU texture for a static image resource.
///
/// Created by [`Compositor::ensure_image_texture`] on first reference and
/// reused across frames until eviction.
pub struct ImageTextureEntry {
    /// The GPU texture holding RGBA pixel data, kept alive for `bind_group`.
    /// Prefixed with `_` to make intent explicit: this field exists solely to
    /// retain ownership and keep the wgpu texture alive as long as the bind
    /// group references it.
    pub _texture: wgpu::Texture,
    /// Pre-built bind group (texture view + sampler) ready for draw calls.
    pub bind_group: wgpu::BindGroup,
    /// Image width in pixels (needed for fit-mode UV calculations).
    pub width: u32,
    /// Image height in pixels (needed for fit-mode UV calculations).
    pub height: u32,
}

/// GPU state and render pipeline.
/// Runtime-side local composer echo state.
///
/// Passed from the input-event thread to the compositor thread via an
/// [`Arc<StdMutex<Option<LocalComposerState>>>`] so the compositor can render
/// the current draft text + caret locally on every frame — WITHOUT waiting for
/// an adapter round-trip (§4.1 "Local feedback first").
///
/// Created by the windowed runtime on every keystroke that mutates the draft,
/// cleared when the composer is deactivated (blur, submit, cancel).
#[derive(Debug, Clone)]
pub struct LocalComposerState {
    /// Draft text at the cursor byte position (pre-caret).
    pub text: String,
    /// Caret position as a byte offset into `text`.
    pub cursor_byte: usize,
    /// True when the draft buffer has reached its byte capacity.
    pub at_capacity: bool,
    /// The `SceneId` of the `HitRegionNode` that owns this composer region.
    /// Used to locate the tile bounds for overlay placement.
    pub node_id: tze_hud_scene::types::SceneId,
}

/// Shared update slot for delivering [`LocalComposerState`] from the
/// input-event thread to the compositor thread without locking the full scene.
///
/// Semantics:
/// - `None` — no new update since the last frame; compositor keeps its current
///   `local_composer` unchanged.
/// - `Some(None)` — explicit deactivation (blur / submit / cancel); compositor
///   clears `local_composer` so the overlay disappears.
/// - `Some(Some(state))` — new draft state; compositor replaces `local_composer`.
///
/// Written by the input-event thread (main thread) on every composer mutation
/// and on deactivation.  Drained (taken) by the compositor thread once per
/// frame at the top of the render loop.
pub type LocalComposerStateHandle = Arc<StdMutex<Option<Option<LocalComposerState>>>>;

/// Build the display string for the composer echo overlay: the draft text with
/// the caret glyph (`▌`, U+258C LEFT HALF BLOCK) inserted at `cursor_byte`.
///
/// `cursor_byte` is an **agent-provided** offset and may be:
/// - out-of-range (> `text.len()`) — clamped to `text.len()`.
/// - mid-multi-byte-character — snapped **down** to the nearest valid char
///   boundary (same pattern as the overflow / input crates).
///
/// Either way this function never panics on agent input.
pub(crate) fn composer_display_text(text: &str, cursor_byte: usize) -> String {
    // Clamp to [0, text.len()] first, then walk back to a char boundary.
    let mut cursor = cursor_byte.min(text.len());
    while cursor > 0 && !text.is_char_boundary(cursor) {
        cursor -= 1;
    }
    let mut display = String::with_capacity(text.len() + '▌'.len_utf8());
    display.push_str(&text[..cursor]);
    display.push('▌');
    display.push_str(&text[cursor..]);
    display
}

/// Apply one drain cycle: take the pending update from `handle` and write it
/// into `current`.
///
/// - `handle` holds `None`          → `current` is left unchanged.
/// - `handle` holds `Some(None)`    → `current` is cleared (deactivation).
/// - `handle` holds `Some(Some(s))` → `current` is replaced with `s`.
///
/// This is the pure logic extracted from [`Compositor::drain_local_composer_state`]
/// so that tests can drive it without a GPU-backed [`Compositor`].
pub(crate) fn apply_composer_slot(
    handle: &LocalComposerStateHandle,
    current: &mut Option<LocalComposerState>,
) {
    if let Ok(mut guard) = handle.lock() {
        if let Some(update) = guard.take() {
            *current = update;
        }
    }
}

// ─── Font / image loading methods ────────────────────────────────────────────

impl super::Compositor {
    /// Initialize (or re-initialize) the text rasterizer for a given surface format.
    ///
    /// Must be called once after creation before text rendering is available.
    /// For headless compositors, `format` should be `Rgba8UnormSrgb`.
    /// For windowed compositors, use the negotiated swapchain format.
    ///
    /// Calling this multiple times replaces the existing rasterizer (e.g. on
    /// surface resize or format change).
    pub fn init_text_renderer(&mut self, format: wgpu::TextureFormat) {
        self.text_rasterizer = Some(TextRasterizer::new(&self.device, &self.queue, format));
        tracing::debug!(format = ?format, "text renderer initialized");
    }

    /// Load raw font bytes (TTF or OTF) from an agent upload into glyphon's
    /// `FontSystem`.
    ///
    /// After this call, the font is available for text layout in subsequent
    /// frames.  The `resource_id` is the 32-byte BLAKE3 digest of `data`
    /// (matches `tze_hud_resource::ResourceId::as_bytes()`); duplicate calls
    /// with the same `resource_id` are no-ops.
    ///
    /// Silently skips if the text renderer is not yet initialized (e.g. if
    /// `init_text_renderer` has not been called).  Callers in the runtime may
    /// defer font uploads until the compositor is ready.
    ///
    /// # Truncation cache invalidation
    ///
    /// Loading a new font changes shaping metrics (advance widths, kerning),
    /// which invalidates any cached truncation results computed with the old
    /// font.  This method therefore resets `truncation_cache_scene_version` to
    /// `u64::MAX` — the forced-re-prime sentinel — so the next call to
    /// `prime_truncation_cache` recomputes all entries regardless of whether
    /// the scene version has changed.  The cadence gate is bypassed for
    /// forced primes (see `prime_truncation_cache`), ensuring the new font is
    /// reflected immediately on the next commit.  [hud-v2z6u]
    pub fn load_font_bytes(&mut self, resource_id: [u8; 32], data: &[u8]) {
        if let Some(rasterizer) = &mut self.text_rasterizer {
            let was_new = !rasterizer.has_font(&resource_id);
            rasterizer.load_font_bytes(resource_id, data);
            if was_new {
                // A new font changes shaping widths — cached truncation points
                // are now stale.  Reset to the forced-prime sentinel so the
                // next prime_truncation_cache call re-resolves all entries.
                self.truncation_cache_scene_version = u64::MAX;
                tracing::debug!(
                    resource_id = %crate::text::format_resource_id(&resource_id),
                    "load_font_bytes: new font loaded — truncation cache invalidated [hud-v2z6u]"
                );
            }
        } else {
            tracing::debug!("load_font_bytes called before text renderer is initialized — skipped");
        }
    }

    /// Returns `true` if the font with the given `resource_id` has been loaded
    /// into the `FontSystem`.
    ///
    /// Returns `false` if the text renderer is not yet initialized.
    pub fn has_font(&self, resource_id: &[u8; 32]) -> bool {
        self.text_rasterizer
            .as_ref()
            .map(|r| r.has_font(resource_id))
            .unwrap_or(false)
    }

    // ─── Image texture cache ─────────────────────────────────────────────────

    /// Register decoded RGBA image bytes for a resource.
    ///
    /// The runtime should call this after each successful image upload so the
    /// compositor can create GPU textures on demand. `rgba_data` must be
    /// `width × height × 4` bytes of RGBA8 pixel data. The explicit `width` and
    /// `height` are stored alongside the bytes so that
    /// [`Compositor::ensure_scene_image_textures`] can create GPU textures for
    /// zone images (which do not carry dimensions in `ZoneContent::StaticImage`)
    /// without resorting to the square-root heuristic that fails for non-square
    /// images (e.g. 640×360).
    ///
    /// Duplicate calls with the same `resource_id` are ignored (content-addressed
    /// identity guarantees the bytes are identical).
    ///
    /// Returns without registering if `width` or `height` is zero, or if the
    /// product `width × height × 4` overflows `usize`.
    pub fn register_image_bytes(
        &mut self,
        resource_id: ResourceId,
        rgba_data: Arc<[u8]>,
        width: u32,
        height: u32,
    ) {
        // Guard: reject zero-dimension images; content-addressed uploads should
        // never arrive with degenerate dimensions, so treat this as a caller bug.
        if width == 0 || height == 0 {
            return;
        }
        // Guard: overflow — reject if the byte count would overflow usize.
        if width
            .checked_mul(height)
            .and_then(|px| px.checked_mul(4))
            .is_none()
        {
            return;
        }

        // Insert both maps atomically via the Entry API.  Using separate
        // `or_insert` calls would allow one map to contain the key without the
        // other if a previous partially-failed insertion left them out of sync.
        if let std::collections::hash_map::Entry::Vacant(entry) =
            self.image_bytes.entry(resource_id)
        {
            entry.insert(rgba_data);
            self.image_dims.insert(resource_id, (width, height));
        }
    }

    /// Ensure a GPU texture exists for the given `ResourceId`.
    ///
    /// Returns `true` if a texture is ready (either already cached or just
    /// created). Returns `false` if the image bytes are not registered.
    ///
    /// The flow: check `image_texture_cache` → miss → look up `image_bytes` →
    /// create `wgpu::Texture` → write data → create `BindGroup` → cache.
    pub(super) fn ensure_image_texture(
        &mut self,
        resource_id: ResourceId,
        img_width: u32,
        img_height: u32,
    ) -> bool {
        // Already cached?
        if self.image_texture_cache.contains_key(&resource_id) {
            return true;
        }

        // Retrieve raw RGBA bytes.
        let rgba_data = match self.image_bytes.get(&resource_id) {
            Some(data) => Arc::clone(data),
            None => {
                tracing::debug!(
                    resource_id = %resource_id,
                    "image bytes not registered — falling back to placeholder"
                );
                return false;
            }
        };

        // Validate byte count.
        let expected_bytes = (img_width as usize) * (img_height as usize) * 4;
        if rgba_data.len() != expected_bytes {
            tracing::warn!(
                resource_id = %resource_id,
                expected = expected_bytes,
                actual = rgba_data.len(),
                "image bytes length mismatch — falling back to placeholder"
            );
            return false;
        }

        // Create GPU texture.
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some(&format!("img_tex_{resource_id}")),
            size: wgpu::Extent3d {
                width: img_width,
                height: img_height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        // Upload pixel data.
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &rgba_data,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(img_width * 4),
                rows_per_image: Some(img_height),
            },
            wgpu::Extent3d {
                width: img_width,
                height: img_height,
                depth_or_array_layers: 1,
            },
        );

        // Create bind group.
        let texture_view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(&format!("img_bg_{resource_id}")),
            layout: &self.texture_rect_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&texture_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.image_sampler),
                },
            ],
        });

        self.image_texture_cache.insert(
            resource_id,
            ImageTextureEntry {
                _texture: texture,
                bind_group,
                width: img_width,
                height: img_height,
            },
        );

        tracing::debug!(
            resource_id = %resource_id,
            width = img_width,
            height = img_height,
            "image texture uploaded to GPU"
        );

        true
    }

    /// Ensure a GPU texture exists for a status-bar icon SVG file path.
    ///
    /// Uses `ResourceId::of(path.as_bytes())` as the cache key so the texture
    /// is stored in `image_texture_cache` and drawn by the standard image pass.
    ///
    /// On cache hit, returns `true` immediately (no I/O).  On miss:
    /// 1. Reads the SVG bytes from `path` (filesystem).
    ///    NOTE: This is a blocking filesystem read on the compositor thread.
    ///    It only occurs once per unique SVG path (cache miss); subsequent
    ///    calls for the same path return immediately from cache or the
    ///    negative-cache set (`failed_icon_paths`).
    /// 2. Rasterizes at [`ICON_SIZE_PX`] × [`ICON_SIZE_PX`] via resvg/tiny-skia.
    /// 3. Uploads the RGBA pixmap to a `wgpu::Texture`.
    /// 4. Inserts into `image_texture_cache` under the path-derived `ResourceId`.
    ///
    /// Returns `false` if the file cannot be read or parsed.  In that case the
    /// `ResourceId` is added to `failed_icon_paths` so no further I/O is
    /// attempted for this path (graceful degradation — text-only fallback).
    pub(super) fn ensure_icon_texture(&mut self, path: &str) -> bool {
        let resource_id = ResourceId::of(path.as_bytes());
        if self.image_texture_cache.contains_key(&resource_id) {
            return true;
        }
        // Negative cache: skip repeated I/O for known-bad paths.
        if self.failed_icon_paths.contains(&resource_id) {
            return false;
        }

        let size = super::token_colors::ICON_SIZE_PX as u32;

        // Read SVG bytes from disk (blocking; occurs at most once per path).
        let svg_bytes = match std::fs::read(path) {
            Ok(b) => b,
            Err(e) => {
                tracing::warn!(path, error = %e, "icon: failed to read SVG file");
                self.failed_icon_paths.insert(resource_id);
                return false;
            }
        };
        let svg_str = match std::str::from_utf8(&svg_bytes) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(path, error = %e, "icon: SVG file is not valid UTF-8");
                self.failed_icon_paths.insert(resource_id);
                return false;
            }
        };

        // Apply design token substitution before parsing.
        // `resolve_token_placeholders` has an internal fast path for SVGs that
        // contain neither `{{` nor escape sequences — only a `contains` scan,
        // no allocation.  For SVGs with placeholders that cannot be resolved
        // (token missing), log a warning and degrade gracefully (skip icon).
        let resolved_svg;
        let svg_str = match resolve_token_placeholders(svg_str, &self.token_map) {
            Ok(s) => {
                resolved_svg = s;
                resolved_svg.as_str()
            }
            Err(missing_key) => {
                tracing::warn!(
                    path,
                    token_key = %missing_key,
                    "icon: unresolved design token in SVG; skipping icon"
                );
                self.failed_icon_paths.insert(resource_id);
                return false;
            }
        };

        // Rasterize via resvg (same pipeline as WidgetRenderer).
        let opts = resvg::usvg::Options::default();
        let tree = match resvg::usvg::Tree::from_str(svg_str, &opts) {
            Ok(t) => t,
            Err(e) => {
                tracing::warn!(path, error = %e, "icon: failed to parse SVG");
                self.failed_icon_paths.insert(resource_id);
                return false;
            }
        };
        let mut pixmap = match tiny_skia::Pixmap::new(size, size) {
            Some(p) => p,
            None => {
                tracing::warn!(path, size, "icon: failed to allocate pixmap");
                self.failed_icon_paths.insert(resource_id);
                return false;
            }
        };
        // Scale uniformly with centering (same transform logic as widget rasterization).
        let svg_size = tree.size();
        let sx = size as f32 / svg_size.width();
        let sy = size as f32 / svg_size.height();
        let uniform_scale = sx.min(sy);
        let rendered_w = svg_size.width() * uniform_scale;
        let rendered_h = svg_size.height() * uniform_scale;
        let offset_x = (size as f32 - rendered_w) * 0.5;
        let offset_y = (size as f32 - rendered_h) * 0.5;
        let transform = tiny_skia::Transform::from_translate(offset_x, offset_y)
            .post_scale(uniform_scale, uniform_scale);
        resvg::render(&tree, transform, &mut pixmap.as_mut());

        // Upload to GPU.
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some(&format!("icon_tex_{path}")),
            size: wgpu::Extent3d {
                width: size,
                height: size,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            pixmap.data(),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(size * 4),
                rows_per_image: Some(size),
            },
            wgpu::Extent3d {
                width: size,
                height: size,
                depth_or_array_layers: 1,
            },
        );
        let texture_view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(&format!("icon_bg_{path}")),
            layout: &self.texture_rect_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&texture_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.image_sampler),
                },
            ],
        });
        self.image_texture_cache.insert(
            resource_id,
            ImageTextureEntry {
                _texture: texture,
                bind_group,
                width: size,
                height: size,
            },
        );
        tracing::debug!(path, size, "icon texture rasterized and uploaded to GPU");
        true
    }

    /// Evict cached GPU textures for resources no longer referenced in the scene.
    ///
    /// Call once per frame after rendering. `referenced_ids` is the set of
    /// `ResourceId`s that appeared in zone publications or tile nodes during
    /// this frame. Any cache entry not in this set is dropped.
    pub fn evict_unused_image_textures(&mut self, referenced_ids: &HashSet<ResourceId>) {
        self.image_texture_cache
            .retain(|id, _| referenced_ids.contains(id));
        // Also evict bytes and dims for resources no longer referenced.
        self.image_bytes.retain(|id, _| referenced_ids.contains(id));
        self.image_dims.retain(|id, _| referenced_ids.contains(id));
    }
}
