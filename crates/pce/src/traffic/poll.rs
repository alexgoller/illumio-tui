//! Runs one Explorer query: POST, poll with backoff until completed, GET the
//! download. Cancellable between polls and during the status and download
//! requests. Reports progress over a channel so a UI can show elapsed time
//! without owning the future.
use crate::client::PceClient;
use crate::error::{PceError, Result};
use crate::models::{AsyncQueryStatus, Flow, TrafficQuery};
use std::time::Duration;
use tokio::sync::mpsc::UnboundedSender;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone, Copy)]
pub struct PollConfig {
    pub initial: Duration,
    pub factor: f64,
    pub cap: Duration,
    /// How long to keep polling after `completed` arrives without a result.
    pub grace: Duration,
    /// The result download's own timeout: a 200000-row result is hundreds
    /// of MB, far past the client's per-request timeout.
    pub download_timeout: Duration,
}

impl Default for PollConfig {
    /// Polls at 1 s, then ×1.5 up to 3 s: a finished query waits at most
    /// 3 s for its next status GET.
    fn default() -> Self {
        Self {
            initial: Duration::from_secs(1),
            factor: 1.5,
            cap: Duration::from_secs(3),
            grace: Duration::from_secs(30),
            download_timeout: Duration::from_secs(300),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum QueryProgress {
    Submitted { href: String },
    Polling { status: String, elapsed: Duration },
    Downloading,
    Done { rows: usize },
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct TrafficResult {
    pub flows: Vec<Flow>,
    /// The completed status's `flows_count`: the rows after the cap.
    pub flows_count: Option<u64>,
    /// The cap bound the result (`truncation`).
    pub truncated: bool,
    /// A supercluster region did not respond: its rows are missing.
    pub incomplete: bool,
    pub query_href: String,
    pub elapsed: Duration,
}

/// What a completed query's status says about its result.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Truncation {
    /// The cap bound the result: more records may exist.
    pub truncated: bool,
    /// A region did not respond.
    pub incomplete: bool,
}

/// Reads the cap signal from a completed status. With `regions` (a
/// supercluster, each region capped on its own) any region whose
/// `flows_count` reaches `cap`; otherwise `flows_count >= cap`; without a
/// `flows_count`, `rows >= cap`. `matches_count` is a bounded raw count
/// (400 at a cap of 200, 4631 for a complete 4625), so it never decides.
pub fn truncation(status: &AsyncQueryStatus, rows: usize, cap: u32) -> Truncation {
    let cap = u64::from(cap);
    let regions = status.regions.as_deref().unwrap_or_default();
    let counted = !regions.is_empty() && regions.iter().all(|r| r.flows_count.is_some());
    let truncated = if counted {
        regions
            .iter()
            .any(|r| r.flows_count.is_some_and(|n| n >= cap))
    } else {
        status.flows_count.unwrap_or(rows as u64) >= cap
    };
    Truncation {
        truncated,
        incomplete: regions.iter().any(|r| r.responded == Some(false)),
    }
}

fn report(progress: Option<&UnboundedSender<QueryProgress>>, p: QueryProgress) {
    if let Some(tx) = progress {
        let _ = tx.send(p);
    }
}

pub async fn run_traffic_query(
    client: &PceClient,
    query: &TrafficQuery,
    poll: &PollConfig,
    cancel: &CancellationToken,
    progress: Option<&UnboundedSender<QueryProgress>>,
) -> Result<TrafficResult> {
    let started = Instant::now();
    let submit_path = client.org_path("traffic_flows/async_queries");
    let submitted: AsyncQueryStatus = client
        .post(&submit_path, query, &[("Prefer", "respond-async")])
        .await?
        .body;
    let query_href = submitted
        .href
        .clone()
        .ok_or_else(|| PceError::Decode("async query response has no href".into()))?;
    report(
        progress,
        QueryProgress::Submitted {
            href: query_href.clone(),
        },
    );

    let mut delay = poll.initial;
    let mut completed_at: Option<Instant> = None;
    let (result_href, done) = loop {
        tokio::select! {
            () = cancel.cancelled() => return Err(PceError::Cancelled),
            () = tokio::time::sleep(delay) => {}
        }
        let status: AsyncQueryStatus = tokio::select! {
            () = cancel.cancelled() => return Err(PceError::Cancelled),
            r = client.get(&query_href, &[]) => r?.body,
        };
        let state = status.status.clone().unwrap_or_else(|| "unknown".into());
        report(
            progress,
            QueryProgress::Polling {
                status: state.clone(),
                elapsed: started.elapsed(),
            },
        );
        match state.as_str() {
            "failed" => {
                return Err(PceError::QueryFailed(
                    status.message.unwrap_or_else(|| "no message".into()),
                ));
            }
            "completed" | "done" => match &status.result {
                Some(r) => break (r.href().to_string(), status),
                None => {
                    // Known PCE race: completed before the result href exists.
                    let since = *completed_at.get_or_insert_with(Instant::now);
                    if since.elapsed() > poll.grace {
                        return Err(PceError::Decode(format!(
                            "query {query_href} completed but returned no result within {:?}",
                            poll.grace
                        )));
                    }
                }
            },
            _ => {}
        }
        delay = Duration::from_secs_f64(
            (delay.as_secs_f64() * poll.factor).min(poll.cap.as_secs_f64()),
        );
    };

    report(progress, QueryProgress::Downloading);
    let flows: Vec<Flow> = tokio::select! {
        () = cancel.cancelled() => return Err(PceError::Cancelled),
        r = client.get_with_timeout(&result_href, &[], poll.download_timeout) => r?.body,
    };
    let rows = flows.len();
    let t = truncation(&done, rows, query.max_results);
    report(progress, QueryProgress::Done { rows });
    Ok(TrafficResult {
        flows,
        flows_count: done.flows_count,
        truncated: t.truncated,
        incomplete: t.incomplete,
        query_href,
        elapsed: started.elapsed(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::QueryRegion;

    fn status(flows: Option<u64>, matches: Option<u64>) -> AsyncQueryStatus {
        AsyncQueryStatus {
            flows_count: flows,
            matches_count: matches,
            ..Default::default()
        }
    }

    fn region(flows: Option<u64>, responded: Option<bool>) -> QueryRegion {
        QueryRegion {
            flows_count: flows,
            responded,
            ..Default::default()
        }
    }

    const NO: Truncation = Truncation {
        truncated: false,
        incomplete: false,
    };
    const CUT: Truncation = Truncation {
        truncated: true,
        incomplete: false,
    };

    #[test]
    fn polls_cap_at_3s_and_the_download_gets_300s() {
        let p = PollConfig::default();
        assert_eq!(
            (p.initial, p.factor, p.cap),
            (Duration::from_secs(1), 1.5, Duration::from_secs(3))
        );
        assert_eq!(p.download_timeout, Duration::from_secs(300));
    }

    #[test]
    fn the_demo_statuses() {
        // demo100, one day at cap 10000: complete, though matches > rows.
        assert_eq!(
            truncation(&status(Some(4625), Some(4631)), 4625, 10_000),
            NO
        );
        // The same day at cap 200: flows_count reached the cap.
        assert_eq!(truncation(&status(Some(200), Some(400)), 200, 200), CUT);
    }

    #[test]
    fn flows_count_decides_and_rows_only_stand_in_for_it() {
        assert_eq!(truncation(&status(Some(999), None), 1000, 1000), NO);
        assert_eq!(truncation(&status(Some(1000), None), 999, 1000), CUT);
        assert_eq!(truncation(&status(None, Some(5000)), 999, 1000), NO);
        assert_eq!(truncation(&status(None, None), 1000, 1000), CUT);
    }

    #[test]
    fn a_supercluster_is_capped_per_region() {
        // 16815 rows at a cap of 10000: two regions, neither at the cap.
        let mut s = status(Some(16_815), Some(16_900));
        s.regions = Some(vec![
            region(Some(9_000), Some(true)),
            region(Some(7_815), Some(true)),
        ]);
        assert_eq!(truncation(&s, 16_815, 10_000), NO);
        s.regions.as_mut().unwrap()[1].flows_count = Some(10_000);
        assert_eq!(truncation(&s, 19_000, 10_000), CUT);
        // A region without a count: the overall count decides.
        s.regions.as_mut().unwrap()[1].flows_count = None;
        assert_eq!(truncation(&s, 16_815, 10_000), CUT);
    }

    #[test]
    fn a_region_that_did_not_respond_is_incomplete_not_truncated() {
        let mut s = status(Some(10), None);
        s.regions = Some(vec![
            region(Some(10), Some(true)),
            region(Some(0), Some(false)),
        ]);
        assert_eq!(
            truncation(&s, 10, 1000),
            Truncation {
                truncated: false,
                incomplete: true
            }
        );
        s.regions = Some(Vec::new());
        assert_eq!(truncation(&s, 10, 1000), NO, "an empty list is no regions");
    }
}
