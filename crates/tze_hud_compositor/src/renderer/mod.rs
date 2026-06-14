//! The compositor — renders the scene graph to a surface.
//!
//! For the vertical slice, this renders tiles as colored rectangles
//! with hit-region highlighting for local feedback.
//!
//! The frame pipeline is **surface-agnostic**: it renders through the
//! `CompositorSurface` trait, which is implemented by both `HeadlessSurface`
//! (offscreen) and `WindowSurface` (display-connected).  No conditional
//! compilation exists in the render path — only the surface implementation
//! differs between modes.
//!
//! Per runtime-kernel/spec.md Requirement: Headless Mode (line 198):
//! "No conditional compilation for the render path."
//!
//! ## Two-pass rendering (content → chrome)
//!
//! [`Compositor::render_frame_with_chrome`] implements the three-layer ordering required by
//! the chrome sovereignty contract:
//!   1. Background + content pass (`LoadOp::Clear` — clears and draws agent tiles)
//!   2. Chrome pass (`LoadOp::Load` — draws chrome on top of content, preserving pixels)
//!
//! This separation is the architectural foundation for future render-skip redaction
//! (capture-safe architecture): the content and chrome passes are structurally independent.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex as StdMutex};

use crate::pipeline::{
    ChromeDrawCmd, ROUNDED_RECT_OVERLAY_SHADER, ROUNDED_RECT_SHADER, RectVertex,
    RoundedRectDrawCmd, RoundedRectVertex, create_texture_rect_bind_group_layout,
    create_texture_rect_pipeline, rect_vertices,
};
use crate::surface::{CompositorSurface, HeadlessSurface};
use crate::text::{TextItem, TextRasterizer};
use crate::widget::WidgetRenderer;
use tze_hud_scene::DegradationLevel;
use tze_hud_scene::graph::SceneGraph;
use tze_hud_scene::types::*;
use tze_hud_telemetry::FrameTelemetry;

pub mod animation;
pub mod draw_cmds;
pub mod encode_pass;
pub mod frame;
pub mod hit_regions;
pub mod icon;
pub mod image_cache;
pub mod text;
pub mod tile_render;
pub mod token_colors;
pub mod video;
pub mod widget_geometry;
pub mod zone_render;

// Re-export submodule items into this module's namespace so all existing code
// in mod.rs can reference them without path changes (move-only, zero refactor).
use draw_cmds::*;
use icon::*;
// pub use re-exports the public items (ImageTextureEntry, LocalComposerState,
// LocalComposerStateHandle) so callers via `tze_hud_compositor::renderer::*`
// see unchanged paths.  The pub(crate) helpers are brought in separately.
use image_cache::apply_composer_slot;
pub use image_cache::{ImageTextureEntry, LocalComposerState, LocalComposerStateHandle};
use text::*;
use token_colors::*;

pub struct Compositor {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pipeline: wgpu::RenderPipeline,
    /// Pipeline with no blending — writes RGBA directly. Used to clear
    /// the framebuffer to transparent in overlay mode (LoadOp::Clear
    /// doesn't write alpha correctly on some GPUs).
    clear_pipeline: wgpu::RenderPipeline,
    /// Render pipeline for textured rectangles (static image rendering).
    texture_rect_pipeline: wgpu::RenderPipeline,
    /// Bind group layout for the texture rect pipeline (shared by all images).
    texture_rect_bind_group_layout: wgpu::BindGroupLayout,
    /// Shared linear-filtering sampler for all image textures (created once).
    image_sampler: wgpu::Sampler,
    /// SDF rounded-rectangle pipeline (fullscreen / straight-alpha mode).
    ///
    /// Used to render zone backdrops whose `RenderingPolicy` has
    /// `backdrop_radius` set.  Encoded in a separate pass after the main
    /// rect pass so rounded corners composite cleanly over the background.
    ///
    /// Uses `BlendState::ALPHA_BLENDING`; vertex colors are non-premultiplied.
    /// In overlay mode `rounded_rect_overlay_pipeline` is selected instead.
    rounded_rect_pipeline: wgpu::RenderPipeline,
    /// SDF rounded-rectangle pipeline for overlay / premultiplied-alpha mode.
    ///
    /// Identical geometry to `rounded_rect_pipeline` but uses
    /// `BlendState::PREMULTIPLIED_ALPHA_BLENDING` and
    /// `ROUNDED_RECT_OVERLAY_SHADER` (which scales all four channels by
    /// coverage).  Selected by `encode_rounded_rect_pass` when
    /// `self.overlay_mode` is true.
    rounded_rect_overlay_pipeline: wgpu::RenderPipeline,
    pub width: u32,
    pub height: u32,
    frame_number: u64,
    /// When true, the clear color uses alpha=0 for transparent overlay mode.
    pub overlay_mode: bool,
    /// When true, render all zone boundaries with colored tints even when
    /// zones have no active content. Controlled by `TZE_HUD_DEBUG_ZONES=1`.
    pub debug_zone_tints: bool,
    /// Current degradation level, set by the runtime before each frame.
    ///
    /// At [`DegradationLevel::Significant`] or higher, widget transition
    /// interpolation is skipped and final parameter values are applied
    /// immediately to reduce re-rasterization under load.
    pub degradation_level: DegradationLevel,
    /// Optional text rasterizer (glyphon). Absent until `init_text_renderer`
    /// is called. When `None`, TextMarkdownNode and zone StreamText content
    /// renders as solid-color rectangles only (no glyph output).
    pub(crate) text_rasterizer: Option<TextRasterizer>,
    /// Optional widget renderer. Absent until `init_widget_renderer` is called.
    /// When `None`, widget instances in the scene graph are not composited.
    pub(crate) widget_renderer: Option<WidgetRenderer>,
    /// Per-zone fade-in / fade-out animation state.
    ///
    /// Keyed by zone name. An entry is inserted on the first publish to a zone
    /// that has `transition_in_ms > 0`, and on every zone clear when
    /// `transition_out_ms > 0`. Completed transitions are pruned each frame.
    pub(crate) zone_animation_states: HashMap<String, ZoneAnimationState>,
    /// Track which zones had active publishes in the previous frame so we can
    /// detect publish → clear transitions and start fade-out animations.
    prev_active_zones: HashMap<String, bool>,
    /// Per-portal-tile fade-in / fade-out animation state (§6.3 transition tokens).
    ///
    /// Keyed by tile `SceneId`. An entry is inserted when a scrollable tile
    /// (i.e. a portal tile, identified by a registered `TileScrollConfig`)
    /// gains its first root node (`transition_in_ms` fade-in) and when it
    /// loses its root node (`transition_out_ms` fade-out).
    ///
    /// Durations are sourced from the `portal.transition.in_ms` and
    /// `portal.transition.out_ms` design tokens so no literal durations are
    /// present in the compositor.  Completed transitions are pruned each frame.
    pub(crate) portal_tile_anim_states: HashMap<SceneId, ZoneAnimationState>,
    /// Track which scrollable tiles had a root node in the previous frame so
    /// we can detect appear/disappear transitions.
    prev_portal_tile_has_content: HashMap<SceneId, bool>,
    /// Per-publication TTL fade-out animation state.
    ///
    /// Outer key: zone name.  Inner key: `PubKey = (published_at_wall_us, publisher_namespace)`.
    ///
    /// Created when the compositor first encounters a publication in a Stack zone.
    /// Entries for publications no longer in `active_publishes` are pruned by
    /// [`Compositor::prune_faded_publications`].
    pub(crate) pub_animation_states: HashMap<String, HashMap<PubKey, PublicationAnimationState>>,
    /// Per-zone streaming word-by-word reveal state.
    ///
    /// Keyed by zone name.  Present only when the latest publication in that zone
    /// has non-empty breakpoints.  Absent (or None) means reveal all at once.
    ///
    /// Per spec §Subtitle Streaming Word-by-Word Reveal: when a new publication
    /// replaces the old one (latest-wins), the old reveal state is discarded and
    /// a new one starts from the beginning of the new breakpoints.
    pub(crate) stream_reveal_states: HashMap<String, StreamRevealState>,
    /// Resolved design token map, set at startup via `set_token_map`.
    ///
    /// Used to resolve `color.severity.{info,warning,critical}` tokens for
    /// alert-banner backdrop colors. When empty (the default), all severity
    /// lookups fall back to the hardcoded `SEVERITY_*` constants.
    ///
    /// Populated by calling `set_token_map` after `run_component_startup`
    /// produces a `ComponentStartupResult::global_tokens`.
    pub token_map: HashMap<String, String>,
    /// Decoded RGBA image bytes indexed by `ResourceId`.
    ///
    /// Populated by calling [`Compositor::register_image_bytes`] after an image
    /// resource is uploaded. The compositor decodes/uploads to GPU texture on
    /// first reference via [`Compositor::ensure_image_texture`].
    ///
    /// The raw bytes are kept until the ResourceId is no longer referenced by
    /// any scene node or zone publication (evicted by `evict_unused_image_textures`).
    image_bytes: HashMap<ResourceId, Arc<[u8]>>,
    /// Explicit pixel dimensions for registered image resources, keyed by `ResourceId`.
    ///
    /// Populated alongside `image_bytes` by [`Compositor::register_image_bytes`].
    /// Stores `(width, height)` so that `ensure_scene_image_textures` can pass
    /// exact dimensions to [`Compositor::ensure_image_texture`] without resorting
    /// to the square-root heuristic that fails for non-square images.
    image_dims: HashMap<ResourceId, (u32, u32)>,
    /// GPU texture cache for static images, keyed by `ResourceId`.
    ///
    /// Entries are created on-demand by `ensure_image_texture` and evicted
    /// when the resource is no longer referenced in the scene.
    ///
    /// Also stores status-bar icon textures via `ensure_icon_texture`, which
    /// uses `ResourceId::of(svg_path.as_bytes())` as the key.  Icon entries
    /// are kept alive by including their ResourceIds in the eviction-guard set
    /// (see `ensure_scene_icon_textures`).
    image_texture_cache: HashMap<ResourceId, ImageTextureEntry>,
    /// Negative cache for SVG paths that failed to load/parse.
    ///
    /// Paths are inserted on the first failed `ensure_icon_texture` call.
    /// Subsequent calls skip the filesystem read entirely, avoiding per-frame
    /// I/O and log spam for invalid icon paths. Entries persist for the
    /// lifetime of the compositor (SVG paths are static config; runtime reload
    /// of icon paths is not currently supported).
    failed_icon_paths: HashSet<ResourceId>,
    /// Phase-1 markdown parse cache, fronted by the async primer (hud-5jbra.2,
    /// hud-380dl, hud-33qo7).
    ///
    /// Maps BLAKE3(content) → [`ParsedMarkdown`].  Populated at content-commit
    /// time by [`Compositor::prime_markdown_cache`]; consumed at render time by
    /// [`Compositor::collect_text_items_from_node`] with zero parse cost on
    /// cache hits.  The cache is evicted in sync with node removal.
    ///
    /// hud-33qo7: the primer owns an `Arc<ArcSwap<MarkdownCache>>`.  Large
    /// payloads are parsed on a background thread and swapped in atomically; the
    /// render path `load()`s the current snapshot lock-free.  Read it via
    /// [`Compositor::markdown_cache`].
    ///
    /// [`ParsedMarkdown`]: crate::markdown::ParsedMarkdown
    pub(crate) markdown_primer: crate::markdown::MarkdownPrimer,
    /// Per-node content-key cache (hud-gpqde).
    ///
    /// Maps `SceneId` → precomputed BLAKE3 content key, populated at prime time
    /// alongside `markdown_cache`.  Eliminates the per-frame BLAKE3 re-hash in
    /// `collect_text_items_from_node` — the frame path does a 32-byte HashMap
    /// lookup rather than hashing the full content string every frame.
    ///
    /// Invalidated and rebuilt on every `prime_markdown_cache` call when the
    /// scene version changes.  Evicted in sync with `markdown_cache.evict_except`.
    pub(crate) node_key_cache: HashMap<SceneId, [u8; 32]>,
    /// Design-token–resolved markdown styling (hud-5jbra.2).
    ///
    /// Rebuilt from `token_map` whenever `set_token_map` is called.
    /// Consumed by `prime_markdown_cache` when new content is parsed.
    pub(crate) markdown_tokens: crate::markdown::MarkdownTokens,
    /// The `SceneGraph::version` value at the last `prime_markdown_cache` call.
    ///
    /// `prime_markdown_cache` is gated on this value so it only runs when the
    /// scene has actually changed.  Initialized to `u64::MAX` so the first
    /// frame always primes.
    markdown_cache_scene_version: u64,
    /// The `SceneGraph::version` value at the last `prime_truncation_cache` call.
    ///
    /// `prime_truncation_cache` is gated on this value so it only runs when the
    /// scene has actually changed.  Initialized to `u64::MAX` so the first
    /// frame always primes.
    truncation_cache_scene_version: u64,
    /// Instant of the last completed `prime_truncation_cache` run.
    ///
    /// Used by the adaptive mid-drag re-truncation cadence gate (hud-ghhxa,
    /// hud-3to8i): when the scene version changes rapidly (e.g. per-frame
    /// bounds updates during a resize drag), we cap the re-prime rate to at
    /// most once per the adaptive interval so we never re-prime every frame.
    ///
    /// `None` means no prime has run yet (the first frame always primes).
    /// When `Some`, a new prime is only allowed when at least the adaptive
    /// interval (derived from [`resize_reprime_content_bytes`]) has elapsed.
    resize_reprime_last_at: Option<std::time::Instant>,
    /// Total byte count of Ellipsis text content across all visible tiles at
    /// the time of the last successful `prime_truncation_cache` run.
    ///
    /// Used as the input to [`adaptive_reprime_interval_ms`] on the *next*
    /// cadence-gate check.  Using the last-prime byte count (rather than
    /// recomputing from the scene on every deferred frame) keeps the gate
    /// decision O(1) with no scene traversal on deferred frames.
    ///
    /// Initialized to 0 so the first prime always uses the short interval
    /// (which is effectively bypassed because `resize_reprime_last_at` is
    /// `None` and the gate never defers on the first call).
    resize_reprime_content_bytes: usize,
    /// Per-surface video state machines (v2 media plane, E26 / B11).
    ///
    /// Keyed by the `SceneId` carried in `ZoneContent::VideoSurfaceRef`.
    /// In v1 builds (no `v2_preview` feature) this is a zero-cost empty
    /// stub that always returns `VideoRenderState::Placeholder`.
    ///
    /// The runtime delivers [`crate::video_surface::MediaEvent`]s to this
    /// map via [`Compositor::handle_media_event`].  The render path queries
    /// it each frame via [`crate::video_surface::VideoSurfaceMap::render_state_for`].
    pub video_surfaces: crate::video_surface::VideoSurfaceMap,
    /// GPU texture cache for decoded video frames (v2 media plane, `v2_preview` only).
    ///
    /// Keyed by the `SceneId` carried in `ZoneContent::VideoSurfaceRef`.
    /// One entry per active video surface; populated by
    /// [`Compositor::upload_video_frame`] and evicted when the surface
    /// transitions to a terminal state ([`Compositor::evict_video_frame_texture`]).
    ///
    /// In v1 builds (no `v2_preview` feature) this field is absent; the render
    /// path always falls back to the dark placeholder quad.
    #[cfg(feature = "v2_preview")]
    pub(crate) video_frame_cache: HashMap<tze_hud_scene::types::SceneId, ImageTextureEntry>,
    /// Shared handle for receiving local composer echo state from the
    /// input-event thread (main thread) without blocking on the scene lock.
    ///
    /// Drained at the start of each frame loop iteration in the compositor
    /// thread.  `None` when no composer is active or no draft has arrived
    /// for the current frame.
    pub local_composer_state: LocalComposerStateHandle,
    /// Most-recently drained local composer state for this frame.
    ///
    /// Populated by draining `local_composer_state` at frame start.
    /// Consumed by `collect_text_items` (text pass) and the geometry pass
    /// (background rect + caret rect).  Cleared when the handle delivers
    /// `None` (composer deactivated).
    pub(crate) local_composer: Option<LocalComposerState>,
}

/// Partitioned rounded-rectangle draw commands organized by layer.
pub(crate) struct LayerPartitionedRoundedRectCmds {
    pub(crate) background: Vec<RoundedRectDrawCmd>,
    pub(crate) content: Vec<RoundedRectDrawCmd>,
    pub(crate) chrome: Vec<RoundedRectDrawCmd>,
}

/// Per-zone Stack slot layout, computed once per zone per frame by
/// [`Compositor::zone_slot_layout`] and consumed by all zone-rendering call
/// sites that need slot geometry: `collect_text_items`, `render_zone_content`,
/// and `collect_all_rounded_rect_cmds`.
///
/// Centralising this computation eliminates the hand-mirrored duplication that
/// was previously guarded by a "must-mirror" comment (hud-lu50e / hud-qlerb).
#[derive(Debug)]
pub(crate) struct ZoneSlotLayout {
    /// Publication indices sorted by the zone's ordering rule (alert-banner:
    /// severity-descending, then recency; others: newest-first / reverse order).
    pub(crate) ordered_indices: Vec<usize>,
    /// Per-slot heights in the same order as `ordered_indices`.
    /// Two-line notifications produce a taller slot; all others use
    /// `stack_slot_height`.
    pub(crate) slot_heights: Vec<f32>,
    /// Cumulative y-offsets from the zone origin, one per slot.
    /// `slot_offsets[i]` is the y-start of slot `i` relative to `zy`.
    pub(crate) slot_offsets: Vec<f32>,
    /// Effective zone height in pixels.
    /// For alert-banner zones this equals `sum(slot_heights)` (dynamic growth).
    /// For all other Stack zones this equals the configured zone height `zh`.
    pub(crate) effective_h: f32,
}

impl ZoneSlotLayout {
    /// Iterate `(pub_idx, slot_offset, slot_h)` triples for the slots that
    /// fall inside the zone.  Slots whose top edge is at or beyond `zy +
    /// effective_h` are excluded (the iterator stops early).
    pub(crate) fn iter_visible(&self, zy: f32) -> impl Iterator<Item = (usize, f32, f32)> + '_ {
        let zone_bottom = zy + self.effective_h;
        self.ordered_indices
            .iter()
            .copied()
            .zip(
                self.slot_offsets
                    .iter()
                    .copied()
                    .zip(self.slot_heights.iter().copied()),
            )
            .take_while(move |(_, (offset, _))| zy + offset < zone_bottom)
            .map(move |(pub_idx, (offset, slot_h))| {
                let slot_y = zy + offset;
                let effective_slot_h = slot_h.min(zone_bottom - slot_y);
                (pub_idx, slot_y, effective_slot_h)
            })
    }
}

impl Compositor {
    /// Create a new headless compositor.
    ///
    /// Checks the `HEADLESS_FORCE_SOFTWARE` environment variable.  When set to
    /// `1`, wgpu adapter selection uses `force_fallback_adapter = true`, which
    /// selects llvmpipe on Linux or WARP on Windows.
    ///
    /// Per runtime-kernel/spec.md Requirement: Headless Software GPU (line 211):
    /// "When set, the wgpu adapter selection MUST request a software fallback
    /// (force_fallback_adapter = true)."
    pub async fn new_headless(width: u32, height: u32) -> Result<Self, CompositorError> {
        // Check HEADLESS_FORCE_SOFTWARE env var (spec line 409: "conventionally
        // named HEADLESS_FORCE_SOFTWARE").
        let force_software = std::env::var("HEADLESS_FORCE_SOFTWARE")
            .map(|v| v.trim() == "1")
            .unwrap_or(false);

        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(),
            ..Default::default()
        });

        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::LowPower,
                compatible_surface: None,
                force_fallback_adapter: force_software,
            })
            .await
            .ok_or(CompositorError::NoAdapter)?;

        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    label: Some("tze_hud_compositor"),
                    required_features: wgpu::Features::empty(),
                    required_limits: wgpu::Limits::downlevel_defaults(),
                    memory_hints: wgpu::MemoryHints::default(),
                },
                None,
            )
            .await
            .map_err(|e| CompositorError::DeviceCreation(e.to_string()))?;

        let pipeline = Self::create_pipeline(&device);
        let clear_pipeline =
            Self::create_clear_pipeline(&device, wgpu::TextureFormat::Rgba8UnormSrgb);
        let rounded_rect_pipeline =
            Self::create_rounded_rect_pipeline(&device, wgpu::TextureFormat::Rgba8UnormSrgb);
        let rounded_rect_overlay_pipeline = Self::create_rounded_rect_overlay_pipeline(
            &device,
            wgpu::TextureFormat::Rgba8UnormSrgb,
        );

        let texture_rect_bind_group_layout = create_texture_rect_bind_group_layout(&device);
        let texture_rect_pipeline = create_texture_rect_pipeline(
            &device,
            &texture_rect_bind_group_layout,
            wgpu::TextureFormat::Rgba8UnormSrgb,
        );
        let image_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("image_linear_sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        Ok(Self {
            device,
            queue,
            pipeline,
            clear_pipeline,
            texture_rect_pipeline,
            texture_rect_bind_group_layout,
            image_sampler,
            rounded_rect_pipeline,
            rounded_rect_overlay_pipeline,
            width,
            height,
            frame_number: 0,
            overlay_mode: false,
            debug_zone_tints: std::env::var("TZE_HUD_DEBUG_ZONES").is_ok_and(|v| v == "1"),
            degradation_level: DegradationLevel::Nominal,
            text_rasterizer: None,
            widget_renderer: None,
            zone_animation_states: HashMap::new(),
            prev_active_zones: HashMap::new(),
            portal_tile_anim_states: HashMap::new(),
            prev_portal_tile_has_content: HashMap::new(),
            pub_animation_states: HashMap::new(),
            stream_reveal_states: HashMap::new(),
            token_map: HashMap::new(),
            markdown_primer: crate::markdown::MarkdownPrimer::new(),
            node_key_cache: HashMap::new(),
            markdown_tokens: crate::markdown::MarkdownTokens::default(),
            markdown_cache_scene_version: u64::MAX,
            truncation_cache_scene_version: u64::MAX,
            resize_reprime_last_at: None,
            resize_reprime_content_bytes: 0,
            image_bytes: HashMap::new(),
            image_dims: HashMap::new(),
            image_texture_cache: HashMap::new(),
            failed_icon_paths: HashSet::new(),
            video_surfaces: crate::video_surface::VideoSurfaceMap::new(),
            #[cfg(feature = "v2_preview")]
            video_frame_cache: HashMap::new(),
            local_composer_state: Arc::new(StdMutex::new(None)),
            local_composer: None,
        })
    }

    /// Create a windowed compositor backed by a real `winit::window::Window`.
    ///
    /// This is the factory method for production windowed rendering. It:
    /// 1. Uses `select_gpu_adapter` with platform-mandated backends (Vulkan/D3D12/Metal).
    /// 2. Creates a `wgpu::Surface` from the window via `instance.create_surface`.
    /// 3. Negotiates the surface format (sRGB preferred).
    /// 4. Configures the surface with the window's physical dimensions.
    /// 5. Creates the `wgpu::Device` and `wgpu::Queue`.
    ///
    /// Returns the `(Compositor, WindowSurface)` pair. The `WindowSurface` must
    /// be kept alive for the duration of the runtime.
    ///
    /// Per spec §Compositor Thread Ownership (line 46): the returned `Compositor`
    /// (and thus `Device` + `Queue`) MUST be transferred to the compositor thread
    /// immediately after creation. The `WindowSurface` is owned by the main thread.
    ///
    /// Per spec §Platform GPU Backends (line 189): this path uses the platform-
    /// mandated backends — unlike `new_headless` which uses `Backends::all()`.
    pub async fn new_windowed(
        window: std::sync::Arc<winit::window::Window>,
        width: u32,
        height: u32,
    ) -> Result<(Self, crate::surface::WindowSurface), CompositorError> {
        Self::new_windowed_inner(window, width, height, false).await
    }

    /// Create a windowed compositor, optionally forcing Vulkan for overlay
    /// transparency (DX12 only supports Opaque swapchain alpha mode).
    pub async fn new_windowed_overlay(
        window: std::sync::Arc<winit::window::Window>,
        width: u32,
        height: u32,
    ) -> Result<(Self, crate::surface::WindowSurface), CompositorError> {
        Self::new_windowed_inner(window, width, height, true).await
    }

    async fn new_windowed_inner(
        window: std::sync::Arc<winit::window::Window>,
        width: u32,
        height: u32,
        overlay: bool,
    ) -> Result<(Self, crate::surface::WindowSurface), CompositorError> {
        use crate::surface::WindowSurface;

        // ── Step 1: Create instance with platform-mandated backends ──────────
        // We need the surface before adapter selection so we can pass it as
        // `compatible_surface`. Create a temporary instance first, create the
        // surface, then select the adapter with that surface constraint.
        // On Windows in overlay mode, force Vulkan — DX12 only supports Opaque
        // swapchain alpha mode, which prevents per-pixel transparency.
        let backends = if overlay && cfg!(target_os = "windows") {
            tracing::info!("overlay mode: forcing Vulkan backend for transparent swapchain");
            wgpu::Backends::VULKAN
        } else {
            crate::adapter::platform_backends().flags
        };
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends,
            ..Default::default()
        });

        // ── Step 2: Create wgpu::Surface from the winit window ───────────────
        // SAFETY: `window` is wrapped in Arc — it outlives the surface because
        // we pass 'static lifetime via Arc<Window>.
        let surface = instance
            .create_surface(window.clone())
            .map_err(|e| CompositorError::DeviceCreation(format!("create_surface: {e}")))?;

        // ── Step 3: Select adapter compatible with the surface ────────────────
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
            })
            .await
            .ok_or(CompositorError::NoAdapter)?;

        let adapter_info = adapter.get_info();
        tracing::info!(
            backend = ?adapter_info.backend,
            device_name = %adapter_info.name,
            vendor = adapter_info.vendor,
            "windowed: GPU adapter selected"
        );

        // ── Step 4: Request device ────────────────────────────────────────────
        // Use downlevel_defaults() for broad compatibility but override
        // max_texture_dimension_2d with the adapter's actual capability.
        // downlevel_defaults() caps this at 2048 which is smaller than
        // common display resolutions (e.g. 2560x1440).  The adapter knows
        // the GPU's true limit; requesting it ensures the surface can be
        // configured at the monitor's native resolution.
        let mut required_limits = wgpu::Limits::downlevel_defaults();
        required_limits.max_texture_dimension_2d = adapter.limits().max_texture_dimension_2d;

        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    label: Some("tze_hud_compositor_windowed"),
                    required_features: wgpu::Features::empty(),
                    required_limits,
                    memory_hints: wgpu::MemoryHints::default(),
                },
                None,
            )
            .await
            .map_err(|e| CompositorError::DeviceCreation(e.to_string()))?;

        // ── Step 5: Configure the surface ────────────────────────────────────
        let surface_caps = surface.get_capabilities(&adapter);

        // Guard: wgpu surface capabilities must be non-empty on a valid
        // adapter/surface combination. Return a structured error instead of
        // panicking via index [0] so the caller can diagnose driver issues.
        if surface_caps.formats.is_empty() {
            return Err(CompositorError::DeviceCreation(
                "surface reports no supported texture formats — driver or backend issue"
                    .to_string(),
            ));
        }
        if surface_caps.present_modes.is_empty() {
            return Err(CompositorError::DeviceCreation(
                "surface reports no supported present modes — driver or backend issue".to_string(),
            ));
        }
        if surface_caps.alpha_modes.is_empty() {
            return Err(CompositorError::DeviceCreation(
                "surface reports no supported alpha modes — driver or backend issue".to_string(),
            ));
        }

        // Prefer sRGB surface format; fall back to the first available format.
        let surface_format = surface_caps
            .formats
            .iter()
            .copied()
            .find(|f| f.is_srgb())
            .unwrap_or(surface_caps.formats[0]);

        let present_mode = if surface_caps
            .present_modes
            .contains(&wgpu::PresentMode::Fifo)
        {
            wgpu::PresentMode::Fifo // vsync — latency-stable
        } else {
            surface_caps.present_modes[0]
        };

        // Clamp dimensions to the device's maximum supported texture size.
        // Use device.limits() (not adapter.limits()) because the device is
        // created with required_limits=downlevel_defaults(); the actual device
        // limits reflect what the adapter provides subject to those requirements.
        // Some GPUs (e.g. certain Intel/Mesa drivers) report a max of 2048,
        // which is smaller than common display resolutions like 2560x1440.
        //
        // Also guard against zero-size dimensions: wgpu panics if width or
        // height is 0 in surface.configure().  This can happen if inner_size()
        // returned (0,0) on a minimized or not-yet-shown window.
        let max_dim = device.limits().max_texture_dimension_2d;
        let clamped_width = width.min(max_dim).max(1);
        let clamped_height = height.min(max_dim).max(1);
        if clamped_width != width || clamped_height != height {
            tracing::warn!(
                requested_width = width,
                requested_height = height,
                clamped_width,
                clamped_height,
                max_texture_dimension_2d = max_dim,
                "windowed: surface dimensions clamped to device limit"
            );
        }

        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: surface_format,
            width: clamped_width,
            height: clamped_height,
            present_mode,
            alpha_mode: surface_caps
                .alpha_modes
                .iter()
                .find(|m| **m == wgpu::CompositeAlphaMode::PreMultiplied)
                .or_else(|| {
                    surface_caps
                        .alpha_modes
                        .iter()
                        .find(|m| **m == wgpu::CompositeAlphaMode::PostMultiplied)
                })
                .copied()
                .unwrap_or(surface_caps.alpha_modes[0]),
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        tracing::info!(
            available_alpha_modes = ?surface_caps.alpha_modes,
            selected_alpha_mode = ?config.alpha_mode,
            "windowed: alpha mode selection"
        );
        surface.configure(&device, &config);
        tracing::info!(
            format = ?surface_format,
            present_mode = ?present_mode,
            alpha_mode = ?config.alpha_mode,
            width = clamped_width,
            height = clamped_height,
            "windowed: surface configured"
        );

        // ── Step 6: Create render pipelines (format-aware) ────────────────────
        let pipeline = Self::create_pipeline_with_format(&device, surface_format);
        let clear_pipeline = Self::create_clear_pipeline(&device, surface_format);
        let rounded_rect_pipeline = Self::create_rounded_rect_pipeline(&device, surface_format);
        let rounded_rect_overlay_pipeline =
            Self::create_rounded_rect_overlay_pipeline(&device, surface_format);

        let texture_rect_bind_group_layout = create_texture_rect_bind_group_layout(&device);
        let texture_rect_pipeline =
            create_texture_rect_pipeline(&device, &texture_rect_bind_group_layout, surface_format);
        let image_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("image_linear_sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let compositor = Self {
            device,
            queue,
            pipeline,
            clear_pipeline,
            texture_rect_pipeline,
            texture_rect_bind_group_layout,
            image_sampler,
            rounded_rect_pipeline,
            rounded_rect_overlay_pipeline,
            width: clamped_width,
            height: clamped_height,
            frame_number: 0,
            overlay_mode: false,
            debug_zone_tints: std::env::var("TZE_HUD_DEBUG_ZONES").is_ok_and(|v| v == "1"),
            degradation_level: DegradationLevel::Nominal,
            text_rasterizer: None,
            widget_renderer: None,
            zone_animation_states: HashMap::new(),
            prev_active_zones: HashMap::new(),
            portal_tile_anim_states: HashMap::new(),
            prev_portal_tile_has_content: HashMap::new(),
            pub_animation_states: HashMap::new(),
            stream_reveal_states: HashMap::new(),
            token_map: HashMap::new(),
            markdown_primer: crate::markdown::MarkdownPrimer::new(),
            node_key_cache: HashMap::new(),
            markdown_tokens: crate::markdown::MarkdownTokens::default(),
            markdown_cache_scene_version: u64::MAX,
            truncation_cache_scene_version: u64::MAX,
            resize_reprime_last_at: None,
            resize_reprime_content_bytes: 0,
            image_bytes: HashMap::new(),
            image_dims: HashMap::new(),
            image_texture_cache: HashMap::new(),
            failed_icon_paths: HashSet::new(),
            video_surfaces: crate::video_surface::VideoSurfaceMap::new(),
            #[cfg(feature = "v2_preview")]
            video_frame_cache: HashMap::new(),
            local_composer_state: Arc::new(StdMutex::new(None)),
            local_composer: None,
        };

        let window_surface = WindowSurface::new(surface, config);
        Ok((compositor, window_surface))
    }

    /// Create a render pipeline targeting a specific texture format.
    ///
    /// This is the canonical pipeline constructor. Both `create_pipeline`
    /// (headless, fixed format) and `create_pipeline_with_format` (windowed,
    /// dynamic swapchain format) delegate here to avoid duplicating the
    /// pipeline descriptor.
    ///
    /// `label_prefix` is prepended to debug labels so GPU profilers can
    /// distinguish headless vs windowed pipelines.
    fn create_pipeline_inner(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        label_prefix: &str,
    ) -> wgpu::RenderPipeline {
        use crate::pipeline::{RECT_SHADER, RectVertex};

        let shader_label = format!("{label_prefix}rect_shader");
        let layout_label = format!("{label_prefix}rect_pipeline_layout");
        let pipeline_label = format!("{label_prefix}rect_pipeline");

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some(&shader_label),
            source: wgpu::ShaderSource::Wgsl(RECT_SHADER.into()),
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some(&layout_label),
            bind_group_layouts: &[],
            push_constant_ranges: &[],
        });

        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(&pipeline_label),
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
                    format,
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

    /// Create a render pipeline targeting a dynamic swapchain format.
    ///
    /// Called by `new_windowed` so the pipeline matches the negotiated
    /// surface format (which varies by platform/driver).
    fn create_pipeline_with_format(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
    ) -> wgpu::RenderPipeline {
        Self::create_pipeline_inner(device, format, "windowed_")
    }

    /// Create a pipeline with no blending — writes RGBA directly.
    /// Used to clear the framebuffer to transparent in overlay mode.
    fn create_clear_pipeline(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
    ) -> wgpu::RenderPipeline {
        use crate::pipeline::{RECT_SHADER, RectVertex};
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("clear_shader"),
            source: wgpu::ShaderSource::Wgsl(RECT_SHADER.into()),
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("clear_pipeline_layout"),
            bind_group_layouts: &[],
            push_constant_ranges: &[],
        });
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("clear_pipeline"),
            layout: Some(&layout),
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
                    format,
                    blend: None, // No blending — direct RGBA write
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

    /// Create a render pipeline for headless mode (`Rgba8UnormSrgb`).
    fn create_pipeline(device: &wgpu::Device) -> wgpu::RenderPipeline {
        Self::create_pipeline_inner(device, wgpu::TextureFormat::Rgba8UnormSrgb, "")
    }

    /// Create the SDF rounded-rectangle pipeline for the given output format.
    ///
    /// Uses `ROUNDED_RECT_SHADER` and `RoundedRectVertex::desc()`.
    /// Alpha blending is enabled (standard source-over) so rounded corners
    /// composite correctly over any background.
    ///
    /// This pipeline is used in `encode_rounded_rect_pass` for zones whose
    /// `RenderingPolicy` has `backdrop_radius` set.
    fn create_rounded_rect_pipeline(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
    ) -> wgpu::RenderPipeline {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("rounded_rect_shader"),
            source: wgpu::ShaderSource::Wgsl(ROUNDED_RECT_SHADER.into()),
        });

        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("rounded_rect_pipeline_layout"),
            bind_group_layouts: &[],
            push_constant_ranges: &[],
        });

        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("rounded_rect_pipeline"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[RoundedRectVertex::desc()],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
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

    /// Create the SDF rounded-rectangle pipeline for overlay / premultiplied-alpha mode.
    ///
    /// Uses `ROUNDED_RECT_OVERLAY_SHADER` (which scales all four RGBA channels by
    /// SDF coverage) and `BlendState::PREMULTIPLIED_ALPHA_BLENDING`.  Selected by
    /// `encode_rounded_rect_pass` when `self.overlay_mode` is true.
    ///
    /// In overlay mode vertex colors are premultiplied by `gpu_color`; the shader
    /// must therefore output `(rgb * cov, a * cov)` so the premultiplied blend
    /// equation applies coverage exactly once without double-multiplying alpha.
    fn create_rounded_rect_overlay_pipeline(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
    ) -> wgpu::RenderPipeline {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("rounded_rect_overlay_shader"),
            source: wgpu::ShaderSource::Wgsl(ROUNDED_RECT_OVERLAY_SHADER.into()),
        });

        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("rounded_rect_overlay_pipeline_layout"),
            bind_group_layouts: &[],
            push_constant_ranges: &[],
        });

        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("rounded_rect_overlay_pipeline"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[RoundedRectVertex::desc()],
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

    /// Replace the compositor's resolved design token map.
    ///
    /// Should be called once at startup after `run_component_startup` produces a
    /// `ComponentStartupResult::global_tokens`.  The map is keyed by canonical
    /// token names (e.g. `"color.severity.warning"`) with hex-color string values
    /// (e.g. `"#FFB800"`).
    ///
    /// At render time the compositor looks up `color.severity.{info,warning,critical}`
    /// to derive alert-banner backdrop colors, falling back to hardcoded constants
    /// when a key is absent or unparseable.
    pub fn set_token_map(&mut self, map: HashMap<String, String>) {
        // Rebuild markdown tokens from the new map so heading weights, link
        // colors, and code family are in sync with the current theme.
        // Clear the markdown cache so existing entries are re-parsed with the
        // new token values on the next prime() call.
        self.markdown_tokens = crate::markdown::MarkdownTokens::from_token_map(&map);
        self.markdown_primer.reset();
        // Clear the per-node key cache: it will be repopulated by the next
        // prime_markdown_cache call once the version sentinel fires.
        self.node_key_cache.clear();
        // Reset the version sentinel so the next prime_markdown_cache call
        // re-parses all nodes with the updated tokens, even if scene.version
        // has not changed since the last prime.
        self.markdown_cache_scene_version = u64::MAX;

        // Clear the truncation cache: a token-map change can alter font metrics
        // (monospace vs. sans-serif substitution, heading-scale changes) which
        // affects the shaped width and thus the truncation point.  Re-prime
        // unconditionally on the next frame.
        if let Some(rasterizer) = &mut self.text_rasterizer {
            rasterizer.truncation_cache.clear();
        }
        self.truncation_cache_scene_version = u64::MAX;

        self.token_map = map;
        // Token-substitution failures in ensure_icon_texture are tied to the
        // token map state, not to the SVG file on disk.  Clearing the negative
        // cache here lets icons that previously failed due to a missing token
        // be retried with the updated map (e.g. on hot-reload or late init).
        self.failed_icon_paths.clear();
    }

    /// Drain any pending [`LocalComposerState`] update from the shared handle.
    ///
    /// Called once per frame at the top of the compositor render loop.
    ///
    /// - `Some(Some(state))` in the slot → update `self.local_composer`.
    /// - `Some(None)` in the slot → clear `self.local_composer` (composer
    ///   deactivated by blur / submit / cancel).
    /// - `None` in the slot → no update; keep the current frame state so the
    ///   overlay persists between keystrokes.
    ///
    /// Taking the value (`.take()`) resets the slot to `None` so the
    /// same update is not applied twice.
    pub fn drain_local_composer_state(&mut self) {
        apply_composer_slot(&self.local_composer_state, &mut self.local_composer);
    }

    /// Prime the markdown parse cache for all [`TextMarkdownNode`] nodes in the scene.
    ///
    /// **Call site**: this must be called by the **runtime at Stage 4 (scene
    /// commit time)**, before `render_frame` or `render_frame_headless` is
    /// invoked.  Callers are responsible for measuring the cost and recording it
    /// as `FrameTelemetry::markdown_prime_us` (hud-380dl, Option A).
    ///
    /// The method is gated internally on `scene.version` so repeated calls on
    /// unchanged scenes are no-ops (constant-time, no allocation).
    ///
    /// After this call returns, the frame pipeline can consume cached results via
    /// [`MarkdownCache::get_by_key`] with zero parse cost — satisfying the
    /// "parse-on-commit, zero per-frame parse cost" contract (hud-5jbra.2,
    /// hud-380dl).
    ///
    /// The method also evicts cache entries for content no longer referenced by
    /// any scene node, keeping the cache bounded to the live node set.
    ///
    /// [`TextMarkdownNode`]: tze_hud_scene::types::TextMarkdownNode
    pub fn prime_markdown_cache(&mut self, scene: &SceneGraph) {
        use tze_hud_scene::types::NodeData;

        // Skip if the scene has not changed since we last primed.
        if scene.version == self.markdown_cache_scene_version {
            return;
        }
        self.markdown_cache_scene_version = scene.version;

        // Snapshot the current cache once so we can decide, per node, whether
        // its content still needs parsing.  Already-cached entries are carried
        // forward by the primer without re-cloning their source string
        // (hud-33qo7); only cache misses ship their `Arc<str>` content.
        let current = self.markdown_primer.load();

        // Build the complete live job set and repopulate the per-node key cache
        // (hud-gpqde) in the same pass — eliminating per-frame re-hashes in
        // collect_text_items_from_node.
        let mut jobs: Vec<crate::markdown::PrimeJob> = Vec::new();

        // Rebuild the node key cache for this scene version.
        self.node_key_cache.clear();

        for (node_id, node) in scene.nodes.iter() {
            if let NodeData::TextMarkdown(tm) = &node.data {
                let key = crate::markdown::MarkdownCache::compute_key(&tm.content);
                // Store the precomputed key so the frame path can look it up
                // in O(1) without re-hashing content.
                self.node_key_cache.insert(*node_id, key);
                // Attach content only for cache misses; hits carry forward by
                // key with no source clone.
                let content = if current.get_by_key(&key).is_none() {
                    Some(std::sync::Arc::<str>::from(tm.content.as_str()))
                } else {
                    None
                };
                jobs.push(crate::markdown::PrimeJob { key, content });
            }
        }

        // Hand the complete live set to the primer.  It parses any missing
        // content (inline for small payloads, on the background thread for large
        // ones), atomically swaps in the new snapshot, and evicts dead entries
        // implicitly (the new snapshot is exactly the live set).  Gated on
        // scene.version so a late background result never clobbers a newer one.
        self.markdown_primer
            .prime(jobs, &self.markdown_tokens, scene.version);
    }

    /// Load the current markdown parse-cache snapshot lock-free (hud-33qo7).
    ///
    /// Render-path callers use this to read [`ParsedMarkdown`] via
    /// [`MarkdownCache::get_by_key`].  The returned [`Arc`] pins the snapshot for
    /// the duration of the borrow, so a concurrent background swap cannot free it
    /// mid-read.  Hold it across a traversal (it is a cheap atomic load) rather
    /// than re-loading per node.
    ///
    /// [`MarkdownCache::get_by_key`]: crate::markdown::MarkdownCache::get_by_key
    /// [`ParsedMarkdown`]: crate::markdown::ParsedMarkdown
    #[inline]
    pub(crate) fn markdown_cache(&self) -> std::sync::Arc<crate::markdown::MarkdownCache> {
        self.markdown_primer.load()
    }

    /// Prime the ellipsis-truncation cache for all `TextOverflow::Ellipsis` items
    /// currently in the scene (hud-wgq7j, hud-ghhxa).
    ///
    /// Must be called **at content-commit time** (when `scene.version` changes),
    /// never on the per-frame render path.  The method is gated internally on
    /// `scene.version` so repeated calls on unchanged scenes are no-ops.
    ///
    /// After this call, `prepare_text_items` resolves each Ellipsis item in O(1)
    /// via `TruncationCache::get_by_key` — satisfying the "stable-shape-caching"
    /// contract from RFC 0013 §3.4 and §4.2, Phase-1 design §3.
    ///
    /// # Mid-drag cadence (hud-ghhxa — spec §6b.3)
    ///
    /// When a portal tile is resized (hotkey or pointer drag), `scene.version`
    /// increments on every geometry change.  Because `truncate_line_to_ellipsis`
    /// is O(n) in content length, re-priming every frame during a fast drag
    /// would blow the Stage 5 / Stage 6 frame budget on large content.
    ///
    /// The adaptive cadence gate caps re-primes to at most once per the
    /// content-length-derived interval (see [`adaptive_reprime_interval_ms`])
    /// while guaranteeing that every distinct intermediate geometry is
    /// *eventually* reflected — not only at drag-end.  When the geometry
    /// settles (scene.version stops changing), the last geometry is primed on
    /// the next frame after the interval elapses.
    pub fn prime_truncation_cache(&mut self, scene: &SceneGraph) {
        // Skip if the scene has not changed since we last primed.
        if scene.version == self.truncation_cache_scene_version {
            return;
        }

        // ── Adaptive mid-drag cadence gate (hud-ghhxa, hud-3to8i) ───────────
        // When geometry changes rapidly (bounds updates from resize), cap the
        // re-prime rate to an adaptive interval derived from the total byte
        // count of Ellipsis text content visible in the scene.  This avoids
        // per-frame O(n) shaping cost on large content while keeping truncation
        // visually responsive for short/cheap content.
        //
        // Strategy: if a prime ran within the adaptive interval, defer — leave
        // the sentinel unchanged so the next frame will retry.  When the
        // interval has elapsed (or this is the very first prime), proceed.
        //
        // The adaptive interval is derived from `resize_reprime_content_bytes`,
        // which holds the total content bytes from the *last* successful prime.
        // Using the cached value keeps this check O(1) with no scene traversal
        // on deferred frames (frame-loop contract: the adaptive decision itself
        // must be cheap).
        //
        // This guarantees:
        //  a) The first geometry change in each interval triggers an immediate
        //     re-prime (the TruncationKey includes bounds, so the new geometry
        //     will be primed on first visit).
        //  b) Intermediate geometries *between* interval ticks are not orphaned:
        //     the sentinel is not updated on a defer, so the most-recent
        //     scene.version is always picked up on the next allowed tick.
        //  c) At drag-end (geometry settles), the final geometry is primed on the
        //     next tick after the interval elapses — completing the re-resolution
        //     required by spec §6b.3.
        //
        // The cadence gate is bypassed when the sentinel is `u64::MAX` (a forced
        // re-prime requested by `set_token_map` or initialisation) so that token/
        // font-metric changes are always reflected immediately regardless of resize
        // cadence.
        let forced_prime = self.truncation_cache_scene_version == u64::MAX;
        let interval_ms = adaptive_reprime_interval_ms(self.resize_reprime_content_bytes);
        if !forced_prime && should_defer_reprime(self.resize_reprime_last_at, interval_ms) {
            // Within the debounce window: defer, do not update the sentinel.
            tracing::trace!(
                scene_version = scene.version,
                interval_ms,
                content_bytes = self.resize_reprime_content_bytes,
                "prime_truncation_cache: mid-drag cadence defer (within adaptive interval)"
            );
            return;
        }

        // Build the set of TextItems with Ellipsis overflow that are currently
        // reachable in the scene.  We need TextItem geometry, which is the same
        // geometry produced by `collect_text_items_from_node`.  Rather than
        // duplicating that traversal here, we ask each tile's node tree for its
        // Ellipsis TextMarkdownNodes and reconstruct the same geometry that
        // `collect_text_items_from_node` would compute.
        //
        // NOTE: the scene-version sentinel is updated only after confirming the
        // rasterizer is available.  If we returned early here (rasterizer = None)
        // while recording the version, subsequent frames would skip priming even
        // once the rasterizer is initialized — until the scene changes again.
        if self.text_rasterizer.is_none() {
            return; // text renderer not yet initialized; retry next version change
        }

        // Zone StreamText items are not produced by the tile-node walk below, so
        // prime them separately.  Without this, an overflowing streaming zone
        // would hit the render path with no cache entry and fall back to raw
        // clipping (head-anchored), never showing the tail (hud-gxz0x).  Zone
        // geometry resolves against the scene's display area — kept in sync with
        // the surface size (windowed.rs::sync_scene_display_area), so the primed
        // geometry matches what `collect_text_items` produces on the frame path.
        //
        // Computed here (before the &mut self.text_rasterizer borrow below) so
        // the &self collector does not conflict with that borrow.
        let zone_stream_items = self.collect_zone_stream_text_ellipsis_items(
            scene,
            scene.display_area.width,
            scene.display_area.height,
        );

        // Record the version and timestamp now that priming will proceed.
        self.truncation_cache_scene_version = scene.version;
        self.resize_reprime_last_at = Some(std::time::Instant::now());

        // Safe: checked is_none() above and returned; rasterizer stays Some
        // (this method holds &mut self exclusively).
        let rasterizer = self
            .text_rasterizer
            .as_mut()
            .expect("text rasterizer presence checked above");

        // Load the markdown snapshot once (lock-free atomic load) and pin it for
        // the whole traversal — a concurrent background swap cannot free it
        // mid-read (hud-33qo7).
        let markdown_cache = self.markdown_primer.load();
        let mut live_items: Vec<crate::text::TextItem> = Vec::new();
        for tile in scene.visible_tiles() {
            let tile_x = tile.bounds.x;
            let tile_y = tile.bounds.y;
            // Determine whether this tile is currently following its tail.
            // Use the shared helper so this computation is identical to
            // collect_text_items_from_node (hud-plz8q / hud-lu50e).
            let at_tail = tile_at_tail_for_ellipsis(tile.id, scene);
            if let Some(root_id) = tile.root_node {
                collect_ellipsis_text_items_from_node(
                    root_id,
                    scene,
                    tile_x,
                    tile_y,
                    at_tail,
                    &markdown_cache,
                    &self.node_key_cache,
                    &self.markdown_tokens,
                    &mut live_items,
                );
            }
        }

        // Append the zone StreamText Ellipsis items computed before the
        // rasterizer borrow (see comment above) so they are primed alongside the
        // tile-node items.
        live_items.extend(zone_stream_items);

        // Update the cached content byte count for the adaptive cadence gate on
        // the *next* call.  Summing item lengths is O(n) in item count here, but
        // this only runs when a prime is allowed (not on every deferred frame),
        // so it stays within the prime-time O(n) budget.
        self.resize_reprime_content_bytes = live_items.iter().map(|item| item.text.len()).sum();

        let mut live_keys = rasterizer.prime_truncation_cache(&live_items);

        // Evict stale truncation entries for nodes/geometry no longer in the scene.
        //
        // Deduplicate live_keys before the length comparison: duplicate scene
        // items (same content + geometry) produce the same TruncationKey.
        // Without dedup, `live_keys.len()` may exceed the number of distinct
        // cache entries, causing the `len > live_keys.len()` condition to be
        // false even when stale entries exist — a memory leak.
        live_keys.sort_unstable();
        live_keys.dedup();
        if rasterizer.truncation_cache.len() > live_keys.len() {
            rasterizer.truncation_cache.evict_except(&live_keys);
        }
    }

    /// Scan the scene graph for all StaticImage resources, ensure their GPU
    /// textures are uploaded, and return the set of referenced `ResourceId`s.
    ///
    /// This method uploads textures for tile-backed static images directly,
    /// and for zone-publication images when dimensions can be inferred from
    /// registered RGBA bytes.
    ///
    /// The returned `HashSet<ResourceId>` should be passed to
    /// [`Compositor::evict_unused_image_textures`] once per frame so that
    /// stale cache entries are reclaimed and the cache does not grow without
    /// bound across frames.
    fn ensure_scene_image_textures(&mut self, scene: &SceneGraph) -> HashSet<ResourceId> {
        let mut referenced: HashSet<ResourceId> = HashSet::new();

        // Collect all referenced StaticImage resource IDs from tiles.
        for node in scene.nodes.values() {
            if let NodeData::StaticImage(img) = &node.data {
                self.ensure_image_texture(img.resource_id, img.width, img.height);
                referenced.insert(img.resource_id);
            }
        }

        // Collect from zone publications.
        for publishes in scene.zone_registry.active_publishes.values() {
            for record in publishes {
                match &record.content {
                    ZoneContent::StaticImage(resource_id) => {
                        referenced.insert(*resource_id);
                        // Use the explicit dimensions stored by `register_image_bytes`.
                        // This correctly handles non-square images (e.g. 640×360) that
                        // the old square-root heuristic would mis-detect.
                        if !self.image_texture_cache.contains_key(resource_id) {
                            if let Some(&(w, h)) = self.image_dims.get(resource_id) {
                                if w > 0 && h > 0 {
                                    self.ensure_image_texture(*resource_id, w, h);
                                }
                            }
                        }
                    }
                    ZoneContent::Notification(payload) if !payload.icon.is_empty() => {
                        // Parse the icon field via the shared helper (handles empty string
                        // and invalid hex with graceful None). Non-parseable icons are
                        // silently skipped (render notification text-only).
                        if let Some(resource_id) = parse_notification_icon(&payload.icon) {
                            referenced.insert(resource_id);
                            // Use the explicit dimensions stored by `register_image_bytes`.
                            if !self.image_texture_cache.contains_key(&resource_id) {
                                if let Some(&(w, h)) = self.image_dims.get(&resource_id) {
                                    if w > 0 && h > 0 {
                                        self.ensure_image_texture(resource_id, w, h);
                                    }
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
        }

        referenced
    }

    /// Scan the scene's zone registry for all `key_icon_map` SVG paths,
    /// ensure their GPU textures are cached, and return the set of their
    /// path-derived `ResourceId`s.
    ///
    /// The returned set must be merged into the `referenced_ids` passed to
    /// `evict_unused_image_textures` so that icon textures are not evicted
    /// on frames where no matching StaticImage publication is active.
    ///
    /// Called once per frame before `render_zone_content` so that the
    /// immutable render path can look up `image_texture_cache` by ResourceId.
    ///
    /// SVG rasterization only occurs on cache miss (init-time / first-seen
    /// path).  Subsequent calls are O(1) per path due to the cache hit-check
    /// guard in `ensure_icon_texture`.
    fn ensure_scene_icon_textures(&mut self, scene: &SceneGraph) -> HashSet<ResourceId> {
        let mut icon_ids: HashSet<ResourceId> = HashSet::new();
        for zone_def in scene.zone_registry.zones.values() {
            let icon_map = &zone_def.rendering_policy.key_icon_map;
            if icon_map.is_empty() {
                continue;
            }
            for svg_path in icon_map.values() {
                let id = ResourceId::of(svg_path.as_bytes());
                icon_ids.insert(id);
                // ensure_icon_texture is a no-op on cache hit.
                self.ensure_icon_texture(svg_path);
            }
        }
        icon_ids
    }

    /// Shared encode pipeline used by `render_frame` and `render_frame_headless`.
    ///
    /// Encodes the geometry pass (clear + vertex draw) and the text pass into a
    /// single `CommandEncoder`.  The encoder is returned to the caller **before**
    /// `queue.submit` so that headless callers can append a `copy_to_buffer`
    /// command (which must precede submit).
    ///
    /// # Parameters
    ///
    /// - `vertices` — pre-built vertex list (caller is responsible for overlay
    ///   quads, zone content, etc.)
    /// - `frame_view` — render target view for this frame
    /// - `scene` — used only for the text pass (`collect_text_items`)
    /// - `surf_w` / `surf_h` — surface dimensions used for the text viewport
    /// - `use_overlay_pipeline` — when `true`, selects `clear_pipeline` (no
    ///   blending) instead of the standard `pipeline`
    ///
    /// # Returns
    ///
    /// `(encoder, encode_us)` — the ready-to-submit encoder and the wall-clock
    /// microseconds spent encoding (for `FrameTelemetry::stage6_render_encode_us`).
    /// `bg_vertex_count`: number of vertices at the start of `vertices` that
    /// make up the initial Background slice.  This includes any overlay-mode
    /// clear quad prepended before Background zone vertices, so in overlay
    /// mode callers must include those 6 clear-quad vertices in the count
    /// even when Background zones emit no flat-rect vertices.  The
    /// Background SDF rounded-rect pass is inserted between this initial
    /// slice and the remaining tile/content/chrome vertices so that
    /// Background backdrops are correctly drawn BELOW agent tiles.  Pass `0`
    /// only when the initial Background slice is empty (i.e. no Background
    /// flat-rect vertices and no overlay-mode clear quad); the Background
    /// SDF pass still runs before the tile/content/chrome pass.
    // All arguments serve distinct roles in the GPU encode pass: vertex data,
    // frame target, scene snapshot, surface dimensions, pipeline selector, and
    // the Background vertex split index.  No natural grouping exists that would
    // reduce the count without creating an ad-hoc context struct.
    #[allow(clippy::too_many_arguments)]
    fn encode_frame(
        &mut self,
        vertices: &[RectVertex],
        frame_view: &wgpu::TextureView,
        scene: &SceneGraph,
        surf_w: u32,
        surf_h: u32,
        use_overlay_pipeline: bool,
        bg_vertex_count: usize,
    ) -> (wgpu::CommandEncoder, u64) {
        let encode_start = std::time::Instant::now();

        // Upload the full vertex buffer once — individual passes render
        // different vertex sub-ranges from it using draw ranges.
        let vertex_buffer = if vertices.is_empty() {
            None
        } else {
            let buffer = self
                .device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("vertex_buffer"),
                    contents: bytemuck::cast_slice(vertices),
                    usage: wgpu::BufferUsages::VERTEX,
                });
            Some(buffer)
        };

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("frame_encoder"),
            });

        let sw = surf_w as f32;
        let sh = surf_h as f32;

        // Both flat-rect geometry sub-passes use the same pipeline selector:
        // in overlay mode, the clear_pipeline (no blending) is used for ALL
        // flat-rect rendering. The first 6 vertices are a full-screen
        // transparent quad that zeros out every pixel's alpha. Subsequent
        // content overwrites specific regions with their own alpha.

        // ── Geometry pass 1: Background flat-rect zones ───────────────────────
        // Clears the surface and draws Background-layer zone backdrops (those
        // without backdrop_radius; zones with backdrop_radius emit no vertices
        // here).  Tile/content/chrome vertices are deferred to pass 2 so that
        // the Background SDF pass can be interleaved between the two.
        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("frame_pass_bg"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: frame_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(self.clear_color()),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });

            if use_overlay_pipeline {
                render_pass.set_pipeline(&self.clear_pipeline);
            } else {
                render_pass.set_pipeline(&self.pipeline);
            }

            if let Some(ref buffer) = vertex_buffer {
                let bg_end = bg_vertex_count.min(vertices.len());
                if bg_end > 0 {
                    render_pass.set_vertex_buffer(0, buffer.slice(..));
                    render_pass.draw(0..bg_end as u32, 0..1);
                }
            }
        }

        // ── Single-pass partition of all rounded-rect commands ────────────────
        // Collect once; reuse the partitioned results in both SDF passes below.
        let rr_all = self.collect_all_rounded_rect_cmds(scene, sw, sh);

        // ── Background SDF rounded-rect pass ──────────────────────────────────
        // Runs after the Clear pass (so it composites over the cleared surface)
        // but BEFORE tile/content/chrome geometry — this is what ensures
        // Background backdrops are correctly occluded by agent tiles.
        self.encode_rounded_rect_pass(&mut encoder, frame_view, &rr_all.background, sw, sh);

        // ── Geometry pass 2: Tiles + Content + Chrome flat-rect zones ─────────
        // Uses LoadOp::Load to preserve the Background geometry drawn above.
        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("frame_pass_tiles_content_chrome"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: frame_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });

            if use_overlay_pipeline {
                render_pass.set_pipeline(&self.clear_pipeline);
            } else {
                render_pass.set_pipeline(&self.pipeline);
            }

            if let Some(ref buffer) = vertex_buffer {
                let bg_end = bg_vertex_count.min(vertices.len());
                let rest_count = vertices.len().saturating_sub(bg_end);
                if rest_count > 0 {
                    render_pass.set_vertex_buffer(0, buffer.slice(..));
                    render_pass.draw(bg_end as u32..vertices.len() as u32, 0..1);
                }
            }
        }

        // ── Content + Chrome SDF rounded-rect pass ────────────────────────────
        // Runs after tiles so Content/Chrome backdrops composite above tiles.
        {
            let mut rr_post: Vec<crate::pipeline::RoundedRectDrawCmd> = Vec::new();
            rr_post.extend(rr_all.content);
            rr_post.extend(rr_all.chrome);
            rr_post.extend(self.collect_tile_rounded_rect_cmds(scene));
            self.encode_rounded_rect_pass(&mut encoder, frame_view, &rr_post, sw, sh);
        }

        // ── Text pass (Stage 6) ───────────────────────────────────────────────
        // Collect text items before borrowing the rasterizer mutably, to avoid
        // simultaneous mutable + immutable borrow of `self`.
        let has_text_rasterizer = self.text_rasterizer.is_some();
        let text_items: Vec<TextItem> = if has_text_rasterizer {
            self.collect_text_items(scene, sw, sh)
        } else {
            vec![]
        };
        // Phase A: prepare glyphon buffers (requires mutable tr borrow).
        // Drop the borrow immediately after so Phase B can call self.gpu_color_raw.
        //
        // When text_items is empty we return None — not Some(Ok([])) — so that
        // Phase C skips render_text_pass entirely.  Calling render_text_pass
        // without a preceding prepare would replay glyphon's previously prepared
        // TextAreas from the last non-empty frame, causing stale text to linger.
        let prepare_result: Option<Result<Vec<crate::text::InlineBackdropQuad>, String>> =
            if let Some(ref mut tr) = self.text_rasterizer {
                tr.update_viewport(&self.queue, surf_w, surf_h);
                if text_items.is_empty() {
                    None
                } else {
                    Some(tr.prepare_text_items(&self.device, &self.queue, &text_items))
                }
            } else {
                None
            };
        // Phase B: convert inline backdrop quads to GPU vertices.
        // `self` is no longer mutably borrowed here, so gpu_color_raw is available.
        let inline_verts: Vec<crate::pipeline::RectVertex> =
            if let Some(Ok(ref inline_quads)) = prepare_result {
                inline_quads
                    .iter()
                    .flat_map(|q| {
                        let color = self.gpu_color_raw([
                            q.color[0] as f32 / 255.0,
                            q.color[1] as f32 / 255.0,
                            q.color[2] as f32 / 255.0,
                            q.color[3] as f32 / 255.0,
                        ]);
                        rect_vertices(q.x, q.y, q.w, q.h, sw, sh, color)
                    })
                    .collect()
            } else {
                vec![]
            };
        // Phase C: render passes (re-takes tr mutable borrow).
        if let Some(ref mut tr) = self.text_rasterizer {
            if let Some(ref result) = prepare_result {
                match result {
                    Err(e) => {
                        tracing::warn!(error = %e, "text prepare failed — frame continues without text");
                    }
                    Ok(_) => {
                        // ── Inline backdrop pass (Phase 2, hud-9ieev) ────────────
                        // Render pixel-exact backdrop quads for inline code spans
                        // BEFORE the glyphon glyph pass so backdrops appear behind
                        // the glyphs.  Uses LoadOp::Load to preserve geometry pixels.
                        if !inline_verts.is_empty() {
                            let inline_buf =
                                self.device
                                    .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                                        label: Some("inline_backdrop_verts"),
                                        contents: bytemuck::cast_slice(&inline_verts),
                                        usage: wgpu::BufferUsages::VERTEX,
                                    });
                            let mut inline_pass =
                                encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                                    label: Some("inline_backdrop_pass"),
                                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                                        view: frame_view,
                                        resolve_target: None,
                                        ops: wgpu::Operations {
                                            load: wgpu::LoadOp::Load,
                                            store: wgpu::StoreOp::Store,
                                        },
                                    })],
                                    depth_stencil_attachment: None,
                                    timestamp_writes: None,
                                    occlusion_query_set: None,
                                });
                            if use_overlay_pipeline {
                                inline_pass.set_pipeline(&self.clear_pipeline);
                            } else {
                                inline_pass.set_pipeline(&self.pipeline);
                            }
                            inline_pass.set_vertex_buffer(0, inline_buf.slice(..));
                            inline_pass.draw(0..inline_verts.len() as u32, 0..1);
                        }

                        // ── Glyphon text pass ─────────────────────────────────────
                        let mut text_pass =
                            encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                                label: Some("text_pass"),
                                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                                    view: frame_view,
                                    resolve_target: None,
                                    ops: wgpu::Operations {
                                        // LoadOp::Load: preserve geometry pixels under the text.
                                        load: wgpu::LoadOp::Load,
                                        store: wgpu::StoreOp::Store,
                                    },
                                })],
                                depth_stencil_attachment: None,
                                timestamp_writes: None,
                                occlusion_query_set: None,
                            });
                        if let Err(e) = tr.render_text_pass(&mut text_pass) {
                            tracing::warn!(error = %e, "text render failed — frame continues without text");
                        }
                    }
                }
            }
            // Trim every frame regardless of item count — glyphs from prior frames
            // must be evicted even when the current frame has no text.
            tr.trim_atlas();
        }

        let encode_us = encode_start.elapsed().as_micros() as u64;
        (encoder, encode_us)
    }
}

/// Collect [`TextItem`]s for all `TextOverflow::Ellipsis` nodes reachable from
/// `node_id`, without scroll offset (prime-time geometry).
///
/// This is a free function (not a method) to avoid a split-borrow conflict in
/// [`Compositor::prime_truncation_cache`], where `self.text_rasterizer` is
/// borrowed mutably while the markdown snapshot (loaded from the primer) and
/// `self.node_key_cache` are read immutably.
///
/// The geometry produced here is identical to what `collect_text_items_from_node`
/// produces at scroll_x=0, scroll_y=0 (valid because truncation is geometry-
/// Returns `true` if the mid-drag cadence gate should defer a truncation cache
/// re-prime call.
///
/// Called by [`Compositor::prime_truncation_cache`] to rate-limit re-primes
/// during rapid geometry changes (e.g. hotkey resize).  The gate is bypassed
/// for forced primes (`truncation_cache_scene_version == u64::MAX`) — callers
/// are responsible for that check.
///
/// # Parameters
/// - `last_at`: timestamp of the last successful truncation cache prime, or
///   `None` if no prime has ever run.
/// - `interval_ms`: minimum interval between primes in milliseconds
///   (derived from content length via [`adaptive_reprime_interval_ms`]).
///
/// Returns `true` (defer) only when `last_at` is `Some` and the elapsed time
/// is less than `interval_ms`.  Returns `false` (allow) when `last_at` is
/// `None` (first call ever) or the interval has elapsed.
pub(crate) fn should_defer_reprime(last_at: Option<std::time::Instant>, interval_ms: u64) -> bool {
    match last_at {
        None => false, // first call: never defer
        Some(last) => last.elapsed() < std::time::Duration::from_millis(interval_ms),
    }
}

/// Return whether a tile is currently in follow-tail/at-tail mode for the
/// purpose of ellipsis truncation geometry.
///
/// This is the single authoritative computation shared by
/// [`Compositor::prime_truncation_cache`] (via [`collect_ellipsis_text_items_from_node`])
/// and [`Compositor::collect_text_items`] (via [`Compositor::collect_text_items_from_node`]).
/// Both callers must resolve `at_tail` identically so the per-frame
/// [`crate::text::TruncationKey`] matches the primed cache entry (hud-lu50e).
///
/// Returns `true` (TailAnchored — spec §3.2: newest lines visible) when the
/// tile has been registered at-tail by the follow-tail driver.  Returns `false`
/// (HeadAnchored — spec §3.3: viewport stability) when the tile has been
/// scrolled back or has never been registered (e.g., non-scrollable tiles).
///
/// Extracting this as a named helper eliminates the hand-mirrored
/// `must-mirror` comment that previously guarded the two call sites (hud-plz8q).
pub(crate) fn tile_at_tail_for_ellipsis(tile_id: SceneId, scene: &SceneGraph) -> bool {
    scene.tile_follow_tail_at_tail(tile_id)
}

/// Resolve the pixel dimensions for a widget instance.
///
/// Returns (width, height) in pixels. Returns (0, 0) if the geometry cannot be
/// resolved (e.g., zero-sized surface or unrecognized policy).
pub(super) fn resolve_widget_pixel_size(
    instance: &WidgetInstance,
    def: &WidgetDefinition,
    surf_w: u32,
    surf_h: u32,
) -> (u32, u32) {
    let geo = instance
        .geometry_override
        .as_ref()
        .unwrap_or(&def.default_geometry_policy);
    let sw = surf_w as f32;
    let sh = surf_h as f32;
    let (w, h) = match geo {
        GeometryPolicy::Relative {
            width_pct,
            height_pct,
            ..
        } => (sw * width_pct, sh * height_pct),
        GeometryPolicy::EdgeAnchored {
            width_pct,
            height_pct,
            ..
        } => (sw * width_pct, sh * height_pct),
    };
    let w = (w.max(1.0) as u32).min(surf_w.max(1));
    let h = (h.max(1.0) as u32).min(surf_h.max(1));
    (w, h)
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
mod tests;
