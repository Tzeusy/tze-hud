# Runtime resident-memory envelope

The selected display profile freezes one restart-only resident-memory envelope.
Full-display uses 1024 MiB total: 512 MiB resources, 192 MiB widget source,
256 MiB widget raster, and 64 MiB fonts. Headless uses 512 MiB total: 256 MiB
resources, 64 MiB widget source, 128 MiB widget raster, and 64 MiB fonts.

The four classes are disjoint and cannot borrow capacity. A shared physical
allocation is charged once by stable allocation identity; distinct CPU and GPU
copies are charged separately. Logical agent texture accounting remains a
separate per-lease/aggregate admission domain and intentionally double-charges
shared logical references. Durable widget blobs on disk are not resident bytes.

After its accounting consumers are constructed, the runtime emits a structured
`tze_hud::resident_accounting` event. The JSON snapshot contains the selected
profile; session, lease, aggregate-scene, and hard admission ceilings; exact
resident class limits and current usage; allocation, denial, and safe-eviction
counters; deterministic byte-counting rules; and a consumer trace for resource,
image, widget-source, widget-raster, and font residency. These totals are
enforcement values, not exact allocator metadata, GPU-driver heap usage, or RSS.

Before compositor cache admission, stale entries not referenced by the current
frame are released through the shared ledger. Current-frame entries stay pinned.
Optional raster/image work falls back to the existing uncached, old-cache, or
lower-quality path when no safe headroom remains; mandatory resource admission
returns a structured budget error. No class borrows unused capacity from another.
