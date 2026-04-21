//! tze_hud_media_android — Android GStreamer media shim.
//!
//! Phase 3 target: D19 — 1x Pixel (aarch64-linux-android).
//! Decode path: GStreamer `androidmedia` plugin (MediaCodec backend, HYBRID-NATIVE-MEDIACODEC
//! verdict from hud-4znng) with `ndk::media` direct bindings as fallback.
//!
//! # Scope
//!
//! This crate implements:
//! - JNI bootstrap ordering guards: `JNI_OnLoad -> gst_android_init -> gst_init`.
//! - Runtime loading/verification of the `androidmedia` plugin (`amcvideodec`).
//! - An AppSrc/AppSink bridge matching the desktop media-pipeline model.
//! - Optional ANativeWindow handoff to a video-overlay sink when present.
//!
//! On non-Android targets, the same API surface is available but returns
//! [`AndroidMediaError::UnsupportedPlatform`] for runtime operations.

use std::fmt;

/// Stable string error codes for structured diagnostics.
pub mod codes {
    pub const ORDERING_VIOLATION: &str = "ANDROID_MEDIA_ORDERING_VIOLATION";
    pub const FFI_FAILURE: &str = "ANDROID_MEDIA_FFI_FAILURE";
    pub const PLUGIN_MISSING: &str = "ANDROID_MEDIA_PLUGIN_MISSING";
    pub const PIPELINE_BUILD_FAILED: &str = "ANDROID_MEDIA_PIPELINE_BUILD_FAILED";
    pub const PIPELINE_STATE_FAILED: &str = "ANDROID_MEDIA_PIPELINE_STATE_FAILED";
    pub const PUSH_FAILED: &str = "ANDROID_MEDIA_PUSH_FAILED";
    pub const UNSUPPORTED_PLATFORM: &str = "ANDROID_MEDIA_UNSUPPORTED_PLATFORM";
}

/// Error type for Android media bootstrap and pipeline operations.
#[derive(Debug)]
pub enum AndroidMediaError {
    OrderingViolation { detail: String },
    FfiFailure { detail: String },
    PluginMissing { detail: String },
    PipelineBuildFailed { detail: String },
    PipelineStateFailed { detail: String },
    PushFailed { detail: String },
    UnsupportedPlatform,
}

impl AndroidMediaError {
    /// Stable code associated with this error.
    pub fn code(&self) -> &'static str {
        match self {
            Self::OrderingViolation { .. } => codes::ORDERING_VIOLATION,
            Self::FfiFailure { .. } => codes::FFI_FAILURE,
            Self::PluginMissing { .. } => codes::PLUGIN_MISSING,
            Self::PipelineBuildFailed { .. } => codes::PIPELINE_BUILD_FAILED,
            Self::PipelineStateFailed { .. } => codes::PIPELINE_STATE_FAILED,
            Self::PushFailed { .. } => codes::PUSH_FAILED,
            Self::UnsupportedPlatform => codes::UNSUPPORTED_PLATFORM,
        }
    }
}

impl fmt::Display for AndroidMediaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OrderingViolation { detail } => {
                write!(f, "[{}] {}", self.code(), detail)
            }
            Self::FfiFailure { detail } => write!(f, "[{}] {}", self.code(), detail),
            Self::PluginMissing { detail } => write!(f, "[{}] {}", self.code(), detail),
            Self::PipelineBuildFailed { detail } => write!(f, "[{}] {}", self.code(), detail),
            Self::PipelineStateFailed { detail } => write!(f, "[{}] {}", self.code(), detail),
            Self::PushFailed { detail } => write!(f, "[{}] {}", self.code(), detail),
            Self::UnsupportedPlatform => {
                write!(f, "[{}] android target required", self.code())
            }
        }
    }
}

impl std::error::Error for AndroidMediaError {}

#[cfg_attr(not(any(target_os = "android", test)), allow(dead_code))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BootstrapState {
    Fresh,
    JniOnLoadSeen,
    GstAndroidInitialized,
    GstInitialized,
}

#[cfg_attr(not(any(target_os = "android", test)), allow(dead_code))]
#[derive(Debug, Clone)]
struct BootstrapTracker {
    state: BootstrapState,
}

impl Default for BootstrapTracker {
    fn default() -> Self {
        Self {
            state: BootstrapState::Fresh,
        }
    }
}

#[cfg_attr(not(any(target_os = "android", test)), allow(dead_code))]
impl BootstrapTracker {
    fn mark_jni_on_load_seen(&mut self) -> Result<(), AndroidMediaError> {
        match self.state {
            BootstrapState::Fresh => {
                self.state = BootstrapState::JniOnLoadSeen;
                Ok(())
            }
            _ => Err(AndroidMediaError::OrderingViolation {
                detail: "JNI_OnLoad may only be observed once at startup".to_string(),
            }),
        }
    }

    fn mark_gst_android_initialized(&mut self) -> Result<(), AndroidMediaError> {
        match self.state {
            BootstrapState::JniOnLoadSeen => {
                self.state = BootstrapState::GstAndroidInitialized;
                Ok(())
            }
            BootstrapState::Fresh => Err(AndroidMediaError::OrderingViolation {
                detail: "gst_android_init called before JNI_OnLoad".to_string(),
            }),
            BootstrapState::GstAndroidInitialized | BootstrapState::GstInitialized => {
                Err(AndroidMediaError::OrderingViolation {
                    detail: "gst_android_init may only run once".to_string(),
                })
            }
        }
    }

    fn mark_gst_initialized(&mut self) -> Result<(), AndroidMediaError> {
        match self.state {
            BootstrapState::GstAndroidInitialized => {
                self.state = BootstrapState::GstInitialized;
                Ok(())
            }
            BootstrapState::Fresh | BootstrapState::JniOnLoadSeen => {
                Err(AndroidMediaError::OrderingViolation {
                    detail: "gst_init called before gst_android_init".to_string(),
                })
            }
            BootstrapState::GstInitialized => Ok(()),
        }
    }

    fn ensure_pipeline_allowed(&self) -> Result<(), AndroidMediaError> {
        if self.state == BootstrapState::GstInitialized {
            Ok(())
        } else {
            Err(AndroidMediaError::OrderingViolation {
                detail: "pipeline creation requires gst_init after gst_android_init".to_string(),
            })
        }
    }
}

#[cfg(target_os = "android")]
mod platform {
    use super::{AndroidMediaError, BootstrapTracker};
    use std::ffi::{CStr, CString, c_char, c_int, c_uint, c_void};
    use std::ptr::{self, NonNull};
    use std::sync::{Mutex, OnceLock};

    const JNI_VERSION_1_6: c_int = 0x0001_0006;
    const GST_CLOCK_TIME_NONE: u64 = u64::MAX;

    #[repr(C)]
    struct GstElement(c_void);
    #[repr(C)]
    struct GstObject(c_void);
    #[repr(C)]
    struct GstAppSrc(c_void);
    #[repr(C)]
    struct GstAppSink(c_void);
    #[repr(C)]
    struct GstBuffer(c_void);
    #[repr(C)]
    struct GstSample(c_void);
    #[repr(C)]
    struct GstPlugin(c_void);
    #[repr(C)]
    struct GError {
        domain: c_uint,
        code: c_int,
        message: *mut c_char,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    enum GstState {
        Null = 1,
        Playing = 4,
    }

    #[repr(C)]
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum GstStateChangeReturn {
        Failure = 0,
    }

    unsafe extern "C" {
        fn gst_android_init(env: *mut c_void, context: *mut c_void);
        fn gst_init(argc: *mut c_int, argv: *mut *mut *mut c_char);
        fn gst_plugin_load_file(filename: *const c_char, error: *mut *mut GError)
        -> *mut GstPlugin;
        fn gst_element_factory_find(name: *const c_char) -> *mut c_void;
        fn gst_parse_launch(
            pipeline_description: *const c_char,
            error: *mut *mut GError,
        ) -> *mut GstElement;
        fn gst_bin_get_by_name(bin: *mut GstElement, name: *const c_char) -> *mut GstElement;
        fn gst_object_unref(object: *mut GstObject);
        fn gst_element_set_state(element: *mut GstElement, state: GstState)
        -> GstStateChangeReturn;
        fn gst_buffer_new_allocate(
            allocator: *mut c_void,
            size: usize,
            params: *mut c_void,
        ) -> *mut GstBuffer;
        fn gst_buffer_fill(
            buffer: *mut GstBuffer,
            offset: usize,
            src: *const c_void,
            size: usize,
        ) -> usize;
        fn gst_mini_object_unref(mini_object: *mut c_void);
        fn gst_app_src_push_buffer(appsrc: *mut GstAppSrc, buffer: *mut GstBuffer) -> c_int;
        fn gst_app_sink_try_pull_sample(appsink: *mut GstAppSink, timeout: u64) -> *mut GstSample;
        fn gst_sample_unref(sample: *mut GstSample);
        fn g_error_free(error: *mut GError);
        fn gst_video_overlay_set_window_handle(overlay: *mut c_void, handle: usize);
    }

    fn bootstrap() -> &'static Mutex<BootstrapTracker> {
        static BOOTSTRAP: OnceLock<Mutex<BootstrapTracker>> = OnceLock::new();
        BOOTSTRAP.get_or_init(|| Mutex::new(BootstrapTracker::default()))
    }

    fn c_error_to_string(error: *mut GError) -> String {
        if error.is_null() {
            return "unknown GStreamer error".to_string();
        }
        // SAFETY: `error` originates from GLib/GStreamer APIs. We free it exactly once.
        let message = unsafe {
            let c_msg = if (*error).message.is_null() {
                "unknown GStreamer error".to_string()
            } else {
                CStr::from_ptr((*error).message)
                    .to_string_lossy()
                    .into_owned()
            };
            g_error_free(error);
            c_msg
        };
        message
    }

    /// Android native JNI entry point.
    #[unsafe(no_mangle)]
    pub unsafe extern "system" fn JNI_OnLoad(_vm: *mut c_void, _reserved: *mut c_void) -> c_int {
        let result = bootstrap()
            .lock()
            .map_err(|_| AndroidMediaError::FfiFailure {
                detail: "bootstrap mutex poisoned during JNI_OnLoad".to_string(),
            })
            .and_then(|mut tracker| tracker.mark_jni_on_load_seen());

        if result.is_ok() { JNI_VERSION_1_6 } else { 0 }
    }

    /// Runs `gst_android_init(env, context)` and records the bootstrap stage.
    pub unsafe fn run_gst_android_init(
        env: *mut c_void,
        context: *mut c_void,
    ) -> Result<(), AndroidMediaError> {
        {
            let mut tracker = bootstrap()
                .lock()
                .map_err(|_| AndroidMediaError::FfiFailure {
                    detail: "bootstrap mutex poisoned before gst_android_init".to_string(),
                })?;
            tracker.mark_gst_android_initialized()?;
        }

        // SAFETY: caller guarantees Android JNI environment/context pointers are valid.
        unsafe { gst_android_init(env, context) };
        Ok(())
    }

    /// Runs global `gst_init` only after `gst_android_init` was observed.
    pub fn run_gst_init() -> Result<(), AndroidMediaError> {
        {
            let mut tracker = bootstrap()
                .lock()
                .map_err(|_| AndroidMediaError::FfiFailure {
                    detail: "bootstrap mutex poisoned before gst_init".to_string(),
                })?;
            tracker.mark_gst_initialized()?;
        }

        // SAFETY: GStreamer permits null argc/argv for process-global init.
        unsafe { gst_init(ptr::null_mut(), ptr::null_mut()) };
        Ok(())
    }

    /// Loads `libgstandroidmedia.so` and verifies `amcvideodec` registration.
    pub fn load_androidmedia_plugin(plugin_path: &str) -> Result<(), AndroidMediaError> {
        let tracker = bootstrap()
            .lock()
            .map_err(|_| AndroidMediaError::FfiFailure {
                detail: "bootstrap mutex poisoned while loading androidmedia plugin".to_string(),
            })?;
        tracker.ensure_pipeline_allowed()?;
        drop(tracker);

        let plugin_path =
            CString::new(plugin_path).map_err(|_| AndroidMediaError::PluginMissing {
                detail: format!("invalid plugin path (contains NUL): {plugin_path}"),
            })?;

        let mut err: *mut GError = ptr::null_mut();
        // SAFETY: C string is valid and null-terminated.
        let plugin = unsafe { gst_plugin_load_file(plugin_path.as_ptr(), &mut err) };
        if plugin.is_null() {
            return Err(AndroidMediaError::PluginMissing {
                detail: format!(
                    "failed to load androidmedia plugin '{}': {}",
                    plugin_path.to_string_lossy(),
                    c_error_to_string(err)
                ),
            });
        }

        let factory_name = CString::new("amcvideodec").expect("static string has no NUL");
        // SAFETY: `factory_name` is a valid static lookup string.
        let factory = unsafe { gst_element_factory_find(factory_name.as_ptr()) };
        if factory.is_null() {
            return Err(AndroidMediaError::PluginMissing {
                detail: "androidmedia loaded but amcvideodec factory missing".to_string(),
            });
        }

        // SAFETY: plugin/factory refs are valid GObjects and can be unreffed.
        unsafe {
            gst_object_unref(plugin.cast::<GstObject>());
            gst_object_unref(factory.cast::<GstObject>());
        }

        Ok(())
    }

    /// Owned pulled sample. Drop releases the underlying GStreamer sample ref.
    pub struct PulledSample {
        raw: NonNull<GstSample>,
    }

    impl Drop for PulledSample {
        fn drop(&mut self) {
            // SAFETY: `raw` came from `gst_app_sink_try_pull_sample` and owns one ref.
            unsafe { gst_sample_unref(self.raw.as_ptr()) };
        }
    }

    /// AppSrc/AppSink bridge for Android MediaCodec decode via `androidmedia`.
    pub struct AndroidMediaBridge {
        pipeline: NonNull<GstElement>,
        appsrc: NonNull<GstAppSrc>,
        appsink: NonNull<GstAppSink>,
        video_sink: Option<NonNull<GstElement>>,
    }

    impl Drop for AndroidMediaBridge {
        fn drop(&mut self) {
            // SAFETY: all handles are valid GStreamer refs owned by this struct.
            unsafe {
                let _ = gst_element_set_state(self.pipeline.as_ptr(), GstState::Null);
                gst_object_unref(self.appsrc.as_ptr().cast::<GstObject>());
                gst_object_unref(self.appsink.as_ptr().cast::<GstObject>());
                if let Some(video_sink) = self.video_sink {
                    gst_object_unref(video_sink.as_ptr().cast::<GstObject>());
                }
                gst_object_unref(self.pipeline.as_ptr().cast::<GstObject>());
            }
        }
    }

    impl AndroidMediaBridge {
        /// Builds the phase-3 decode bridge:
        /// `appsrc -> h264parse -> amcvideodec -> videoconvert -> appsink`.
        ///
        /// If `with_video_overlay_sink` is true, a tee branch with `glimagesink`
        /// (`video_sink`) is added for optional ANativeWindow handoff.
        pub fn new_h264(with_video_overlay_sink: bool) -> Result<Self, AndroidMediaError> {
            struct PendingHandles {
                pipeline: NonNull<GstElement>,
                appsrc: Option<NonNull<GstElement>>,
                appsink: Option<NonNull<GstElement>>,
                video_sink: Option<NonNull<GstElement>>,
            }

            impl Drop for PendingHandles {
                fn drop(&mut self) {
                    // SAFETY: any handles set here were acquired by this constructor and
                    // must be released if construction fails before ownership transfers.
                    unsafe {
                        if let Some(appsrc) = self.appsrc {
                            gst_object_unref(appsrc.as_ptr().cast::<GstObject>());
                        }
                        if let Some(appsink) = self.appsink {
                            gst_object_unref(appsink.as_ptr().cast::<GstObject>());
                        }
                        if let Some(video_sink) = self.video_sink {
                            gst_object_unref(video_sink.as_ptr().cast::<GstObject>());
                        }
                        gst_object_unref(self.pipeline.as_ptr().cast::<GstObject>());
                    }
                }
            }

            let tracker = bootstrap()
                .lock()
                .map_err(|_| AndroidMediaError::FfiFailure {
                    detail: "bootstrap mutex poisoned before pipeline creation".to_string(),
                })?;
            tracker.ensure_pipeline_allowed()?;
            drop(tracker);

            let desc = if with_video_overlay_sink {
                concat!(
                    "appsrc name=ingress is-live=true format=time do-timestamp=true ",
                    "caps=video/x-h264,stream-format=byte-stream,alignment=au ! ",
                    "queue ! h264parse ! amcvideodec ! tee name=t ",
                    "t. ! queue ! videoconvert ! video/x-raw,format=RGBA ! ",
                    "appsink name=egress emit-signals=false sync=false max-buffers=4 drop=true ",
                    "t. ! queue ! glimagesink name=video_sink sync=false"
                )
            } else {
                concat!(
                    "appsrc name=ingress is-live=true format=time do-timestamp=true ",
                    "caps=video/x-h264,stream-format=byte-stream,alignment=au ! ",
                    "queue ! h264parse ! amcvideodec ! videoconvert ! video/x-raw,format=RGBA ! ",
                    "appsink name=egress emit-signals=false sync=false max-buffers=4 drop=true"
                )
            };

            let desc = CString::new(desc).expect("pipeline description is static and NUL-free");
            let mut err: *mut GError = ptr::null_mut();

            // SAFETY: description is a valid null-terminated pipeline string.
            let pipeline = unsafe { gst_parse_launch(desc.as_ptr(), &mut err) };
            let pipeline =
                NonNull::new(pipeline).ok_or_else(|| AndroidMediaError::PipelineBuildFailed {
                    detail: c_error_to_string(err),
                })?;

            let mut pending = PendingHandles {
                pipeline,
                appsrc: None,
                appsink: None,
                video_sink: None,
            };

            pending.appsrc = Some(lookup_element(pipeline, "ingress")?);
            pending.appsink = Some(lookup_element(pipeline, "egress")?);
            pending.video_sink = if with_video_overlay_sink {
                Some(lookup_element(pipeline, "video_sink")?)
            } else {
                None
            };

            let bridge = Self {
                pipeline,
                appsrc: pending.appsrc.expect("set above").cast(),
                appsink: pending.appsink.expect("set above").cast(),
                video_sink: pending.video_sink,
            };
            std::mem::forget(pending);
            Ok(bridge)
        }

        /// Sets pipeline state to `PLAYING`.
        pub fn start(&self) -> Result<(), AndroidMediaError> {
            // SAFETY: pipeline pointer remains valid for the struct lifetime.
            let result =
                unsafe { gst_element_set_state(self.pipeline.as_ptr(), GstState::Playing) };
            if result == GstStateChangeReturn::Failure {
                return Err(AndroidMediaError::PipelineStateFailed {
                    detail: "failed to transition pipeline to PLAYING".to_string(),
                });
            }
            Ok(())
        }

        /// Pushes one encoded access unit into `appsrc`.
        pub fn push_access_unit(&self, data: &[u8]) -> Result<(), AndroidMediaError> {
            // SAFETY: returns null on allocation failure.
            let buffer =
                unsafe { gst_buffer_new_allocate(ptr::null_mut(), data.len(), ptr::null_mut()) };
            let Some(buffer) = NonNull::new(buffer) else {
                return Err(AndroidMediaError::PushFailed {
                    detail: "gst_buffer_new_allocate returned NULL".to_string(),
                });
            };

            // SAFETY: destination buffer is valid and large enough for `data`.
            let filled = unsafe {
                gst_buffer_fill(
                    buffer.as_ptr(),
                    0,
                    data.as_ptr().cast::<c_void>(),
                    data.len(),
                )
            };
            if filled != data.len() {
                // SAFETY: `buffer` is still exclusively owned by this function on fill failure.
                unsafe { gst_mini_object_unref(buffer.as_ptr().cast::<c_void>()) };
                return Err(AndroidMediaError::PushFailed {
                    detail: format!(
                        "gst_buffer_fill wrote {filled} bytes, expected {}",
                        data.len()
                    ),
                });
            }

            // SAFETY: appsrc is valid; ownership of buffer transfers to appsrc call.
            let flow = unsafe { gst_app_src_push_buffer(self.appsrc.as_ptr(), buffer.as_ptr()) };
            if flow != 0 {
                return Err(AndroidMediaError::PushFailed {
                    detail: format!("gst_app_src_push_buffer failed with flow={flow}"),
                });
            }

            Ok(())
        }

        /// Pulls one decoded sample from `appsink`.
        pub fn try_pull_sample(&self, timeout_ns: Option<u64>) -> Option<PulledSample> {
            let timeout = timeout_ns.unwrap_or(GST_CLOCK_TIME_NONE);
            // SAFETY: appsink pointer is valid for the struct lifetime.
            let sample = unsafe { gst_app_sink_try_pull_sample(self.appsink.as_ptr(), timeout) };
            NonNull::new(sample).map(|raw| PulledSample { raw })
        }

        /// Optional ANativeWindow handoff to the `video_sink` branch.
        pub fn attach_native_window(
            &self,
            native_window: *mut c_void,
        ) -> Result<(), AndroidMediaError> {
            let Some(video_sink) = self.video_sink else {
                return Err(AndroidMediaError::PipelineBuildFailed {
                    detail: "pipeline was created without video overlay sink".to_string(),
                });
            };

            // SAFETY: `video_sink` is a valid GStreamer video-overlay element and
            // `native_window` is an ANativeWindow pointer from Android runtime.
            unsafe {
                gst_video_overlay_set_window_handle(
                    video_sink.as_ptr().cast::<c_void>(),
                    native_window as usize,
                );
            }
            Ok(())
        }
    }

    fn lookup_element(
        pipeline: NonNull<GstElement>,
        name: &str,
    ) -> Result<NonNull<GstElement>, AndroidMediaError> {
        let c_name = CString::new(name).expect("element name is static and NUL-free");
        // SAFETY: pipeline is valid and `c_name` is a proper C string.
        let element = unsafe { gst_bin_get_by_name(pipeline.as_ptr(), c_name.as_ptr()) };
        NonNull::new(element).ok_or_else(|| AndroidMediaError::PipelineBuildFailed {
            detail: format!("pipeline missing required element '{name}'"),
        })
    }
}

#[cfg(not(target_os = "android"))]
mod platform {
    use super::AndroidMediaError;
    use std::ffi::c_void;

    /// Non-Android JNI stub; always returns 0.
    ///
    /// # Safety
    /// This function is an FFI entry point and may be called by foreign code
    /// with arbitrary pointers. On non-Android builds it ignores all inputs.
    #[unsafe(no_mangle)]
    #[allow(non_snake_case)]
    pub unsafe extern "system" fn JNI_OnLoad(_vm: *mut c_void, _reserved: *mut c_void) -> i32 {
        0
    }

    /// Non-Android stub for `gst_android_init`.
    ///
    /// # Safety
    /// Signature matches the Android FFI API. Pointers are ignored because this
    /// platform path always returns `UnsupportedPlatform`.
    pub unsafe fn run_gst_android_init(
        _env: *mut c_void,
        _context: *mut c_void,
    ) -> Result<(), AndroidMediaError> {
        Err(AndroidMediaError::UnsupportedPlatform)
    }

    pub fn run_gst_init() -> Result<(), AndroidMediaError> {
        Err(AndroidMediaError::UnsupportedPlatform)
    }

    pub fn load_androidmedia_plugin(_plugin_path: &str) -> Result<(), AndroidMediaError> {
        Err(AndroidMediaError::UnsupportedPlatform)
    }

    pub struct PulledSample;

    pub struct AndroidMediaBridge;

    impl AndroidMediaBridge {
        pub fn new_h264(_with_video_overlay_sink: bool) -> Result<Self, AndroidMediaError> {
            Err(AndroidMediaError::UnsupportedPlatform)
        }

        pub fn start(&self) -> Result<(), AndroidMediaError> {
            Err(AndroidMediaError::UnsupportedPlatform)
        }

        pub fn push_access_unit(&self, _data: &[u8]) -> Result<(), AndroidMediaError> {
            Err(AndroidMediaError::UnsupportedPlatform)
        }

        pub fn try_pull_sample(&self, _timeout_ns: Option<u64>) -> Option<PulledSample> {
            None
        }

        pub fn attach_native_window(
            &self,
            _native_window: *mut c_void,
        ) -> Result<(), AndroidMediaError> {
            Err(AndroidMediaError::UnsupportedPlatform)
        }
    }
}

pub use platform::{
    AndroidMediaBridge, JNI_OnLoad, PulledSample, load_androidmedia_plugin, run_gst_android_init,
    run_gst_init,
};

#[cfg(test)]
mod tests {
    use super::{AndroidMediaError, BootstrapTracker, codes};

    #[test]
    fn tracker_rejects_gst_android_init_before_jni_on_load() {
        let mut tracker = BootstrapTracker::default();
        let err = tracker
            .mark_gst_android_initialized()
            .expect_err("gst_android_init before JNI_OnLoad must fail");
        assert!(matches!(err, AndroidMediaError::OrderingViolation { .. }));
        assert_eq!(err.code(), codes::ORDERING_VIOLATION);
    }

    #[test]
    fn tracker_rejects_gst_init_before_gst_android_init() {
        let mut tracker = BootstrapTracker::default();
        tracker
            .mark_jni_on_load_seen()
            .expect("JNI_OnLoad transition should succeed");

        let err = tracker
            .mark_gst_initialized()
            .expect_err("gst_init before gst_android_init must fail");
        assert!(matches!(err, AndroidMediaError::OrderingViolation { .. }));
        assert_eq!(err.code(), codes::ORDERING_VIOLATION);
    }

    #[test]
    fn tracker_accepts_required_bootstrap_order() {
        let mut tracker = BootstrapTracker::default();
        tracker
            .mark_jni_on_load_seen()
            .expect("JNI_OnLoad transition should succeed");
        tracker
            .mark_gst_android_initialized()
            .expect("gst_android_init transition should succeed");
        tracker
            .mark_gst_initialized()
            .expect("gst_init transition should succeed");
        tracker
            .ensure_pipeline_allowed()
            .expect("pipeline should be allowed after gst_init");
    }

    #[test]
    fn unsupported_platform_error_has_stable_code() {
        let err = AndroidMediaError::UnsupportedPlatform;
        assert_eq!(err.code(), codes::UNSUPPORTED_PLATFORM);
        assert!(err.to_string().contains("android target required"));
    }
}
