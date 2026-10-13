//! `pce` is a PCE-friendly client library for the Illumio Core
//! Policy Compute Engine REST API v2.
//!
//! Layers, bottom up:
//! - [`client::PceClient`]: HTTP with Basic auth, a token-bucket throttle,
//!   request counting and backoff. The only place network I/O happens.
//! - [`cache::TtlCache`]: in-memory cache so the same list is never fetched
//!   twice within its TTL.
//! - [`Pce`]: the facade the TUI and CLI use. Typed reads, cached, with
//!   truncation reporting.
//! - `models`, `resolve`, `traffic`: data types, href-to-name resolution, and
//!   the Explorer traffic query (builder, poller, aggregation).
pub mod auth;
pub mod cache;
pub mod client;
pub mod error;
pub mod impact;
pub mod iplist_size;
pub mod key_order;
pub mod limiter;
pub mod models;
pub mod resolve;
pub mod traffic;
pub mod write;

use std::sync::Arc;
use std::time::Duration;

pub use impact::{
    Impact, ImpactInputs, IpListRef, LabelUsage, RuleActors, ScopeImpact, Side, label_usage,
    rule_impact, rule_impact_with, workload_labels,
};
pub use iplist_size::{IpListSize, ip_list_size};
pub use key_order::KeyOrder;
pub use resolve::{
    NameIndex, ResolvedActor, ResolvedLabel, ResolvedService, RuleKind, RuleSetView, RuleView,
    describe_actor, describe_ingress_service,
};

pub use auth::Credentials;
pub use cache::TtlCache;
pub use client::{ApiResponse, ClientConfig, PceClient, StatsSnapshot, WriteMethod};
pub use error::{PceError, Result};
pub use limiter::RateLimiter;

/// Whether a read may be served from cache.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheMode {
    Use,
    Bypass,
}

/// A value plus where it came from.
#[derive(Debug, Clone)]
pub struct Cached<T> {
    pub value: T,
    pub age: Duration,
    pub hit: bool,
    /// The PCE holds more rows than were returned.
    pub truncated: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct CacheTtls {
    pub objects: Duration,
    pub policy: Duration,
    pub workloads: Duration,
    pub events: Duration,
}

impl Default for CacheTtls {
    fn default() -> Self {
        Self {
            objects: Duration::from_secs(15 * 60),
            policy: Duration::from_secs(5 * 60),
            workloads: Duration::from_secs(5 * 60),
            events: Duration::from_secs(60),
        }
    }
}

pub const DEFAULT_MAX_RESULTS: u32 = 10_000;

/// The facade: typed, cached reads on top of one [`PceClient`].
pub struct Pce {
    client: PceClient,
    cache: TtlCache,
    ttls: CacheTtls,
    poll: traffic::PollConfig,
    key_order: KeyOrder,
}

impl Pce {
    pub fn new(client: PceClient) -> Self {
        Self::with_ttls(client, CacheTtls::default())
    }

    pub fn with_ttls(client: PceClient, ttls: CacheTtls) -> Self {
        Self {
            client,
            cache: TtlCache::new(),
            ttls,
            poll: traffic::PollConfig::default(),
            key_order: KeyOrder::default(),
        }
    }

    /// The order label keys are shown in; the name index is built with it.
    pub fn with_key_order(mut self, order: KeyOrder) -> Self {
        self.key_order = order;
        self
    }

    pub fn set_poll_config(&mut self, poll: traffic::PollConfig) {
        self.poll = poll;
    }

    pub fn client(&self) -> &PceClient {
        &self.client
    }

    pub fn ttls(&self) -> CacheTtls {
        self.ttls
    }

    pub fn stats(&self) -> StatsSnapshot {
        self.client.stats()
    }

    pub fn clear_cache(&self) {
        self.cache.clear();
    }

    pub fn invalidate(&self, prefix: &str) {
        self.cache.invalidate_prefix(prefix);
    }

    /// `GET /api/v2/health`. Never cached: it is the liveness probe.
    pub async fn health(&self) -> Result<serde_json::Value> {
        Ok(self
            .client
            .get::<serde_json::Value>("/health", &[])
            .await?
            .body)
    }

    pub async fn labels(&self, mode: CacheMode) -> Result<Cached<Vec<models::Label>>> {
        let path = self.client.org_path("labels");
        self.cached_list("objects:labels", &path, vec![], self.ttls.objects, mode)
            .await
    }

    pub async fn label_groups(&self, mode: CacheMode) -> Result<Cached<Vec<models::LabelGroup>>> {
        let path = self.client.org_path("sec_policy/draft/label_groups");
        self.cached_list(
            "objects:label_groups",
            &path,
            vec![],
            self.ttls.objects,
            mode,
        )
        .await
    }

    pub async fn ip_lists(&self, mode: CacheMode) -> Result<Cached<Vec<models::IpList>>> {
        let path = self.client.org_path("sec_policy/draft/ip_lists");
        self.cached_list("objects:ip_lists", &path, vec![], self.ttls.objects, mode)
            .await
    }

    pub async fn services(&self, mode: CacheMode) -> Result<Cached<Vec<models::Service>>> {
        let path = self.client.org_path("sec_policy/draft/services");
        self.cached_list("objects:services", &path, vec![], self.ttls.objects, mode)
            .await
    }

    /// Provisioned IP lists, for the TUI's pending diff. Cached under
    /// `objects:` so a write invalidates them with the draft lists; kept five
    /// minutes like the rest of the active policy.
    pub async fn active_ip_lists(&self, mode: CacheMode) -> Result<Cached<Vec<models::IpList>>> {
        let path = self.client.org_path("sec_policy/active/ip_lists");
        self.cached_list(
            "objects:active_ip_lists",
            &path,
            vec![],
            self.ttls.policy,
            mode,
        )
        .await
    }

    /// Provisioned services, for the TUI's pending diff (see `active_ip_lists`).
    pub async fn active_services(&self, mode: CacheMode) -> Result<Cached<Vec<models::Service>>> {
        let path = self.client.org_path("sec_policy/active/services");
        self.cached_list(
            "objects:active_services",
            &path,
            vec![],
            self.ttls.policy,
            mode,
        )
        .await
    }

    /// Provisioned label groups, for the pending diff (see `active_ip_lists`).
    pub async fn active_label_groups(
        &self,
        mode: CacheMode,
    ) -> Result<Cached<Vec<models::LabelGroup>>> {
        let path = self.client.org_path("sec_policy/active/label_groups");
        self.cached_list(
            "objects:active_label_groups",
            &path,
            vec![],
            self.ttls.policy,
            mode,
        )
        .await
    }

    /// Loads (or reuses) the four object lists and builds the name index.
    pub async fn name_index(&self, mode: CacheMode) -> Result<Arc<NameIndex>> {
        if mode == CacheMode::Use
            && let Some((idx, _)) = self.cache.get::<Arc<NameIndex>>("objects:index")
        {
            return Ok(idx);
        }
        let labels = self.labels(mode).await?.value;
        let groups = self.label_groups(mode).await?.value;
        let lists = self.ip_lists(mode).await?.value;
        let services = self.services(mode).await?.value;
        let idx = Arc::new(
            NameIndex::new(&labels, &groups, &lists, &services)
                .with_key_order(self.key_order.clone()),
        );
        self.cache
            .insert("objects:index", idx.clone(), self.ttls.objects);
        Ok(idx)
    }

    pub async fn workloads(
        &self,
        filter: &models::WorkloadFilter,
        mode: CacheMode,
    ) -> Result<Cached<Vec<models::Workload>>> {
        let path = self.client.org_path("workloads");
        self.cached_list(
            "workloads",
            &path,
            filter.to_query(),
            self.ttls.workloads,
            mode,
        )
        .await
    }

    pub const DEFAULT_EVENTS_MAX_RESULTS: u32 = 500;

    pub async fn events(
        &self,
        filter: &models::EventFilter,
        mode: CacheMode,
    ) -> Result<Cached<Vec<models::Event>>> {
        let path = self.client.org_path("events");
        let mut query = filter.to_query();
        // `cached_list` only appends its 10 000 default when `max_results`
        // is absent, so the events default must be pushed first.
        if !query.iter().any(|(k, _)| *k == "max_results") {
            query.push(("max_results", Self::DEFAULT_EVENTS_MAX_RESULTS.to_string()));
        }
        self.cached_list("events", &path, query, self.ttls.events, mode)
            .await
    }

    pub async fn rule_sets(
        &self,
        scope: models::PolicyScope,
        mode: CacheMode,
    ) -> Result<Cached<Vec<models::RuleSet>>> {
        let path = self
            .client
            .org_path(&format!("sec_policy/{}/rule_sets", scope.segment()));
        let key = format!("policy:{}:rule_sets", scope.segment());
        self.cached_list(&key, &path, vec![], self.ttls.policy, mode)
            .await
    }

    /// Deny rules live on a per-ruleset endpoint that the OpenAPI spec omits.
    pub async fn deny_rules(
        &self,
        ruleset_href: &str,
        mode: CacheMode,
    ) -> Result<Cached<Vec<models::DenyRule>>> {
        let path = format!("{}/deny_rules", ruleset_href.trim_end_matches('/'));
        let key = format!("policy:deny:{ruleset_href}");
        self.cached_list(&key, &path, vec![], self.ttls.policy, mode)
            .await
    }

    pub async fn pending_changes(
        &self,
        mode: CacheMode,
    ) -> Result<Cached<Vec<models::PendingChange>>> {
        let key = "policy:pending";
        if mode == CacheMode::Use
            && let Some((value, age)) = self.cache.get::<Vec<models::PendingChange>>(key)
        {
            return Ok(Cached {
                value,
                age,
                hit: true,
                truncated: false,
            });
        }
        let path = self.client.org_path("sec_policy/pending");
        let raw = self.client.get::<serde_json::Value>(&path, &[]).await?.body;
        let value = models::parse_pending(raw);
        self.cache.insert(key, value.clone(), self.ttls.policy);
        Ok(Cached {
            value,
            age: Duration::ZERO,
            hit: false,
            truncated: false,
        })
    }

    /// Runs (or reuses) one Explorer query in the `main` slot: the
    /// unfocused traffic result (see `traffic_slot`).
    pub async fn traffic(
        &self,
        query: &models::TrafficQuery,
        mode: CacheMode,
        cancel: &tokio_util::sync::CancellationToken,
        progress: Option<&tokio::sync::mpsc::UnboundedSender<traffic::QueryProgress>>,
    ) -> Result<Cached<traffic::TrafficResult>> {
        self.traffic_slot("main", query, mode, cancel, progress)
            .await
    }

    /// Runs (or reuses) one Explorer query in cache slot `slot`. The cache
    /// key is the slot plus the full query body, so the same filter never
    /// runs twice until invalidated. Each slot keeps exactly one result set
    /// (a long TTL stands in for "until the filter changes or the user
    /// refreshes"): the unfocused result (`main`), the focused ones
    /// (`focus:in`, `focus:out`) and a timeline's buckets do not evict
    /// each other.
    pub async fn traffic_slot(
        &self,
        slot: &str,
        query: &models::TrafficQuery,
        mode: CacheMode,
        cancel: &tokio_util::sync::CancellationToken,
        progress: Option<&tokio::sync::mpsc::UnboundedSender<traffic::QueryProgress>>,
    ) -> Result<Cached<traffic::TrafficResult>> {
        let prefix = format!("traffic:{slot}:");
        let key = format!(
            "{prefix}{}",
            serde_json::to_string(query).map_err(|e| PceError::Invalid(e.to_string()))?
        );
        if mode == CacheMode::Use
            && let Some((value, age)) = self.cache.get::<traffic::TrafficResult>(&key)
        {
            let truncated = value.truncated;
            return Ok(Cached {
                value,
                age,
                hit: true,
                truncated,
            });
        }
        let result =
            traffic::run_traffic_query(&self.client, query, &self.poll, cancel, progress).await?;
        self.cache.invalidate_prefix(&prefix);
        self.cache
            .insert(&key, result.clone(), Duration::from_secs(24 * 3600));
        let truncated = result.truncated;
        Ok(Cached {
            value: result,
            age: Duration::ZERO,
            hit: false,
            truncated,
        })
    }

    /// `focus`'s queries over `base` (`Focus::queries`: one per side, `in`
    /// first), each run (or reused) in its own slot `{slot}:in` /
    /// `{slot}:out`, one at a time, resolved with `idx`, merged without
    /// duplicates (`union_flows`) and narrowed by `Focus::keeps`. An app the
    /// labels do not name is `PceError::Invalid` before any request.
    #[allow(clippy::too_many_arguments)]
    pub async fn focused_traffic(
        &self,
        slot: &str,
        focus: &traffic::Focus,
        base: &models::TrafficQuery,
        idx: &NameIndex,
        mode: CacheMode,
        cancel: &tokio_util::sync::CancellationToken,
        progress: Option<&tokio::sync::mpsc::UnboundedSender<traffic::QueryProgress>>,
    ) -> Result<traffic::FocusedTraffic> {
        let queries = focus.queries(base, idx).map_err(PceError::Invalid)?;
        let mut merged: Option<Vec<traffic::ResolvedFlow>> = None;
        let mut out = traffic::FocusedTraffic {
            flows: Vec::new(),
            truncated: false,
            matches_count: Some(0),
            elapsed: Duration::ZERO,
            hit: true,
            queries: Vec::new(),
            coverage: None,
        };
        let window = base.window();
        for (side, q) in queries {
            let r = self
                .traffic_slot(
                    &format!("{slot}:{}", side.as_str()),
                    &q,
                    mode,
                    cancel,
                    progress,
                )
                .await?;
            let flows = traffic::resolve_flows(&r.value.flows, idx);
            out.coverage = traffic::truncated_coverage(out.coverage, r.truncated, &flows, window);
            merged = Some(match merged.take() {
                None => flows,
                Some(first) => traffic::union_flows(first, flows),
            });
            out.truncated |= r.truncated;
            out.matches_count = out
                .matches_count
                .zip(r.value.matches_count)
                .map(|(a, b)| a + b);
            out.elapsed += r.value.elapsed;
            out.hit &= r.hit;
            out.queries.push(q);
        }
        out.flows = merged
            .unwrap_or_default()
            .into_iter()
            .filter(|f| focus.keeps(f))
            .collect();
        Ok(out)
    }

    /// Resolves filter expressions (OR of AND-groups) to actor matches,
    /// loading workloads only when a `wl:` expression needs them.
    pub async fn resolve_filter_groups(
        &self,
        groups: &[Vec<traffic::FilterExpr>],
        mode: CacheMode,
    ) -> Result<Vec<Vec<models::ActorMatch>>> {
        let idx = self.name_index(mode).await?;
        let workloads = if groups
            .iter()
            .flatten()
            .any(traffic::FilterExpr::needs_workloads)
        {
            self.workloads(&models::WorkloadFilter::default(), mode)
                .await?
                .value
        } else {
            Vec::new()
        };
        groups
            .iter()
            .map(|g| {
                g.iter()
                    .map(|e| traffic::resolve_actor(e, &idx, &workloads))
                    .collect()
            })
            .collect()
    }

    /// Fetches a JSON array with an explicit `max_results`, caching under
    /// `key` (plus the query string). `truncated` is set when the PCE reports
    /// more rows than it returned.
    pub(crate) async fn cached_list<T>(
        &self,
        key: &str,
        api_path: &str,
        mut query: Vec<(&str, String)>,
        ttl: Duration,
        mode: CacheMode,
    ) -> Result<Cached<Vec<T>>>
    where
        T: serde::de::DeserializeOwned + Clone + Send + Sync + 'static,
    {
        if !query.iter().any(|(k, _)| *k == "max_results") {
            query.push(("max_results", DEFAULT_MAX_RESULTS.to_string()));
        }
        let qs: Vec<String> = query.iter().map(|(k, v)| format!("{k}={v}")).collect();
        let cache_key = format!("{key}?{}", qs.join("&"));
        if mode == CacheMode::Use
            && let Some(((value, truncated), age)) = self.cache.get::<(Vec<T>, bool)>(&cache_key)
        {
            return Ok(Cached {
                value,
                age,
                hit: true,
                truncated,
            });
        }
        let resp = self.client.get::<Vec<T>>(api_path, &query).await?;
        let len = resp.body.len() as u64;
        let max: u64 = query
            .iter()
            .find(|(k, _)| *k == "max_results")
            .and_then(|(_, v)| v.parse().ok())
            .unwrap_or(u64::from(DEFAULT_MAX_RESULTS));
        let truncated = resp.total_count.is_some_and(|t| t > len) || len >= max;
        self.cache
            .insert(&cache_key, (resp.body.clone(), truncated), ttl);
        Ok(Cached {
            value: resp.body,
            age: Duration::ZERO,
            hit: false,
            truncated,
        })
    }
}
