//! Clock-domain timing types for tze_hud.
//!
//! See [`domains`] for the primary types: [`WallUs`], [`MonoUs`],
//! [`DurationUs`], and [`ClockOffset`].

pub mod domains;

pub use domains::{ClockOffset, DurationUs, MonoUs, WallUs};
