# CVMetalTextureCache Zero-Copy GPU Upload for VT Frames

**Issue**: hud-qrd4q  
**Status**: Design note (post-v2 optimization)  
**Priority**: P3 — performance polish  
**Date**: 2026-04-19

---

## Baseline: Current CPU Memcpy Path

In `crates/tze_hud_media_apple/src/session.rs`, the `VtDecodeSession` (from PR #542 hud-l0h6t) currently follows a safe, CPU-side buffering model:

1. **VT Hardware Decode** (`VTDecompressionSession` on Apple hardware) produces a `CVPixelBuffer` in NV12 format (kCVPixelFormatType_420YpCbCr8BiPlanarFullRange).
2. **CPU Lock & Copy** (`CVPixelBufferLockBaseAddress` → `memcpy` → `CVPixelBufferUnlockBaseAddress`) transfers the decoded pixels into a CPU-owned `Vec<u8>`.
3. **Tokio Channel Bridge** delivers the `DecodedFrame` (containing the `Vec<u8>`) to the compositor task via `mpsc::Receiver<DecodedFrame>`.
4. **wgpu Upload** (`Queue::write_texture`) copies the CPU buffer to GPU texture memory.

This path is **safe and correct** — `CVPixelBuffer` is retained only within the VT callback scope. However, it incurs:
- **Per-frame CPU allocation** (`Vec<u8>` per frame, ~3MB per 1080p frame)
- **Two memory copies**: CVPixelBuffer → CPU heap, CPU heap → GPU VRAM
- **Stall risk**: If the GPU upload queue backs up, the CPU buffer persists in memory longer than necessary.

---

## Opportunity: Zero-Copy via CVMetalTextureCache

On iOS with Metal (all iOS 9+ devices), `CVMetalTextureCache` provides a direct GPU upload path:

### How CVMetalTextureCache Works

1. **IOSurface Backing**: `CVPixelBuffer` on iOS is typically backed by an IOSurface (a kernel-managed GPU-accessible memory pool).
2. **CVMetalTextureCache**: A CoreVideo utility that wraps a `CVPixelBuffer` as a `MTLTexture` **without copying**, directly referencing the IOSurface.
3. **wgpu-hal Metal Bridge**: The `wgpu-hal` crate exposes a Metal backend; raw `MTLTexture` pointers can be wrapped into `wgpu-hal` Metal resources and composed into a `wgpu::Texture` binding.

### Elimination of Memcpy

```
VT Hardware Decode
    ↓
CVPixelBuffer (IOSurface-backed, GPU-readable)
    ↓
CVMetalTextureCache::CreateTextureFromImage
    ↓
MTLTexture (zero-copy reference into IOSurface)
    ↓
wgpu-hal MTLTexture → wgpu::Texture → Bind & Render
```

**Result**: No CPU buffer allocation, no memcpy. The compositor reads directly from the GPU-resident IOSurface.

---

## API Surface Sketch

A new function on `VtDecodeSession` to opt into the zero-copy path:

```rust
impl VtDecodeSession {
    /// Enable zero-copy GPU upload via CVMetalTextureCache.
    ///
    /// This method gates the zero-copy path and must be called **before**
    /// the session begins receiving frames. It requires:
    /// 1. Metal backend availability (checked at runtime)
    /// 2. IOSurface-backed CVPixelBuffers (standard on iOS hardware decode)
    ///
    /// If the Metal backend is unavailable, returns an error; the session
    /// falls back to the safe CPU-copy path.
    ///
    /// # Errors
    ///
    /// - [`VtError::MetalBackendUnavailable`] if wgpu is not using Metal
    /// - [`VtError::CVMetalTextureCacheCreateFailed`] on cache creation failure
    pub fn enable_zero_copy_gpu_upload(&mut self) -> Result<(), VtError> {
        // Create a CVMetalTextureCache bound to the Metal device
        // Store the cache in VtDecodeSession state
        // Set a flag to route frames through the zero-copy path
        todo!()
    }
}
```

**Routing in the callback**:
- If zero-copy path is enabled: `CVMetalTextureCache::CreateTextureFromImage(cv_pixel_buf) → MTLTexture → wrap in wgpu-hal → send MTLTextureRef over channel`
- If zero-copy path is disabled or unavailable: fall back to safe CPU-copy memcpy path

---

## Challenges & Mitigation

### 1. wgpu-hal Metal Interop API Availability

**Risk**: wgpu-hal's public Metal backend API may not expose raw MTLTexture wrapping.

**Mitigation**: 
- Check `wgpu-hal` v0.19+ (post-v2 timeline) for `metal::Device::texture_from_metal` or equivalent.
- If the API is unavailable or unstable, implement as an internal unsafe wrapper over `wgpu-hal`'s Metal device backend.
- Keep the fallback path (CPU memcpy) as the default; zero-copy is an opt-in accelerator.

### 2. Apple Private-API Caveats in VideoToolbox Buffer Pool

**Risk**: VideoToolbox's buffer pool and IOSurface lifecycle are private APIs. The CVPixelBuffer lifetime semantics within a callback may have undocumented constraints.

**Mitigation**:
- The callback must not hold the MTLTexture past the callback scope. Reference counting on the MTLTexture must be tight.
- The `MTLTexture` should be converted to a wgpu-owned binding immediately; do not pass raw MTL pointers across thread boundaries.
- Test extensively on real iOS hardware across iOS versions (iOS 14+, targeting the v2 device lane).

### 3. CPU vs GPU Frame Access Synchronization

**Risk**: If the VT decode hardware and GPU renderer are accessing the same IOSurface concurrently, race conditions may cause visual corruption or stalls.

**Mitigation**:
- Rely on Metal's built-in command-queue synchronization: the wgpu render pass will not begin until the prior decode's MTLBlitCommandEncoder has committed.
- Ensure the callback does not fire on the render thread — it fires on VT's internal thread. The channel deliver happens asynchronously, so the IOSurface is safe for re-use by VT once the wgpu render passes complete (managed by the compositor's frame budget).
- Document the synchronization contract clearly: "Zero-copy path assumes IOSurface is not re-used by VT until the wgpu frame has rendered."

---

## Effort Estimate

Once the baseline v2 iOS implementation (hud-l0h6t) ships and is stable:

- **wgpu-hal MTLTexture interop API research**: 0.5 days
- **CVMetalTextureCache integration** (unsafe FFI wrapper): 1 day
- **Unit tests** (callback path, lifetime validation): 0.5 days
- **Real device validation** (iOS 14+ on iPhone/iPad, visual + latency): 1 day

**Total**: 2–3 days. This is feasible as a post-v2 performance improvement once the baseline is shipping.

---

## Why Post-v2

1. **v2 scope is feature delivery**, not optimization. CPU memcpy per frame is acceptable for the initial iOS pilot (1–2 concurrent streams).
2. **wgpu-hal API stability**: Post-v2 (late 2026) is a safer target for relying on undocumented Metal interop APIs.
3. **Real device feedback loop**: The v2 iOS device lane (hud-l0h6t) will reveal actual frame drop / latency impact in real use. If profiling shows the CPU memcpy is a bottleneck, this work becomes a priority. If not, defer indefinitely.
4. **Fallback path is robust**: The safe CPU memcpy path will ship in v2 and can carry the iOS media plane until this optimization lands.

---

## Cross-References

- **PR #542** (hud-l0h6t): "Safe VtDecodeSession skeleton crate" — baseline v2 iOS decode implementation
- **hud-urfbw**: Review PR for hud-l0h6t, discovered this optimization opportunity
- **docs/audits/ios-videotoolbox-alternative-audit.md** §7.3: Tokio bridge pattern and CVPixelBuffer upload semantics
- **crates/tze_hud_media_apple/src/frame.rs**: Current `DecodedFrame` structure (CPU buffer ownership model)
- **crates/tze_hud_media_apple/src/session.rs**: `VtDecodeSession` callback and channel delivery

---

## Summary

CVMetalTextureCache zero-copy GPU upload is a straightforward post-v2 optimization that eliminates the CPU memcpy bottleneck in iOS frame decode. The API surface is small (~3 function additions to `VtDecodeSession`), the fallback path is safe and already shipping, and the risk is manageable with tight synchronization discipline. File this as a follow-up P3 bead once v2 iOS validation begins.
