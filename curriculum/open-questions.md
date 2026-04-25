# Open Questions

- How much `GStreamer` and `WebRTC` should a newcomer learn before first contribution work?
  Current answer: only glossary-level understanding is required for v1-safe work because media is explicitly deferred. This should be revisited if the active workstream shifts to V2/media-plane implementation.

- Should `tze_hud_policy` become a required deep prerequisite for v1 contributors?
  Current evidence says no. The spec documents the target arbitration model, but current authority remains split across runtime/session/scene surfaces. That makes policy concepts mandatory, but the pure evaluator crate itself is not yet the hot-path centerpiece.

- How much Windows deployment knowledge should be treated as mandatory?
  The repo clearly supports cross-machine Windows deployment as an operational reality, but not every contributor needs deep platform-ops literacy. The current curriculum keeps deployment at “working” rather than “implementation” depth unless the learner is touching runtime startup or operator workflows.

- Does the media seam deserve a second curriculum path later?
  Probably yes, once the repo’s active work centers on bounded-ingress media, embodied presence, or device-profile execution. At the moment, forcing a second path would over-teach deferred scope.

- Should repo knowledge architecture become its own module?
  Current decision: no. Doctrine/RFC/OpenSpec authority is important, but it is contextual glue rather than a standalone technical concept. If contributors keep misplacing changes, this decision should be revisited.
