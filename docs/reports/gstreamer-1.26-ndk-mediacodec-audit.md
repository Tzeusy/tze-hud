# GStreamer NDK MediaCodec Audit (MR #4115 / hud-fts60)

**Issued for**: `hud-fts60`
**Date**: 2026-04-19
**Auditor**: agent worker (claude-sonnet-4-6)
**Discovered from**: hud-4znng (Android GStreamer SDK spike, PR #536) — discovered follow-up #2
**Cross-reference**: hud-685ha (Android GStreamer SDK CI bootstrap, running in parallel)

---

## Verdict

**MR #4115 IS MERGED — in GStreamer 1.24, not 1.26.**

The NDK MediaCodec path ships in GStreamer 1.24 (released 4 March 2024). It is **not new in 1.26**. The JNI_OnLoad bootstrap constraint (`gst_android_init`) is **NOT eliminated** — it remains mandatory because codec enumeration and surface texture handling still require JNI. The NDK path is an additive optimization on top of the existing JNI-backed `amcvideodec`, not a replacement for it.

---

## Research Scope and Method

This is a pure research task. The upstream GStreamer GitLab instance (gitlab.freedesktop.org) blocked automated access via Anubis anti-bot protection during this session, so the MR page could not be fetched directly. Research relied on:

- GitLab search result snippets (title, author, description available from search engine cache)
- GitHub mirror of the GStreamer monorepo (`github.com/GStreamer/gstreamer`, which is the canonical upstream)
- GStreamer official release notes source (`github.com/GStreamer/www`)
- GStreamer Discourse and community forum discussions
- Direct inspection of the live source tree at `subprojects/gst-plugins-bad/sys/androidmedia/`

---

## MR #4115 Status

| Field | Value |
|---|---|
| Title | `androidmedia: add NDK implement of Android MediaCodec` |
| Author | Ratchanan Srirattanamet (`peat-psuwit`) |
| Submitted | March 5, 2023 |
| Merged into main | January 11, 2024 (commit `facb000afe9403e603f2024f2ab9dd1ae5026c04`) |
| Shipped in | **GStreamer 1.24** (released March 4, 2024) |
| GitLab URL | https://gitlab.freedesktop.org/gstreamer/gstreamer/-/merge_requests/4115 |

The GStreamer 1.24 release notes explicitly state:

> "Add NDK implementation of Android MediaCodec. This reduces the amount of Java <-> native calls, which should reduce overhead."

The `ndk/` subdirectory under `subprojects/gst-plugins-bad/sys/androidmedia/` in the main GStreamer repo confirms the merge:

```
sys/androidmedia/ndk/
  gstamc-codec-ndk.c       ← AMediaCodec* C API wrapper
  gstamc-format-ndk.c      ← AMediaFormat* C API wrapper (pure NDK, no JNI)
  gstamc-internal-ndk.h    ← Internal NDK type declarations
  gstamc-ndk.h             ← Public NDK interface header
```

The build system (`meson.build`) conditionally compiles these files when the NDK header `media/NdkMediaCodec.h` is available:

```meson
if cc.check_header('media/NdkMediaCodec.h')
  androidmedia_sources += ndk_sources
  extra_cargs += [ '-DHAVE_NDKMEDIA' ]
endif
```

---

## What the NDK Path Actually Does

The merged NDK path (`HAVE_NDKMEDIA`) replaces **some but not all** JNI calls with direct `AMediaCodec*` C API calls via `dlopen("libmediandk.so")`:

### NDK-ified (pure C, no JNI)

| Operation | NDK function |
|---|---|
| Create codec by name | `AMediaCodec_createCodecByName` |
| Configure codec | `AMediaCodec_configure` |
| Start / stop / flush | `AMediaCodec_start` / `_stop` / `_flush` |
| Buffer dequeue/queue | `AMediaCodec_dequeueInputBuffer`, `AMediaCodec_queueInputBuffer`, `AMediaCodec_dequeueOutputBuffer`, `AMediaCodec_releaseOutputBuffer` |
| Format read (audio/video properties) | `AMediaFormat_getInt32`, `AMediaFormat_setInt32`, `AMediaFormat_getString`, etc. (all via `gstamc-format-ndk.c`, pure NDK) |

### Still JNI-backed (NOT eliminated)

| Operation | Why JNI remains |
|---|---|
| **Codec enumeration / listing** | `AMediaCodecList` NDK API lacks `getCodecInfoAt().getCapabilitiesForType().colorFormats` — GStreamer requires color format introspection that the NDK codec list API does not expose. Codec discovery (`gst_amc_codeclist_*`) remains JNI-backed. |
| **Surface texture (output rendering)** | `gstamc-codec-ndk.c` calls `gst_amc_jni_get_env()` to obtain a `JNIEnv*` and convert a Java `Surface` jobject to `ANativeWindow*` via `ANativeWindow_fromSurface(env, jobject)`. There is an explicit TODO comment: `/* TODO: support NDK-based ASurfaceTexture. */` |
| **GStreamer bootstrap / plugin init** | `gst_android_init(JNIEnv*, jobject)` is still required. The Cerbero-generated `gstreamer_android-1.0.c.in` bootstrap file defines `JNI_OnLoad` → registers native methods → Java calls `nativeInit()` → `gst_android_init()`. This path is unchanged in 1.24 and 1.26. |

### Architecture with HAVE_NDKMEDIA

```
JNI_OnLoad
  └─ gst_android_init(JNIEnv*, context)   ← STILL REQUIRED
       └─ gst_init_check()
            └─ androidmedia plugin loads
                 ├─ codec enumeration: JNI (AMediaCodecList lacks color format query)
                 └─ codec operation (HAVE_NDKMEDIA):
                       ├─ AMediaCodec_* C calls  ← NDK path (MR #4115)
                       └─ Surface → ANativeWindow: JNI  ← TODO in ndk code
```

---

## Relationship to `amcvideodec` (Java-backed) vs. New NDK Path

The `amcvideodec` GStreamer element is **not replaced** by MR #4115. It is the same element; the change is in the underlying codec operation implementation:

- **Before MR #4115**: `amcvideodec` used the JNI backend (`jni/gstamc-codec-jni.c`) exclusively — all codec calls routed through Java `android.media.MediaCodec` via JNI reflection.
- **After MR #4115** (GStreamer 1.24+): when built on an NDK that provides `<media/NdkMediaCodec.h>`, `amcvideodec` uses `ndk/gstamc-codec-ndk.c` for the codec operations themselves, reducing JNI call overhead per-frame. The element name (`amcvideodec`) and plugin (`androidmedia`) are unchanged.

This is an internal implementation swap, not a new element. The element factory `GstElementFactory::find("amcvideodec")` returns the same element regardless of whether it's backed by JNI or NDK.

---

## GStreamer 1.26 — What Changed for Android

GStreamer 1.26 (released 11 March 2025) has no further NDK MediaCodec architectural changes. The 1.26 release notes for Android cover:

- **Build system migration**: recommended build path changed from `Android.mk` to `CMake-in-Gradle` using `FindGStreamerMobile.cmake`. `Android.mk` deprecated; to be removed in 1.28.
- **Codec profile/format expansion**: More H.264/H.265 profiles, levels, and pixel format mappings added to `androidmedia` encoder/decoder (P010, packed 4:2:0 variants, RGBA layouts). Fixes decoder failures on devices that only support 'hardware surfaces output' paths.
- No changes to `gst_android_init` requirement or JNI_OnLoad bootstrap constraint.

The `ndk::media` direct-bindings alternative (Rust `ndk` crate, no GStreamer) remains unaffected.

---

## Implications for Phase 3 Android Implementation Plan

### Does MR #4115 obsolete the JNI_OnLoad approach from hud-4znng?

**No.** The JNI_OnLoad ordering constraint documented in hud-4znng (android-gstreamer-sdk-build-spike.md §6.2) remains fully in force:

```
JNI_OnLoad → gst_android_init() → [Android event loop] → gst_init() → pipeline
```

This ordering is required because:
1. `gst_android_init()` must run first to register the JNI callbacks used by androidmedia plugin initialization.
2. Even with HAVE_NDKMEDIA enabled, the androidmedia plugin still calls JNI for codec enumeration and (currently) surface texture setup.
3. The cerbero-generated `gstreamer_android-1.0.c.in` bootstrap code has not changed.

### What does improve with GStreamer 1.24+?

The NDK path reduces per-frame JNI overhead during decode. Operations like `dequeueOutputBuffer`, `releaseOutputBuffer`, and `queueInputBuffer` go through C function pointers rather than JNI method calls. This is a latency win for high-frame-rate streams but does not change the startup bootstrap sequence.

### Phase 3 build recommendation update

hud-4znng's Phase 3 build validation checklist (step 5: validate `androidmedia` plugin and `amcvideodec`) should now additionally verify HAVE_NDKMEDIA is enabled. Add to the checklist:

| Step | Validation | Pass condition |
|---|---|---|
| 5b. NDK codec path active | `adb logcat -s GStreamer \| grep "NDKMEDIA\|ndk codec"` OR check `cc.check_header('media/NdkMediaCodec.h')` output in meson build log | `HAVE_NDKMEDIA` defined in build |

The GStreamer Android SDK prebuilt for GStreamer 1.24+ includes the NDK media headers for API 21+, so `HAVE_NDKMEDIA` should activate automatically in the standard prebuilt environment (no additional configuration needed).

### Upgrade to gstreamer-rs 0.23 (GStreamer 1.24)?

hud-4znng defaulted to GStreamer 1.24.x as the SDK version. Since MR #4115 landed in 1.24.0, the NDK optimization is already included in the phase 3 baseline — no version upgrade from 1.24 to 1.26 is required for NDK MediaCodec support.

If upgrading to GStreamer 1.26 for other reasons (H.264/H.265 profile improvements, CMake build system migration), note that the `Android.mk` ndk-build approach documented in hud-4znng is deprecated in 1.26 and will be removed in 1.28. Phase 3 CI bootstrap (hud-685ha) should plan for CMake-in-Gradle if targeting GStreamer 1.26+.

---

## Summary Table

| Question | Answer |
|---|---|
| Is MR #4115 merged? | **Yes** — merged January 11, 2024, shipped in GStreamer 1.24 |
| Target GStreamer version | **1.24** (not 1.26) |
| New element name? | **No** — same `amcvideodec` element, internal implementation replaced |
| Plugin name? | `androidmedia` (unchanged) |
| Eliminates `gst_android_init` bootstrap? | **No** — JNI_OnLoad constraint unchanged; codec enumeration still JNI |
| Eliminates JVM dependency entirely? | **No** — codec listing and surface-to-ANativeWindow still JNI |
| Per-frame JNI overhead reduced? | **Yes** — codec start/stop/dequeue/queue now use AMediaCodec* C calls |
| Phase 3 plan (hud-4znng) obsoleted? | **No** — ordering constraint, `libgstandroidmedia.so` packaging, `gst_plugin_load_file()` all unchanged |
| Action for phase 3 CI (hud-685ha) | Verify HAVE_NDKMEDIA in build; if upgrading to 1.26, switch to CMake-in-Gradle |

---

## Discovered Follow-Ups

1. **hud-685ha interaction**: The CI bootstrap bead (hud-685ha) should confirm that `cc.check_header('media/NdkMediaCodec.h')` succeeds in the prebuilt GStreamer Android SDK 1.24 environment — if it does, HAVE_NDKMEDIA is enabled without extra work. If the prebuilt SDK does not expose `NdkMediaCodec.h` in its include path, this needs `pkg-config` or manual include path addition. Verify and document in the CI bootstrap.

2. **GStreamer 1.26 CMake migration** (if applicable to phase 3): `Android.mk` ndk-build is deprecated in 1.26 and removed in 1.28. If phase 3 targets 1.26+, hud-685ha must plan for `FindGStreamerMobile.cmake`-based Gradle integration rather than the `Android.mk` path documented in hud-4znng. Estimated extra effort: 0.5–1 day of Gradle/CMake config work.

3. **NDK ASurfaceTexture TODO** (post-phase-3): The explicit `TODO: support NDK-based ASurfaceTexture` in `gstamc-codec-ndk.c` means the remaining JNI path (surface object → ANativeWindow) could be eliminated in a future GStreamer release if this TODO is resolved. Monitor upstream for MRs completing this work; it would reduce (not eliminate) JNI dependency at decode time. For tze_hud's appsink-based model (no Surface sink), this is of lower priority.

---

## Sources

- GStreamer MR #4115 (upstream): https://gitlab.freedesktop.org/gstreamer/gstreamer/-/merge_requests/4115
- GStreamer 1.24 release notes (GitHub www repo): https://raw.githubusercontent.com/GStreamer/www/main/src/htdocs/releases/1.24/release-notes-1.24.md
- GStreamer 1.26 release notes (GitHub www repo): https://raw.githubusercontent.com/GStreamer/www/main/src/htdocs/releases/1.26/release-notes-1.26.md
- GStreamer monorepo androidmedia/ndk source: https://github.com/GStreamer/gstreamer/tree/main/subprojects/gst-plugins-bad/sys/androidmedia/ndk
- GStreamer meson.build for androidmedia: https://github.com/GStreamer/gstreamer/blob/main/subprojects/gst-plugins-bad/sys/androidmedia/meson.build
- NDK codec-ndk.c (merged commit facb000): https://github.com/GStreamer/gstreamer/blob/main/subprojects/gst-plugins-bad/sys/androidmedia/ndk/gstamc-codec-ndk.c
- Cerbero bootstrap template: https://github.com/GStreamer/cerbero/blob/master/data/ndk-build/gstreamer_android-1.0.c.in
- UBports community discussion (MR context and remaining JNI deps): https://forums.ubports.com/topic/11819/an-ndk-based-rendering-path-to-gstreamer-androidmedia-plugin
- Android GStreamer SDK spike (companion): `docs/audits/android-gstreamer-sdk-build-spike.md` (hud-4znng)
