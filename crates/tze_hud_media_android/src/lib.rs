//! tze_hud_media_android — Android GStreamer media shim.
//!
//! Phase 3 target: D19 — 1x Pixel (aarch64-linux-android).
//! Decode path: GStreamer `androidmedia` plugin (MediaCodec backend, HYBRID-NATIVE-MEDIACODEC
//! verdict from hud-4znng) with `ndk::media` direct bindings as fallback.
//!
//! # Current state
//!
//! Scaffold only. The build system bridge (`build.rs`) is implemented and validates
//! the GStreamer SDK linker path via CI (android-bootstrap.yml, gates 1–5).
//!
//! Phase 3 implementation will fill this crate with:
//! - JNI_OnLoad → gst_android_init → [event loop] → gst_init ordering (see spike
//!   doc §5.3 — mandatory before androidmedia plugin registers its JNI callbacks).
//! - AppSrc/AppSink bridge mirroring the desktop GStreamer pipeline model.
//! - MediaCodec surface handoff via ANativeWindow.
//!
//! # JVM bootstrap ordering constraint (DO NOT SKIP)
//!
//! `libgstandroidmedia.so` requires JVM initialization before GStreamer registers
//! the `androidmedia` plugin. The call ordering MUST be:
//!
//! ```text
//! JNI_OnLoad  →  gst_android_init(env, context)  →  [event loop starts]  →  gst_init()
//! ```
//!
//! Violating this order causes `amcvideodec` to silently fail to register.
//! See `docs/ci/android-gstreamer-bootstrap.md` §5.3.
//!
//! # References
//!
//! - `docs/ci/android-gstreamer-bootstrap.md` — CI bootstrap plan (hud-685ha, PR #539)
//! - `docs/audits/android-gstreamer-sdk-build-spike.md` — full spike (hud-4znng, PR #536)
//! - `openspec/changes/v2-embodied-media-presence/` — phase 3 scope: D19, E24/E25 codecs

#![cfg(target_os = "android")]
// Phase 3: implementation placeholder.
// Remove this allow when symbols are added.
#![allow(dead_code)]
