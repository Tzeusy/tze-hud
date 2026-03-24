//! # tze_hud_compositor
//!
//! wgpu compositor for tze_hud. Renders the scene graph to a native window
//! or headless offscreen texture.
//! Satisfies DR-V2: Headless rendering.
//! Satisfies DR-V6: No physical GPU required (llvmpipe/WARP).

pub mod renderer;
pub mod surface;
pub mod pipeline;

pub use renderer::Compositor;
pub use surface::{CompositorFrame, CompositorSurface, HeadlessSurface, WindowSurface};
pub use pipeline::ChromeDrawCmd;
