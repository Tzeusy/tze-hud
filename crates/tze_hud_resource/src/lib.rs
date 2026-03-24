//! # tze_hud_resource
//!
//! Content-addressed resource store for tze_hud.
//!
//! Implements RFC 0011 upload and deduplication requirements:
//!
//! - **BLAKE3 content addressing**: `ResourceId` = 32-byte BLAKE3 digest of raw bytes.
//! - **Deduplication**: `DedupIndex` checks `expected_hash` on `ResourceUploadStart`; returns
//!   existing resource with `was_deduplicated = true` if found.
//! - **Inline fast path**: resources ≤ 64 KiB upload in a single message.
//! - **Chunked upload**: three-phase flow for resources > 64 KiB.
//! - **Validation pipeline**: capability, hash integrity, size limits, budget,
//!   type check, decode validation.
//! - **Concurrent upload limits**: max 4 per agent.
//!
//! ## Crate structure
//!
//! | Module | Contents |
//! |---|---|
//! | [`types`] | `ResourceId`, `ResourceType`, error codes, size constants |
//! | [`dedup`] | Content-addressed dedup index (`DedupIndex`, `ResourceRecord`) |
//! | [`validation`] | Six-step upload validation pipeline |
//! | [`upload`] | Upload state machine and `ResourceStore` |

pub mod dedup;
pub mod types;
pub mod upload;
pub mod validation;

pub use dedup::{DedupIndex, ResourceRecord};
pub use types::{
    DecodedMeta, ResourceError, ResourceId, ResourceStoreConfig, ResourceStored, ResourceType,
    CHUNK_SIZE_LIMIT, DEFAULT_MAX_DECODED_TEXTURE_BYTES, DEFAULT_MAX_RESOURCE_BYTES,
    DEFAULT_MAX_TOTAL_TEXTURE_BYTES, INLINE_SIZE_LIMIT, MAX_CONCURRENT_UPLOADS_PER_AGENT,
    MAX_TEXTURE_DIMENSION_PX,
};
pub use upload::{ResourceStore, UploadId, UploadStartRequest};
pub use validation::{AgentBudget, check_capability, check_hash, CAPABILITY_UPLOAD_RESOURCE};
