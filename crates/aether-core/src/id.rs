//! Typed identifiers.
//!
//! Every object that can be referenced from elsewhere in a project (layers,
//! assets, compositions, rig parameters) carries a small `Copy` identifier
//! rather than being referenced by pointer. This keeps the document tree
//! serialisable, makes undo/redo commands cheap to store, and lets the renderer
//! cache results per identifier.
//!
//! Identifiers are unique *within a project*. They are allocated by an
//! [`IdGenerator`] which is serialised with the project so that reopening a
//! file never reissues an id that is already in use.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};

macro_rules! typed_id {
    ($(#[$meta:meta])* $name:ident, $prefix:literal) => {
        $(#[$meta])*
        #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(pub u64);

        impl $name {
            /// The reserved "nothing here" identifier.
            pub const NONE: Self = Self(0);

            /// Wrap a raw value.
            #[inline]
            pub const fn from_raw(raw: u64) -> Self {
                Self(raw)
            }

            /// The underlying raw value.
            #[inline]
            pub const fn raw(self) -> u64 {
                self.0
            }

            /// True for the reserved [`Self::NONE`] value.
            #[inline]
            pub const fn is_none(self) -> bool {
                self.0 == 0
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, concat!($prefix, "{}"), self.0)
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, concat!($prefix, "{}"), self.0)
            }
        }
    };
}

typed_id!(
    /// Identifies a layer inside a document's layer tree.
    LayerId, "layer#");
typed_id!(
    /// Identifies a document inside a project.
    DocumentId, "doc#");
typed_id!(
    /// Identifies a composition (a timeline + layer stack that can be nested).
    CompositionId, "comp#");
typed_id!(
    /// Identifies an imported asset (image, font, palette, ...).
    AssetId, "asset#");
typed_id!(
    /// Identifies a rig parameter such as `AngleX` or `MouthOpen`.
    ParameterId, "param#");

/// Monotonic allocator for typed ids.
///
/// Cloning an [`IdGenerator`] shares the counter, so the document and any
/// background worker allocate from the same sequence.
#[derive(Debug, Serialize, Deserialize)]
#[serde(from = "u64", into = "u64")]
pub struct IdGenerator {
    next: AtomicU64,
}

impl IdGenerator {
    /// Start allocating from 1 (0 is reserved for `NONE`).
    pub fn new() -> Self {
        Self {
            next: AtomicU64::new(1),
        }
    }

    /// Resume allocation from a previously serialised watermark.
    pub fn resume_from(next: u64) -> Self {
        Self {
            next: AtomicU64::new(next.max(1)),
        }
    }

    /// The value that will be handed out next; serialise this to resume later.
    pub fn watermark(&self) -> u64 {
        self.next.load(Ordering::Relaxed)
    }

    /// Allocate a raw identifier.
    pub fn next_raw(&self) -> u64 {
        self.next.fetch_add(1, Ordering::Relaxed)
    }

    /// Make sure future allocations never collide with `raw`.
    pub fn reserve_at_least(&self, raw: u64) {
        self.next.fetch_max(raw + 1, Ordering::Relaxed);
    }
}

impl Default for IdGenerator {
    fn default() -> Self {
        Self::new()
    }
}

impl Clone for IdGenerator {
    fn clone(&self) -> Self {
        Self::resume_from(self.watermark())
    }
}

impl From<u64> for IdGenerator {
    fn from(v: u64) -> Self {
        Self::resume_from(v)
    }
}

impl From<IdGenerator> for u64 {
    fn from(v: IdGenerator) -> u64 {
        v.watermark()
    }
}

/// Allocation helpers so call sites read as `ids.layer()` rather than casting.
impl IdGenerator {
    /// Allocate a fresh [`LayerId`].
    pub fn layer(&self) -> LayerId {
        LayerId(self.next_raw())
    }
    /// Allocate a fresh [`DocumentId`].
    pub fn document(&self) -> DocumentId {
        DocumentId(self.next_raw())
    }
    /// Allocate a fresh [`CompositionId`].
    pub fn composition(&self) -> CompositionId {
        CompositionId(self.next_raw())
    }
    /// Allocate a fresh [`AssetId`].
    pub fn asset(&self) -> AssetId {
        AssetId(self.next_raw())
    }
    /// Allocate a fresh [`ParameterId`].
    pub fn parameter(&self) -> ParameterId {
        ParameterId(self.next_raw())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_unique_and_skip_zero() {
        let ids = IdGenerator::new();
        let a = ids.layer();
        let b = ids.layer();
        assert_ne!(a, b);
        assert!(!a.is_none());
        assert_eq!(a.raw(), 1);
    }

    #[test]
    fn resume_avoids_collisions() {
        let ids = IdGenerator::new();
        ids.reserve_at_least(41);
        assert_eq!(ids.layer().raw(), 42);
        let resumed = IdGenerator::resume_from(ids.watermark());
        assert_eq!(resumed.layer().raw(), 43);
    }

    #[test]
    fn display_is_prefixed() {
        assert_eq!(LayerId(7).to_string(), "layer#7");
        assert_eq!(AssetId(3).to_string(), "asset#3");
    }
}
