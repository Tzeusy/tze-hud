# Console-hidden portal resize hotkeys: capture blocked (hud-3bmcd)

**Verdict: BLOCKED.** The console-hidden input path completed, but this single
capture does not prove that the focused portal visibly grew or shrank. It must
not be used as a passing runtime confirmation.

## Change under test

The generated diagnostic-input script now hides the interactive scheduled
task's own PowerShell console with `GetConsoleWindow` and
`ShowWindow(..., 0)`, then waits 400 ms before loading the input interop or
executing an action ([generator](../../../.claude/skills/user-test/scripts/text_stream_portal_exemplar.py#L1227-L1247)).
It deliberately does not use output redirection or a no-window launch mode,
which could alter the transparent-overlay execution path. The sequence follows
the established re-verification driver ([driver](liveverify-resize-reverify-20260711/resize_injection_driver.py#L92-L107)).

The focused regression test asserts that the hide, hide-window call, and
settle delay are all generated before the first diagnostic action
([test](../../../.claude/skills/user-test/tests/test_text_stream_portal_exemplar.py#L250-L264)).

## Redacted live result

The current deployed executable had SHA-256
`23239a6e0cfa4e7ff776d8c3251d5f1716ef72bbef35c225073f4d03e4154a2b`.
The scheduled diagnostic step completed with `ok=true`, exit code `0`, and a
12.039-second duration. It recorded the existing `input:pointer-down-unhandled`
checkpoint. That establishes only that the injector completed; it does not
establish delivered resize chords or portal geometry.

| Capture | Pixels | Bytes | SHA-256 |
| --- | ---: | ---: | --- |
| Baseline | 2560 x 1440 | 348,286 | `15e6e27ac3e17c8d6927dbfaa6773db905c057e27e928d05bbda45880a29cf0c` |
| Grow | 2560 x 1440 | 17,073 | `aed9ef3677cf567335954caead701787ea69982c54748a34114cb05f885b55af` |
| Shrink | 2560 x 1440 | 348,286 | `15e6e27ac3e17c8d6927dbfaa6773db905c057e27e928d05bbda45880a29cf0c` |

| Comparison | Result | Absolute-error pixels |
| --- | --- | ---: |
| Baseline / grow | Different | 3,686,400 |
| Grow / shrink | Different | 3,686,400 |
| Baseline / shrink | Byte-identical | 0 |

The grow image was uniformly opaque black (zero mean and standard deviation);
the baseline and shrink images had the same non-black statistics and were
byte-identical. Thus nonidentity here proves a capture-state change, not a
visible grow/shrink geometry change.

No raw desktop images, target identity, credentials, or transcript are
retained in this evidence. Raw capture files were used only to derive the
metrics above and then removed from both the local staging directory and the
target.

## Required follow-up

The original console-steal/byte-identical failure is no longer the observed
pattern, but the all-black grow frame is an independent capture, overlay, or
runtime-validation blocker. Under the single-rerun constraint, this result
cannot distinguish those causes. A later authorized investigation must first
explain or eliminate the black capture, then produce non-black baseline, grow,
and shrink images with visible, attributable geometry deltas.
