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

At startup the runtime emits a structured `tze_hud::resident_accounting` event
containing the profile, exact byte ceilings, current per-class usage, aggregate
usage, and allocation count. Class admission failure uses an existing safe
eviction/no-cache boundary; it never borrows from another class.
