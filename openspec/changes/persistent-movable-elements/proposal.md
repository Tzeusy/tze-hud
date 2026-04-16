## Why

Doctrine and RFC reconciliation found blocking contract conflicts in the persistent movable elements direction. These conflicts must be resolved before implementation beads proceed so runtime behavior, v1 scope, and wire contracts remain coherent.

## What Changes

- Amend v1 persistence boundary to carve out a durable element identity store.
- Amend RFC 0004 input model to explicitly allow compositor-internal chrome drag-handle behavior outside the v1-reserved agent gesture pipeline.
- Amend RFC 0001 scene contract to define `PublishToTileMutation`, runtime override application in the transaction pipeline, drag-handle chrome addressing, and element-store durability.
- Amend drag-to-reposition OpenSpec requirements to remove v1-deferred visual effects and define an unambiguous reset interaction for touch/mobile-style input.
- Amend element-identity-store OpenSpec requirements to defer explicit deletion to post-v1 and clarify monotonic growth in v1.

## Impact

- **Doctrine:** `about/heart-and-soul/v1.md`
- **RFCs:** `about/legends-and-lore/rfcs/0004-input.md`, `about/legends-and-lore/rfcs/0001-scene-contract.md`
- **OpenSpec change:** `openspec/changes/persistent-movable-elements/`

No runtime or protocol implementation code is changed in this bead.
