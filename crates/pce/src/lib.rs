//! `pce` is a read-only, PCE-friendly client library for the Illumio Core
//! Policy Compute Engine REST API v2.
//!
//! Everything that talks to a PCE goes through [`client::PceClient`], which
//! throttles, counts and retries requests. Higher-level typed reads with
//! caching live on the [`Pce`] facade (added in Task 5).
