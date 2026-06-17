# windows-runtime-scope Specification
Status: implemented

## Purpose
Defines the active deployment-scope discipline for the single-Windows performance
refocus: which runtime target is authoritative, how deferred media/multi-device
contracts are marked inactive, and the calibration gate that governs when Windows
performance budgets may be locked.

Implementation: about/heart-and-soul/v1.md; about/craft-and-care/engineering-bar.md; scripts/ci/check_windows_perf_budgets.py

## Requirements

### Requirement: Windows-First Active Runtime Scope

The project SHALL treat the native Windows HUD runtime as the active deployment
target for the next performance and release work. macOS and Linux desktop SHALL
remain compile and CI correctness targets only until the Windows runtime meets
the calibrated performance bar.

Source: about/heart-and-soul/v1.md, RFC-level refocus decision (epic hud-9wljr)
Scope: v1-mandatory

#### Scenario: New work is admitted against Windows-first scope

- **WHEN** a new implementation bead or OpenSpec change is proposed during this
  refocus
- **THEN** it MUST either improve the native Windows runtime path or explicitly
  document why it is single-device support work that does not expand the
  platform, media, or embodied-presence surface

### Requirement: Deferred Media And Multi-Device Contracts

The project SHALL preserve the v2 media, mobile, glasses, embodied-presence,
and cross-machine validation documents as historical references while marking
them inactive for current implementation. Reactivating any deferred surface
SHALL require a fresh OpenSpec proposal after the Windows runtime performance
bar is delivered.

Source: about/heart-and-soul/v2.md, about/heart-and-soul/media-doctrine.md, about/heart-and-soul/mobile.md
Scope: v1-mandatory

#### Scenario: Deferred documents are not implementation authority

- **WHEN** reviewers inspect `v2.md`, `mobile.md`, `media-doctrine.md`,
  `media-webrtc-bounded-ingress`, `media-webrtc-privacy-operator-policy`, or
  `cross-machine-runtime-validation`
- **THEN** each surface MUST carry a top-of-file deferral block that points
  current implementation work to `windows-first-performant-runtime`

### Requirement: Reference-Hardware Budget Calibration

The proposed Windows performance budgets SHALL remain provisional until measured
against a documented reference Windows machine. Locked budgets SHALL be recorded
in the engineering quality bar only after a baseline report captures the
reference hardware, benchmark commands, and observed gaps.

Source: about/craft-and-care/engineering-bar.md §2, docs/reports/windows_perf_baseline_2026-05.md
Scope: v1-mandatory

#### Scenario: Budgets are locked only after baseline evidence

- **WHEN** a task attempts to enforce a Windows runtime performance budget in CI
  or `about/craft-and-care/engineering-bar.md`
- **THEN** the change MUST cite a baseline report that includes the reference
  hardware and benchmark evidence used to calibrate that budget
