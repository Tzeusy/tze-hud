# Android GStreamer HAVE_NDKMEDIA Public SDK Audit

**Issued for**: `hud-5dlpd`
**Date**: 2026-04-29
**Scope**: Verify whether the public GStreamer Android SDK material supports
hard-gating `HAVE_NDKMEDIA` in the Android bootstrap workflow.

---

## Verdict

**Documentation/update only; no workflow hard gate.**

The current Android bootstrap assertion should remain:

1. Hard-gate the public SDK download, checksum, archive layout, pkg-config
   resolution, and cargo-ndk link probes.
2. Run the `HAVE_NDKMEDIA` symbol/string inspection and expose
   `HAVE_NDKMEDIA_STATUS`.
3. Treat `not-confirmed` as a warning, not a failure, until source-built Cerbero
   logs or runtime/device evidence prove the NDK-backed MediaCodec path.

PR #617 already demonstrates the correct workflow behavior: the GStreamer 1.24.13
public SDK linked successfully for both Android targets, while `HAVE_NDKMEDIA`
could not be confirmed from the downloaded archive material.

---

## Public SDK Layout Observed

Authoritative listings checked on 2026-04-29:

- `https://gstreamer.freedesktop.org/pkg/android/`
- `https://gstreamer.freedesktop.org/pkg/android/1.24.13/`
- `https://gstreamer.freedesktop.org/data/pkg/android/1.24.13/`
- `https://gstreamer.freedesktop.org/pkg/android/1.28.2/`

The top-level Android package index now includes newer streams through 1.28.x and
1.29.x, but the project bootstrap workflow is pinned to `1.24.13`.

For the pinned `1.24.13` directory, the public listing contains only:

- `gstreamer-1.0-android-universal-1.24.13.tar.xz` (1.2G)
- `gstreamer-1.0-android-universal-1.24.13.tar.xz.asc`
- `gstreamer-1.0-android-universal-1.24.13.tar.xz.sha256sum`

No public per-ABI `arm64` or `x86_64` tarballs were listed for `1.24.13`. The
bootstrap workflow should therefore continue using the universal archive unless
the pin changes and the new pinned directory is verified.

---

## PR #617 CI Evidence

PR #617: `https://github.com/Tzeusy/tze-hud/pull/617`

Android bootstrap run: `https://github.com/Tzeusy/tze-hud/actions/runs/25070858144`

Observed job evidence from `Android GStreamer SDK bootstrap gate`:

- Downloaded and extracted `gstreamer-1.0-android-universal-1.24.13.tar.xz`.
- Gate 1 passed with `125` static archives in `arm64/lib`.
- Gate 2 did not find a separate `libgstandroidmedia.so`; it used the arm64
  static archive set for androidmedia inspection.
- Gate 3 passed: `pkg-config` resolved `gstreamer-1.0` to `1.24.13`.
- Gate 4 passed: cargo-ndk linked GStreamer via pkg-config for
  `aarch64-linux-android`.
- Gate 5 passed: cargo-ndk linked GStreamer via pkg-config for
  `x86_64-linux-android`.
- Gate 6 warning: neither `gst_amc_codec_ndk` / `gst_amc_format_ndk` symbols nor
  the `AMediaCodec_createCodecByName` dlsym string were found in the inspected
  material.
- Summary status: `HAVE_NDKMEDIA audit: not-confirmed`.

That combination proves the bootstrap/linking path is viable for the pinned
public SDK, but does not prove the MR #4115 per-frame NDK MediaCodec path is
compiled into the public prebuilt.

---

## Hard-Gate Policy

Do not hard-gate `HAVE_NDKMEDIA` on the current public archive inspection alone.
The archive does not expose enough evidence to distinguish:

- a public prebuilt compiled without `HAVE_NDKMEDIA`;
- a static or stripped packaging layout that hides the expected symbols/strings;
- an inspection target that is sufficient for linking but insufficient for
  androidmedia internals.

Hard-gating is appropriate only after one of these exists:

- Source-built Cerbero logs for the selected GStreamer version and NDK showing
  `-DHAVE_NDKMEDIA` while building `androidmedia`.
- Device/runtime evidence showing the NDK-backed codec implementation is active.
- Upstream package metadata or release artifact documentation explicitly stating
  the public Android prebuilt was built with `HAVE_NDKMEDIA`.

Until then, the workflow assertion should be: **link gates are mandatory;
`HAVE_NDKMEDIA` confirmation is advisory and visible**.

---

## Recommended Documentation State

`docs/ci/android-gstreamer-bootstrap.md` should describe the current pin as
`1.24.13`, the observed universal-only public archive layout, the absence of a
separate `libgstandroidmedia.so` in PR #617's CI extraction, and Gate 6 as a
warning-level assertion rather than a pre-phase-3 hard gate.
