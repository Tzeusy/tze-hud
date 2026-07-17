//! Runtime-resident allocation accounting shared by cache owners.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Disjoint resident-memory accounting classes. Capacity never borrows across classes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ResidentClass {
    Resource,
    WidgetSource,
    WidgetRaster,
    Font,
}

/// Stable identity for one physical resident allocation.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct AllocationId(pub String);

impl From<String> for AllocationId {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl From<&str> for AllocationId {
    fn from(value: &str) -> Self {
        Self(value.to_string())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResidentLedgerLimits {
    pub aggregate_bytes: u64,
    pub resource_bytes: u64,
    pub widget_source_bytes: u64,
    pub widget_raster_bytes: u64,
    pub font_bytes: u64,
}

impl ResidentLedgerLimits {
    fn class_limit(self, class: ResidentClass) -> u64 {
        match class {
            ResidentClass::Resource => self.resource_bytes,
            ResidentClass::WidgetSource => self.widget_source_bytes,
            ResidentClass::WidgetRaster => self.widget_raster_bytes,
            ResidentClass::Font => self.font_bytes,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ResidentLedgerSnapshot {
    pub aggregate_bytes: u64,
    pub resource_bytes: u64,
    pub widget_source_bytes: u64,
    pub widget_raster_bytes: u64,
    pub font_bytes: u64,
    pub allocation_count: usize,
    pub class_denial_count: u64,
    pub aggregate_denial_count: u64,
    pub eviction_count: u64,
}

impl ResidentLedgerSnapshot {
    fn class_bytes(self, class: ResidentClass) -> u64 {
        match class {
            ResidentClass::Resource => self.resource_bytes,
            ResidentClass::WidgetSource => self.widget_source_bytes,
            ResidentClass::WidgetRaster => self.widget_raster_bytes,
            ResidentClass::Font => self.font_bytes,
        }
    }

    fn add(&mut self, class: ResidentClass, bytes: u64) {
        self.aggregate_bytes = self.aggregate_bytes.saturating_add(bytes);
        match class {
            ResidentClass::Resource => self.resource_bytes += bytes,
            ResidentClass::WidgetSource => self.widget_source_bytes += bytes,
            ResidentClass::WidgetRaster => self.widget_raster_bytes += bytes,
            ResidentClass::Font => self.font_bytes += bytes,
        }
    }

    fn subtract(&mut self, class: ResidentClass, bytes: u64) {
        self.aggregate_bytes = self.aggregate_bytes.saturating_sub(bytes);
        match class {
            ResidentClass::Resource => {
                self.resource_bytes = self.resource_bytes.saturating_sub(bytes)
            }
            ResidentClass::WidgetSource => {
                self.widget_source_bytes = self.widget_source_bytes.saturating_sub(bytes)
            }
            ResidentClass::WidgetRaster => {
                self.widget_raster_bytes = self.widget_raster_bytes.saturating_sub(bytes)
            }
            ResidentClass::Font => self.font_bytes = self.font_bytes.saturating_sub(bytes),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ResidentReserveError {
    ClassLimit {
        class: ResidentClass,
        current: u64,
        requested: u64,
        limit: u64,
    },
    AggregateLimit {
        current: u64,
        requested: u64,
        limit: u64,
    },
    IdentitySizeMismatch {
        class: ResidentClass,
        id: AllocationId,
        existing: u64,
        requested: u64,
    },
    IdentityClassMismatch {
        id: AllocationId,
        existing: ResidentClass,
        requested: ResidentClass,
    },
}

#[derive(Debug)]
struct LedgerState {
    allocations: HashMap<AllocationId, (ResidentClass, u64)>,
    snapshot: ResidentLedgerSnapshot,
}

/// Thread-safe, atomic resident allocation ledger.
#[derive(Clone, Debug)]
pub struct ResidentLedger {
    limits: ResidentLedgerLimits,
    state: Arc<Mutex<LedgerState>>,
}

impl ResidentLedger {
    pub fn new(limits: ResidentLedgerLimits) -> Self {
        Self {
            limits,
            state: Arc::new(Mutex::new(LedgerState {
                allocations: HashMap::new(),
                snapshot: ResidentLedgerSnapshot::default(),
            })),
        }
    }

    pub fn limits(&self) -> ResidentLedgerLimits {
        self.limits
    }

    /// Reserve one physical allocation. An identical identity/size is idempotent.
    pub fn reserve(
        &self,
        class: ResidentClass,
        id: impl Into<AllocationId>,
        bytes: u64,
    ) -> Result<bool, ResidentReserveError> {
        let id = id.into();
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        if let Some((existing_class, existing_bytes)) = state.allocations.get(&id) {
            if *existing_class != class {
                return Err(ResidentReserveError::IdentityClassMismatch {
                    id,
                    existing: *existing_class,
                    requested: class,
                });
            }
            return if *existing_bytes == bytes {
                Ok(false)
            } else {
                Err(ResidentReserveError::IdentitySizeMismatch {
                    class,
                    id,
                    existing: *existing_bytes,
                    requested: bytes,
                })
            };
        }
        let class_current = state.snapshot.class_bytes(class);
        let class_limit = self.limits.class_limit(class);
        if class_current.saturating_add(bytes) > class_limit {
            state.snapshot.class_denial_count = state.snapshot.class_denial_count.saturating_add(1);
            return Err(ResidentReserveError::ClassLimit {
                class,
                current: class_current,
                requested: bytes,
                limit: class_limit,
            });
        }
        if state.snapshot.aggregate_bytes.saturating_add(bytes) > self.limits.aggregate_bytes {
            state.snapshot.aggregate_denial_count =
                state.snapshot.aggregate_denial_count.saturating_add(1);
            return Err(ResidentReserveError::AggregateLimit {
                current: state.snapshot.aggregate_bytes,
                requested: bytes,
                limit: self.limits.aggregate_bytes,
            });
        }
        state.allocations.insert(id, (class, bytes));
        state.snapshot.add(class, bytes);
        state.snapshot.allocation_count += 1;
        Ok(true)
    }

    pub fn release(&self, class: ResidentClass, id: &AllocationId) -> bool {
        self.release_inner(class, id, false)
    }

    /// Release an allocation at a cache-safe eviction boundary.
    pub fn release_evicted(&self, class: ResidentClass, id: &AllocationId) -> bool {
        self.release_inner(class, id, true)
    }

    fn release_inner(&self, class: ResidentClass, id: &AllocationId, evicted: bool) -> bool {
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        let Some((existing_class, bytes)) = state.allocations.get(id).copied() else {
            return false;
        };
        if existing_class != class {
            return false;
        }
        state.allocations.remove(id);
        state.snapshot.subtract(class, bytes);
        state.snapshot.allocation_count = state.snapshot.allocation_count.saturating_sub(1);
        if evicted {
            state.snapshot.eviction_count = state.snapshot.eviction_count.saturating_add(1);
        }
        true
    }

    pub fn snapshot(&self) -> ResidentLedgerSnapshot {
        self.state
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .snapshot
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ledger() -> ResidentLedger {
        ResidentLedger::new(ResidentLedgerLimits {
            aggregate_bytes: 100,
            resource_bytes: 50,
            widget_source_bytes: 20,
            widget_raster_bytes: 20,
            font_bytes: 10,
        })
    }

    #[test]
    fn classes_are_disjoint_and_cannot_borrow() {
        let ledger = ledger();
        ledger.reserve(ResidentClass::Resource, "r", 50).unwrap();
        assert!(matches!(
            ledger.reserve(ResidentClass::Resource, "r2", 1),
            Err(ResidentReserveError::ClassLimit { .. })
        ));
        assert!(ledger.reserve(ResidentClass::WidgetSource, "w", 20).is_ok());
    }

    #[test]
    fn physical_identity_is_single_charged_but_copies_are_separate() {
        let ledger = ledger();
        assert_eq!(
            ledger.reserve(ResidentClass::Resource, "hash:cpu", 20),
            Ok(true)
        );
        assert_eq!(
            ledger.reserve(ResidentClass::Resource, "hash:cpu", 20),
            Ok(false)
        );
        assert_eq!(
            ledger.reserve(ResidentClass::Resource, "hash:gpu", 20),
            Ok(true)
        );
        assert_eq!(ledger.snapshot().resource_bytes, 40);
    }

    #[test]
    fn allocation_identity_cannot_debit_two_classes() {
        let ledger = ledger();
        assert_eq!(
            ledger.reserve(ResidentClass::Resource, "shared-allocation", 10),
            Ok(true)
        );

        assert!(
            ledger
                .reserve(ResidentClass::Font, "shared-allocation", 10)
                .is_err(),
            "one physical allocation identity must belong to exactly one resident class"
        );
        assert_eq!(ledger.snapshot().aggregate_bytes, 10);
        assert_eq!(ledger.snapshot().allocation_count, 1);
    }

    #[test]
    fn release_returns_capacity_atomically() {
        let ledger = ledger();
        let id = AllocationId::from("font-a");
        ledger.reserve(ResidentClass::Font, id.clone(), 10).unwrap();
        assert!(ledger.release(ResidentClass::Font, &id));
        assert_eq!(ledger.snapshot(), ResidentLedgerSnapshot::default());
    }

    #[test]
    fn snapshot_counts_denials_and_only_safe_eviction_releases() {
        let ledger = ledger();
        let id = AllocationId::from("resource-a");
        ledger
            .reserve(ResidentClass::Resource, id.clone(), 50)
            .unwrap();
        assert!(matches!(
            ledger.reserve(ResidentClass::Resource, "resource-overflow", 1),
            Err(ResidentReserveError::ClassLimit { .. })
        ));

        assert!(ledger.release_evicted(ResidentClass::Resource, &id));
        let snapshot = ledger.snapshot();
        assert_eq!(snapshot.class_denial_count, 1);
        assert_eq!(snapshot.aggregate_denial_count, 0);
        assert_eq!(snapshot.eviction_count, 1);
        assert_eq!(snapshot.aggregate_bytes, 0);
    }

    #[test]
    fn replacement_reserves_before_old_frame_allocation_is_released() {
        let ledger = ledger();
        let old = AllocationId::from("widget:old-frame");
        ledger
            .reserve(ResidentClass::WidgetRaster, old.clone(), 15)
            .unwrap();
        assert!(matches!(
            ledger.reserve(ResidentClass::WidgetRaster, "widget:new-frame", 10),
            Err(ResidentReserveError::ClassLimit { .. })
        ));
        assert_eq!(ledger.snapshot().widget_raster_bytes, 15);
        assert!(ledger.release(ResidentClass::WidgetRaster, &old));
    }
}
