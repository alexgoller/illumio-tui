//! Writes to the PCE: guarded, audited, sent once. See the Plan 3 spec.
pub mod audit;

pub use audit::{AuditEntry, AuditSink, JsonlAudit, MemoryAudit, NullAudit};
