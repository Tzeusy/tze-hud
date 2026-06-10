//! Portal cadence coalescing — re-exported from `tze_hud_projection`.
//!
//! The canonical implementation lives in `tze_hud_projection::portal_cadence`
//! so that `ProjectionAuthority` can hold a `PortalCadenceCoalescer` field
//! without creating a circular crate dependency. This module re-exports all
//! public items for consumers that import from `tze_hud_runtime`.

pub use tze_hud_projection::portal_cadence::*;
