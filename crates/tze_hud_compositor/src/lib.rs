//! # tze_hud_compositor
//!
//! wgpu compositor for tze_hud. Renders the scene graph to a native window
//! or headless offscreen texture.
//! Satisfies DR-V2: Headless rendering.
//! Satisfies DR-V6: No physical GPU required (llvmpipe/WARP).
//!
//! ## GPU Adapter Selection (spec §Platform GPU Backends, line 189)
//!
//! Platform-mandated GPU backends:
//! - Linux: Vulkan
//! - Windows: D3D12 and Vulkan
//! - macOS: Metal
//!
//! WHEN no suitable GPU adapter is found THEN the runtime MUST fail with a
//! fatal error and structured error message (spec line 195).
//!
//! The `select_gpu_adapter` function implements this policy. The existing
//! `Compositor::new_headless` path uses `Backends::all()` so that CI on
//! software renderers (llvmpipe/WARP) still works; windowed startup should
//! call `select_gpu_adapter` to enforce the platform constraint.

pub mod adapter;
pub mod pipeline;
pub mod renderer;
pub mod surface;
pub mod text;

pub use adapter::{AdapterSelectionError, PlatformBackends, select_gpu_adapter};
pub use pipeline::ChromeDrawCmd;
pub use renderer::{Compositor, CompositorError};
pub use surface::{CompositorFrame, CompositorSurface, HeadlessSurface, WindowSurface};
pub use text::{TextItem, TextRasterizer};
