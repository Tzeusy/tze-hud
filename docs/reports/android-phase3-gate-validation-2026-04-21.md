# Android Phase 3 Gate Validation (2026-04-21)

## Scope

Validate pre-phase-3 checklist items that require JVM/device execution:

- Gate 12: `gst_android_init()` call site exists in JNI entry point before `gst_init()`.
- Gate 13: on-device `amcvideodec` factory registration is observed on Pixel.

Reference checklist: `docs/ci/android-gstreamer-bootstrap.md` §7.

## Validation Results

### Gate 12 (`gst_android_init` ordering)

Status: **Blocked**

Evidence collected:

```bash
rg -n "JNI_OnLoad|gst_android_init\\(" crates app examples tests -S
```

Observed result:

- No Android JNI entry point implementation (`JNI_OnLoad`) exists yet in the repository.
- `crates/tze_hud_media_android/src/lib.rs` currently documents the required order but is still scaffold-only.

Conclusion:

- Gate 12 cannot be passed until phase-3 Android runtime implementation lands the actual JNI bootstrap path.

### Gate 13 (on-device `amcvideodec` registration)

Status: **Blocked**

Evidence collected:

```bash
adb version
```

Observed result:

```text
zsh:1: command not found: adb
```

Conclusion:

- This worker environment does not have Android platform-tools (`adb`), and therefore cannot execute the required on-device logcat verification.
- Gate 13 remains blocked pending a worker lane with `adb` + a connected D19 Pixel target.

## Resume Conditions

1. Phase-3 Android implementation adds JNI bootstrap (`JNI_OnLoad -> gst_android_init -> gst_init` ordering) so Gate 12 can be code-reviewed against concrete code.
2. A device-capable validation lane is available (`adb` installed + D19 Pixel connected) to run the Gate 13 logcat probe and capture pass/fail evidence.
