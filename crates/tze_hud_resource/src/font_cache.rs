//! LRU font cache with permanent system/bundled font holds.
//!
//! ## Font source hierarchy
//!
//! 1. System fonts (platform font directories)
//! 2. Bundled fonts (compiled into the binary)
//! 3. Agent-uploaded fonts (`ResourceId` lookup → fallback to `SystemSansSerif`)
//!
//! ## GC and eviction rules
//!
//! - System and bundled fonts have **permanent implicit holds**: they are never
//!   GC'd and never evicted from the cache (spec lines 255–258, 261–262).
//! - Agent-uploaded fonts are evicted **LRU** when the cache exceeds its
//!   maximum size (default 64 MiB — spec lines 270–272, 276–277).
//!
//! ## Font family resolution
//!
//! Resolution order (spec lines 255–258):
//! 1. Named variant from the display profile.
//! 2. Custom `ResourceId` lookup; fallback to `SystemSansSerif` if the
//!    `ResourceId` is not in the store (transparent to the agent — no
//!    notification sent).
//! 3. Bundled default (`SystemSansSerif`).
//!
//! ## What this module provides
//!
//! This is a **metadata-level** cache.  In v1, the runtime does not perform
//! GPU texture atlas management inside `tze_hud_resource` (that lives in the
//! compositor).  This module tracks:
//!
//! - Which font faces are logically "cached" and their approximate byte cost.
//! - LRU eviction of agent-uploaded fonts.
//! - Permanent-hold semantics for system/bundled fonts.
//!
//! A `CachedFontHandle` is an opaque token the compositor can hold; it does
//! not carry actual font data in this crate.
//!
//! Source: RFC 0011 §7.1–§7.5; spec lines 255–277.

use std::collections::{HashMap, VecDeque};

use crate::types::ResourceId;

// ─── Font origin ──────────────────────────────────────────────────────────────

/// Distinguishes the source of a font entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FontOrigin {
    /// System font (platform font directory).
    System,
    /// Bundled font (compiled into the binary).
    Bundled,
    /// Agent-uploaded font (content-addressed `ResourceId`).
    Agent,
}

impl FontOrigin {
    /// `true` if this origin has a permanent implicit hold (never evicted).
    #[inline]
    pub fn is_permanent(&self) -> bool {
        matches!(self, FontOrigin::System | FontOrigin::Bundled)
    }
}

// ─── Font cache key ───────────────────────────────────────────────────────────

/// Key used to look up fonts in the cache.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum FontCacheKey {
    /// System or bundled font identified by a stable name (e.g. "SystemSansSerif").
    Named(String),
    /// Agent-uploaded font identified by content-addressed `ResourceId`.
    Resource(ResourceId),
}

// ─── Cache entry ──────────────────────────────────────────────────────────────

/// A single entry in the font cache.
#[derive(Debug, Clone)]
pub struct FontCacheEntry {
    /// Source of this font.
    pub origin: FontOrigin,
    /// Approximate in-memory size (loaded face + shaped glyph cache + rasterized atlas).
    pub byte_cost: usize,
}

// ─── Opaque handle ────────────────────────────────────────────────────────────

/// Opaque handle returned when a font is accessed from the cache.
///
/// The compositor holds these handles; they do not carry raw font data in this
/// crate.  Dropping a handle does NOT evict the font — eviction is managed
/// by the LRU policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CachedFontHandle(FontCacheKey);

impl CachedFontHandle {
    pub fn key(&self) -> &FontCacheKey {
        &self.0
    }
}

// ─── FontCache ────────────────────────────────────────────────────────────────

/// LRU font cache bounded by a configurable memory limit.
///
/// ## Thread safety
///
/// `FontCache` uses `&mut self` for mutations.  It is intended for use on the
/// compositor thread and is not internally synchronized.
pub struct FontCache {
    /// Maximum total byte cost allowed across cached agent-uploaded fonts.
    /// System/bundled fonts are excluded from this limit (permanent holds).
    max_agent_bytes: usize,
    /// Current total byte cost of agent-uploaded fonts.
    agent_bytes_used: usize,
    /// All cache entries (system, bundled, and agent).
    entries: HashMap<FontCacheKey, FontCacheEntry>,
    /// LRU order for agent-uploaded fonts only (most-recently-used is at back).
    lru: VecDeque<FontCacheKey>,
}

impl FontCache {
    /// Create a new font cache.
    ///
    /// `max_agent_bytes`: maximum memory for agent-uploaded fonts (default 64 MiB).
    /// Pass 0 for unlimited (not recommended in production).
    pub fn new(max_agent_bytes: usize) -> Self {
        Self {
            max_agent_bytes,
            agent_bytes_used: 0,
            entries: HashMap::new(),
            lru: VecDeque::new(),
        }
    }

    /// Insert or refresh a **system or bundled** font.
    ///
    /// Permanent fonts are never evicted.  Inserting a permanent font that is
    /// already present is a no-op (the entry is not refreshed).
    pub fn insert_permanent(&mut self, key: FontCacheKey, entry: FontCacheEntry) {
        debug_assert!(
            entry.origin.is_permanent(),
            "insert_permanent called with non-permanent origin {:?}",
            entry.origin
        );
        self.entries.entry(key).or_insert(entry);
    }

    /// Insert or refresh an **agent-uploaded** font.
    ///
    /// If the font is already cached, it is promoted to the back of the LRU
    /// queue (most-recently-used).  If not present, it is inserted and the
    /// LRU eviction policy runs to reclaim space if needed.
    ///
    /// Returns the handle for the font.
    pub fn insert_agent(&mut self, resource_id: ResourceId, byte_cost: usize) -> CachedFontHandle {
        let key = FontCacheKey::Resource(resource_id);

        if self.entries.contains_key(&key) {
            // Promote in LRU.
            self.lru_promote(&key);
            return CachedFontHandle(key);
        }

        // Evict LRU entries until there is room.
        //
        // NOTE: If `byte_cost` is larger than `max_agent_bytes` (or the cache
        // is already empty), the eviction loop exhausts all candidates and
        // breaks, and the oversized entry is still admitted.  This is
        // intentional: the spec (lines 276–277) mandates evicting the LRU
        // agent-uploaded font when the budget is exceeded, but does not prohibit
        // admitting a single entry that cannot fit.  The compositor must not
        // rely on `agent_bytes_used <= max_agent_bytes` as a hard invariant;
        // treat `max_agent_bytes` as a soft target, not a hard cap per entry.
        if self.max_agent_bytes > 0 {
            while self.agent_bytes_used.saturating_add(byte_cost) > self.max_agent_bytes {
                if !self.evict_lru_one() {
                    break; // nothing left to evict
                }
            }
        }

        self.entries.insert(
            key.clone(),
            FontCacheEntry {
                origin: FontOrigin::Agent,
                byte_cost,
            },
        );
        self.lru.push_back(key.clone());
        self.agent_bytes_used += byte_cost;

        CachedFontHandle(key)
    }

    /// Look up a font by key, promoting it in the LRU if it is an agent font.
    ///
    /// Returns `None` if the font is not cached.
    pub fn get(&mut self, key: &FontCacheKey) -> Option<CachedFontHandle> {
        if !self.entries.contains_key(key) {
            return None;
        }
        let entry = self.entries.get(key).unwrap();
        if entry.origin == FontOrigin::Agent {
            self.lru_promote(key);
        }
        Some(CachedFontHandle(key.clone()))
    }

    /// Check whether the cache contains the key (without promoting LRU).
    pub fn contains(&self, key: &FontCacheKey) -> bool {
        self.entries.contains_key(key)
    }

    /// Resolve a font for a `TextMarkdownNode`:
    ///
    /// 1. Try `resource_id` lookup.
    /// 2. On miss, attempt to return the `SystemSansSerif` bundled handle
    ///    (fallback is transparent — no error to the agent, spec lines 265–266).
    /// 3. If the `SystemSansSerif` fallback is not present in the cache,
    ///    returns `None`.  Callers must ensure the fallback is pre-inserted via
    ///    `insert_permanent` at initialization time.
    pub fn resolve_agent_font_or_fallback(
        &mut self,
        resource_id: ResourceId,
    ) -> Option<CachedFontHandle> {
        let agent_key = FontCacheKey::Resource(resource_id);
        if self.entries.contains_key(&agent_key) {
            let entry = self.entries.get(&agent_key).unwrap();
            if entry.origin == FontOrigin::Agent {
                self.lru_promote(&agent_key);
            }
            return Some(CachedFontHandle(agent_key));
        }

        // Fallback to SystemSansSerif (spec lines 265–266). Only return a
        // handle if the fallback is actually present in the cache.
        let fallback = FontCacheKey::Named("SystemSansSerif".to_owned());
        if self.entries.contains_key(&fallback) {
            Some(CachedFontHandle(fallback))
        } else {
            None
        }
    }

    /// Total byte cost of agent-uploaded fonts in the cache.
    pub fn agent_bytes_used(&self) -> usize {
        self.agent_bytes_used
    }

    /// Total number of entries (system + bundled + agent).
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// `true` if no fonts are cached.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Number of permanent (system/bundled) entries.
    pub fn permanent_count(&self) -> usize {
        self.entries
            .values()
            .filter(|e| e.origin.is_permanent())
            .count()
    }

    /// Number of agent-uploaded entries.
    pub fn agent_count(&self) -> usize {
        self.lru.len()
    }

    // ── Private helpers ───────────────────────────────────────────────────────

    /// Promote `key` to the back of the LRU queue (most-recently-used).
    fn lru_promote(&mut self, key: &FontCacheKey) {
        if let Some(pos) = self.lru.iter().position(|k| k == key) {
            self.lru.remove(pos);
            self.lru.push_back(key.clone());
        }
    }

    /// Evict the least-recently-used agent font.  Returns `true` if something
    /// was evicted.
    fn evict_lru_one(&mut self) -> bool {
        while let Some(lru_key) = self.lru.pop_front() {
            if let Some(entry) = self.entries.remove(&lru_key) {
                debug_assert_eq!(
                    entry.origin,
                    FontOrigin::Agent,
                    "LRU queue must never contain permanent fonts"
                );
                self.agent_bytes_used = self.agent_bytes_used.saturating_sub(entry.byte_cost);
                tracing::debug!(
                    font_key = ?lru_key,
                    byte_cost = entry.byte_cost,
                    "font cache: LRU eviction"
                );
                return true;
            }
        }
        false
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ResourceId;

    fn rid(n: u8) -> ResourceId {
        ResourceId::from_content(&[n])
    }

    // Acceptance: system font never GC'd; still available with no scene references [spec line 261-262].
    #[test]
    fn system_font_never_evicted() {
        let mut cache = FontCache::new(0); // 0 = unlimited
        let key = FontCacheKey::Named("SystemSansSerif".to_owned());
        cache.insert_permanent(
            key.clone(),
            FontCacheEntry {
                origin: FontOrigin::System,
                byte_cost: 512 * 1024,
            },
        );

        // Insert many agent fonts to stress the cache.
        for i in 0u8..10 {
            cache.insert_agent(rid(i), 1024 * 1024);
        }

        // System font must still be present.
        assert!(cache.contains(&key), "system font must never be evicted");
    }

    // Acceptance: bundled font never evicted.
    #[test]
    fn bundled_font_never_evicted() {
        let limit = 4 * 1024 * 1024; // 4 MiB limit
        let mut cache = FontCache::new(limit);

        let bundled_key = FontCacheKey::Named("BundledSans".to_owned());
        cache.insert_permanent(
            bundled_key.clone(),
            FontCacheEntry {
                origin: FontOrigin::Bundled,
                byte_cost: 2 * 1024 * 1024,
            },
        );

        // Insert 8 agent fonts of 1 MiB each → all but the newest should be evicted.
        for i in 0u8..8 {
            cache.insert_agent(rid(i), 1024 * 1024);
        }

        // Bundled font must still be present.
        assert!(
            cache.contains(&bundled_key),
            "bundled font must never be evicted"
        );
    }

    // Acceptance: LRU eviction evicts agent fonts, not system/bundled [spec line 276-277].
    #[test]
    fn lru_evicts_agent_fonts_first() {
        let limit = 3 * 1024 * 1024; // 3 MiB limit for agent fonts
        let mut cache = FontCache::new(limit);

        // Insert 3 agent fonts (exactly filling the limit).
        for i in 0u8..3 {
            cache.insert_agent(rid(i), 1024 * 1024);
        }
        assert_eq!(cache.agent_bytes_used(), 3 * 1024 * 1024);

        // Insert a 4th agent font — should evict the LRU (rid(0)).
        cache.insert_agent(rid(3), 1024 * 1024);

        let lru_key = FontCacheKey::Resource(rid(0));
        assert!(
            !cache.contains(&lru_key),
            "LRU agent font should have been evicted"
        );
        let new_key = FontCacheKey::Resource(rid(3));
        assert!(cache.contains(&new_key), "new font should be cached");
        assert!(cache.agent_bytes_used() <= limit + 1024 * 1024);
    }

    // Acceptance: font fallback on missing custom font resource [spec line 265-266].
    #[test]
    fn font_fallback_on_missing_resource() {
        let mut cache = FontCache::new(64 * 1024 * 1024);

        // Pre-populate SystemSansSerif (bundled).
        let fallback_key = FontCacheKey::Named("SystemSansSerif".to_owned());
        cache.insert_permanent(
            fallback_key.clone(),
            FontCacheEntry {
                origin: FontOrigin::Bundled,
                byte_cost: 512 * 1024,
            },
        );

        // Request a custom font that is NOT in the cache.
        let missing = rid(0xFF);
        let handle = cache
            .resolve_agent_font_or_fallback(missing)
            .expect("fallback must be present when SystemSansSerif is pre-inserted");

        // Must silently fall back to SystemSansSerif (no error).
        assert_eq!(
            handle.key(),
            &fallback_key,
            "must fall back to SystemSansSerif"
        );
    }

    // Acceptance: font cache exceeds limit → LRU agent eviction first.
    #[test]
    fn cache_exceeds_limit_evicts_lru() {
        let limit = 2 * 1024 * 1024; // 2 MiB
        let mut cache = FontCache::new(limit);

        cache.insert_agent(rid(0x01), 1 * 1024 * 1024);
        cache.insert_agent(rid(0x02), 1 * 1024 * 1024);
        // Access rid(0x01) to make it more recent than rid(0x02) would be.
        cache.get(&FontCacheKey::Resource(rid(0x01)));

        // Insert a 3rd font: should evict rid(0x02) (least recently used).
        cache.insert_agent(rid(0x03), 1 * 1024 * 1024);

        assert!(
            !cache.contains(&FontCacheKey::Resource(rid(0x02))),
            "LRU font should be evicted"
        );
        assert!(
            cache.contains(&FontCacheKey::Resource(rid(0x01))),
            "MRU font should remain"
        );
        assert!(
            cache.contains(&FontCacheKey::Resource(rid(0x03))),
            "new font should be present"
        );
    }

    // Acceptance: inserting same agent font twice promotes it in LRU.
    #[test]
    fn duplicate_insert_promotes_lru() {
        let limit = 2 * 1024 * 1024;
        let mut cache = FontCache::new(limit);

        cache.insert_agent(rid(0xA), 1024 * 1024);
        cache.insert_agent(rid(0xB), 512 * 1024);

        // Re-insert A (already cached) — should promote A to MRU.
        cache.insert_agent(rid(0xA), 1024 * 1024);

        // Insert C (1 MiB): B should be evicted (LRU), not A.
        cache.insert_agent(rid(0xC), 1024 * 1024);

        assert!(
            cache.contains(&FontCacheKey::Resource(rid(0xA))),
            "A promoted — must stay"
        );
        assert!(
            !cache.contains(&FontCacheKey::Resource(rid(0xB))),
            "B LRU — must be evicted"
        );
        assert!(cache.contains(&FontCacheKey::Resource(rid(0xC))));
    }
}
