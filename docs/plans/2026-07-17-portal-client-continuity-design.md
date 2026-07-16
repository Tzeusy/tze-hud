# Portal Client Continuity Design

## Decision

Projection continuity is client-owned durability, not runtime persistence.
The in-process projection authority may retain its bounded coherent visible
window and reconnect bookkeeping only while that runtime lives. The preferred
`portal_client.py` retains a bounded tail of output that the client itself
authored and explicitly replays it after authenticated attach.

This reconciles two text-stream portal requirements. Cooperative Projection
State Externality keeps retained/full history outside the scene graph and
runtime core. Portal Reconnect and Resume Presentation still allows the live
authority to resume its bounded visible window during lease grace and defines
`logical_unit_id` idempotency plus `coalesce_key` replacement semantics. A
runtime restart or grace expiry creates a fresh portal; client replay rebuilds
only the local authored tail and does not claim the former runtime state
survived.

## State and bounds

The client stores one JSON file per safe projection ID below
`$XDG_STATE_HOME/tze_hud/portal-continuity/`, with a 0700 directory, 0600 file,
and same-directory atomic replacement. The schema contains a version, the
original attach idempotency key, and allowlisted authored output records. Each
record contains text, kind, classification, stable `logical_unit_id`, and an
optional `coalesce_key`. Owner tokens remain in their separate token file;
viewer-authored input, pending queues, acknowledgements, and arbitrary response
data are never stored.

The rolling tail keeps at most 64 records and 64 KiB of canonical UTF-8 record
data. Oldest records are evicted first. A newer record with the same
`coalesce_key` replaces the earlier local record in place, matching the
authority's visible-window semantics. Corrupt or schema-invalid state is moved
to a private `.corrupt` file and excluded from replay. Local state deletion is
an explicit `continuity-clear` operation; detach and remote cleanup do not
silently destroy it.

## Attach and replay

Attach reuses a retained idempotency key unless the caller supplies the same
key explicitly. A mismatched explicit key fails before network access. After a
successful attach, the client atomically stores the returned owner token and
the attach metadata, then replays retained records in order before returning.
Replay uses the stored logical and coalescing keys and current owner token.

Repeating attach/replay against a live authority is idempotent because every
record uses the same `logical_unit_id`. Against a fresh authority, the same
records reconstruct the bounded tail. If replay fails partway, local state is
not advanced or discarded; another attach can retry, and already accepted
records deduplicate. A repeated live publish with an already-retained
`logical_unit_id` preserves the original local record once, matching the
authority's accepted no-op. A definitively rejected live publish restores the
previous local tail; an ambiguous transport or response failure retains the
prepared record so the stable identity can be replayed safely. An atomic-write
failure occurs before network publication and preserves the prior file.

## Verification

Tests cover deterministic item and byte bounds, coalesce and logical-identity
deduplication, private permissions and atomic writes, corrupt-state quarantine,
owner-token and input exclusion, stable double replay, fresh-runtime
reconstruction, definitive-rejection rollback, ambiguous-outcome retention,
storage rollback, and explicit cleanup. Existing token-rotation and MCP-dialect
tests remain part of the focused suite. The final gate includes the full
user-test Python suite, skill-package audit, Ruff formatting/lint, Python
compilation/help smoke tests, documentation links, and repository workspace
guards that do not require a live Windows or GPU target.
