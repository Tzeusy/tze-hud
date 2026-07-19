# `hud-t1900` reference-Chrome enclosing-page rerun

- Date: 2026-07-16
- Window: 2026-07-16T14:51:22Z–15:04:22Z
- Checkout commit: `2084c8d204859fa7f1b513f135279539ec64f6b3`
- Branch: `agent/hud-t1900`
- Target: private TzeHouse Windows host, resolved through the user-test helper
- Profile decision: reuse the already-running implicit default Chrome profile;
  do not reset or change cookies, consent, preferences, or profile flags

## Verdict

**The enclosing-page request path is proved, but player playback remains an
operator-only hard gate.**

The existing Chrome browser requested the loopback page over HTTP and received
`200`. The page then loaded the official YouTube IFrame API, whose callback
reported to the same loopback page with a loopback-enclosing-page referrer. This
proves the browser used an enclosing page rather than navigating directly to an
embed URL.

The player did not emit `onReady`, a playing state, or `onError(153)` during the
35-second observation. Absence of an error callback is not evidence that Error
153 is absent. The result is therefore **indeterminate**, not a playback pass.
No cookie/profile theory is supported by this run.

## Request and browser path

The instrumented page kept the canonical source-evidence properties:

- top-level URL shape:
  `http://127.0.0.1:<ephemeral-port>/youtube_source_evidence.html`;
- official player video: `O0FGCxkHM-U`;
- player URL shape:
  `https://www.youtube.com/embed/VIDEO_ID?enablejsapi=1&origin=http://127.0.0.1:<ephemeral-port>`;
- referrer policy: `strict-origin-when-cross-origin`;
- visible player controls retained;
- no direct media URL, download, extraction, cache, audio bridge, or HUD frame
  ingress.

The official IFrame API was used only to report semantic player events back to
the same loopback server. The observed sequence was:

1. Chrome requested `/youtube_source_evidence.html`; the server returned `200`.
2. The external IFrame API loaded and invoked `onYouTubeIframeAPIReady`.
3. The page reported `iframe_api_ready` with a loopback-enclosing-page referrer.
4. No player-ready, playing, or player-error event arrived before timeout.

See [browser-path.json](browser-path.json) for the sanitized event record.

## Why the prior direct-embed reproduction is not equivalent

A top-level navigation to `https://www.youtube.com/embed/...` is not an embed in
an enclosing page. It bypasses the loopback document that supplies the expected
origin and referrer context. Error 153 observed on that direct navigation cannot
be used to diagnose the existing Chrome profile or to fail the canonical
enclosing-page path.

This run deliberately set `direct_embed_attempted=false`; it did not repeat the
invalid direct-navigation test.

## Profile and HUD preservation

The preflight found one long-running Chrome browser process using its implicit
default profile, without a remote-debugging flag or listener. The test invoked
the installed Chrome executable with only the loopback URL. It added no Chrome
flags and performed no settings, cookie, consent, preference, profile-directory,
or user-data-directory action.

The browser PID and start time were identical before and after the run. This
proves reuse of the existing browser process; it does not claim that normal
browser runtime data such as history was byte-for-byte unchanged after opening
a page.

The user-test environment helper was invoked only with `--host-only`, using its
documented ignored-target override. Its HUD self-heal path was not invoked. The
HUD PID/start time, MCP and gRPC listeners, and GPU-lock timestamp/hash are
identical in [state-before.json](state-before.json) and
[state-after.json](state-after.json).

The event probe ran as an ephemeral interactive scheduled task because a
non-interactive SSH-launched Chrome process produced zero loopback requests. The
interactive task was unregistered after the run. Its script/result files and all
targeted temporary artifacts were removed; the postflight reports no remaining
task or artifact.

No scheduled-task XML was read or captured. No host, Windows user, SSH-key path,
or PSK value appears in these artifacts.

## Sanitized command shapes

```text
TZEHOUSE_TARGET_ENV=<ignored-target.env> \
  .claude/skills/user-test/scripts/tzehouse_env.sh --host-only

ssh <admin-target> powershell -NoProfile -EncodedCommand <sanitized-read-only-preflight>

# Ephemeral interactive task, automatically unregistered:
powershell.exe -NoProfile -ExecutionPolicy Bypass -File <temporary-event-probe.ps1>

ssh <admin-target> powershell -NoProfile -EncodedCommand <sanitized-postflight-and-targeted-cleanup>
```

## Remaining operator gate

An operator must be present at the unlocked TzeHouse desktop for the next run:

1. Launch the source-evidence page through the loopback enclosing page. Do not
   navigate directly to `youtube.com/embed/...`.
2. In the existing reference Chrome profile, confirm the address bar begins
   `http://127.0.0.1:` and ends `/youtube_source_evidence.html`.
3. Observe the official player for at least 10 seconds. A pass requires visible
   controls and visibly advancing video for `O0FGCxkHM-U`, with no Error 153.
4. If the player is blank, stalled, or shows an error, record only the visible
   status/error code. Do not reset cookies, consent, preferences, or the profile.
5. Correlate the human observation with the loopback event trace before closing
   `hud-t1900`.

Until that visual observation exists, this evidence must not be promoted to a
live player-render or playback-success claim.
