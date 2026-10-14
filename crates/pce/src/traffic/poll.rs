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
    /// The longest each polling stage (the query, the draft re-evaluation)
    /// waits before it gives up: a PCE that never finishes is not polled
    /// forever. Cancelling still stops it at once.
    pub max_wait: Duration,
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
            max_wait: Duration::from_secs(600),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum QueryProgress {
    Submitted {
        href: String,
    },
    Polling {
        status: String,
        elapsed: Duration,
    },
    /// The draft re-evaluation: `requested`, then the status's `rules`
    /// state (`pending` while it has none).
    Evaluating {
        rules: String,
        elapsed: Duration,
    },
    Downloading,
    Done {
        rows: usize,
    },
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
/// `flows_count` reaches `cap`, judged on the regions that report one;
/// only when none does, the overall `flows_count >= cap`, and without that
/// `rows >= cap`. A region that did not respond or reports no count makes
/// the result `incomplete`. `matches_count` is a bounded raw count (400 at
/// a cap of 200, 4631 for a complete 4625), so it never decides.
pub fn truncation(status: &AsyncQueryStatus, rows: usize, cap: u32) -> Truncation {
    let cap = u64::from(cap);
    let regions = status.regions.as_deref().unwrap_or_default();
    let counts: Vec<u64> = regions.iter().filter_map(|r| r.flows_count).collect();
    let truncated = if counts.is_empty() {
        status.flows_count.unwrap_or(rows as u64) >= cap
    } else {
        counts.iter().any(|&n| n >= cap)
    };
    Truncation {
        truncated,
        incomplete: regions
            .iter()
            .any(|r| r.responded == Some(false) || r.flows_count.is_none()),
    }
}

fn report(progress: Option<&UnboundedSender<QueryProgress>>, p: QueryProgress) {
    if let Some(tx) = progress {
        let _ = tx.send(p);
    }
}

/// Runs one query: POST, polls until completed, the download.
pub async fn run_traffic_query(
    client: &PceClient,
    query: &TrafficQuery,
    poll: &PollConfig,
    cancel: &CancellationToken,
    progress: Option<&UnboundedSender<QueryProgress>>,
) -> Result<TrafficResult> {
    run(client, query, false, poll, cancel, progress).await
}

/// `run_traffic_query` with the draft policy decision: POST, polls until
/// completed, the `update_rules` PUT, polls until its rules are
/// completed, then one download carrying `draft_policy_decision`.
pub async fn run_traffic_query_draft(
    client: &PceClient,
    query: &TrafficQuery,
    poll: &PollConfig,
    cancel: &CancellationToken,
    progress: Option<&UnboundedSender<QueryProgress>>,
) -> Result<TrafficResult> {
    run(client, query, true, poll, cancel, progress).await
}

/// Re-evaluates a completed query of ours (`query_href`, run with `query`)
/// against the draft policy, without a new POST: the `update_rules` PUT,
/// polls until its rules are completed, the download. A query the PCE no
/// longer holds is `PceError::NotFound`.
pub async fn evaluate_draft(
    client: &PceClient,
    query_href: &str,
    query: &TrafficQuery,
    poll: &PollConfig,
    cancel: &CancellationToken,
    progress: Option<&UnboundedSender<QueryProgress>>,
) -> Result<TrafficResult> {
    let started = Instant::now();
    let (result_href, done) = draft_stage(
        client, query_href, query, None, poll, cancel, progress, started,
    )
    .await?;
    finish(
        client,
        query_href,
        query,
        result_href,
        done,
        poll,
        cancel,
        progress,
        started,
    )
    .await
}

async fn run(
    client: &PceClient,
    query: &TrafficQuery,
    draft: bool,
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
    let (mut result_href, mut done) =
        wait_completed(client, &query_href, poll, cancel, progress, started).await?;
    if draft {
        (result_href, done) = draft_stage(
            client,
            &query_href,
            query,
            Some(done),
            poll,
            cancel,
            progress,
            started,
        )
        .await?;
    }
    finish(
        client,
        &query_href,
        query,
        result_href,
        done,
        poll,
        cancel,
        progress,
        started,
    )
    .await
}

/// One status GET, cancellable.
async fn status(
    client: &PceClient,
    query_href: &str,
    cancel: &CancellationToken,
) -> Result<AsyncQueryStatus> {
    tokio::select! {
        () = cancel.cancelled() => Err(PceError::Cancelled),
        r = client.get(query_href, &[]) => Ok(r?.body),
    }
}

/// `10 min`, or `45s` below a minute.
fn wait_text(d: Duration) -> String {
    let s = d.as_secs();
    if s >= 60 && s.is_multiple_of(60) {
        format!("{} min", s / 60)
    } else {
        format!("{s}s")
    }
}

/// `what` timed out once `since` is past `poll.max_wait`.
fn check_wait(since: Instant, poll: &PollConfig, what: &str) -> Result<()> {
    if since.elapsed() > poll.max_wait {
        return Err(PceError::QueryFailed(format!(
            "{what} timed out after {}",
            wait_text(poll.max_wait)
        )));
    }
    Ok(())
}

async fn pause(delay: &mut Duration, poll: &PollConfig, cancel: &CancellationToken) -> Result<()> {
    tokio::select! {
        () = cancel.cancelled() => return Err(PceError::Cancelled),
        () = tokio::time::sleep(*delay) => {}
    }
    *delay =
        Duration::from_secs_f64((delay.as_secs_f64() * poll.factor).min(poll.cap.as_secs_f64()));
    Ok(())
}

/// Polls with backoff until the query is completed with a result href.
async fn wait_completed(
    client: &PceClient,
    query_href: &str,
    poll: &PollConfig,
    cancel: &CancellationToken,
    progress: Option<&UnboundedSender<QueryProgress>>,
    started: Instant,
) -> Result<(String, AsyncQueryStatus)> {
    let mut delay = poll.initial;
    let mut completed_at: Option<Instant> = None;
    let waiting = Instant::now();
    loop {
        check_wait(waiting, poll, "the query")?;
        pause(&mut delay, poll, cancel).await?;
        let status = status(client, query_href, cancel).await?;
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
                Some(r) => return Ok((r.href().to_string(), status)),
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
    }
}

/// The `update_rules` PUT on a completed query, then polls with backoff
/// until both the query and its `rules` are completed.
///
/// `before` is the query's status just before the PUT: the one that
/// completed a fresh run, or (`None`, a query run earlier) one status GET
/// made here, which also finds a dropped query before the PUT. After a
/// repeat PUT the first polls can still show the previous evaluation's
/// `completed`, so `completed` counts only once the evaluation is new: the
/// query was not evaluated before, a state other than completed was seen
/// since the PUT, or `updated_at` moved; a PCE that shows none of these is
/// believed after `poll.grace`.
///
/// A fresh run's 404 means the PCE does not offer `update_rules` rather
/// than that the query is gone. A PCE that answers the PUT but never
/// reports `rules` fails after `poll.grace`; the stage gives up after
/// `poll.max_wait`.
#[allow(clippy::too_many_arguments)]
async fn draft_stage(
    client: &PceClient,
    query_href: &str,
    query: &TrafficQuery,
    before: Option<AsyncQueryStatus>,
    poll: &PollConfig,
    cancel: &CancellationToken,
    progress: Option<&UnboundedSender<QueryProgress>>,
    started: Instant,
) -> Result<(String, AsyncQueryStatus)> {
    let fresh = before.is_some();
    let refused = |e: PceError| match e {
        PceError::NotFound { .. } if !fresh => e,
        PceError::Cancelled | PceError::Invalid(_) => e,
        e => PceError::QueryFailed(format!(
            "the PCE refused the draft re-evaluation (update_rules): {e}"
        )),
    };
    let before = match before {
        Some(b) => b,
        None => status(client, query_href, cancel).await?,
    };
    let was_evaluated = before.rules_state() == Some("completed");
    report(
        progress,
        QueryProgress::Evaluating {
            rules: "requested".into(),
            elapsed: started.elapsed(),
        },
    );
    tokio::select! {
        () = cancel.cancelled() => return Err(PceError::Cancelled),
        r = client.update_query_rules(query_href, query.max_results) => r.map_err(refused)?,
    }
    let asked = Instant::now();
    let mut delay = poll.initial;
    let mut seen_unfinished = false;
    loop {
        check_wait(asked, poll, "the draft re-evaluation")?;
        pause(&mut delay, poll, cancel).await?;
        let status = status(client, query_href, cancel).await?;
        let rules = status.rules_state().map(str::to_string);
        report(
            progress,
            QueryProgress::Evaluating {
                rules: rules.clone().unwrap_or_else(|| "pending".into()),
                elapsed: started.elapsed(),
            },
        );
        if status.status.as_deref() == Some("failed") || rules.as_deref() == Some("failed") {
            return Err(PceError::QueryFailed(format!(
                "draft re-evaluation failed: {}",
                status.message.unwrap_or_else(|| "no message".into())
            )));
        }
        let completed = matches!(status.status.as_deref(), Some("completed" | "done"))
            && rules.as_deref() == Some("completed");
        if !completed {
            seen_unfinished = true;
        }
        let new = !was_evaluated
            || seen_unfinished
            || (status.updated_at.is_some() && status.updated_at != before.updated_at)
            || asked.elapsed() > poll.grace;
        match (&rules, &status.result) {
            (Some(_), Some(res)) if completed && new => {
                return Ok((res.href().to_string(), status));
            }
            (None, _) if asked.elapsed() > poll.grace => {
                return Err(PceError::QueryFailed(format!(
                    "the PCE did not start the draft re-evaluation within {:?} (no rules state)",
                    poll.grace
                )));
            }
            _ => {}
        }
    }
}

/// The download of a completed (or re-evaluated) query.
#[allow(clippy::too_many_arguments)]
async fn finish(
    client: &PceClient,
    query_href: &str,
    query: &TrafficQuery,
    result_href: String,
    done: AsyncQueryStatus,
    poll: &PollConfig,
    cancel: &CancellationToken,
    progress: Option<&UnboundedSender<QueryProgress>>,
    started: Instant,
) -> Result<TrafficResult> {
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
        query_href: query_href.to_string(),
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
    fn each_polling_stage_gives_up_after_10_minutes() {
        assert_eq!(PollConfig::default().max_wait, Duration::from_secs(600));
        assert_eq!(wait_text(Duration::from_secs(600)), "10 min");
        assert_eq!(wait_text(Duration::from_millis(30)), "0s");
    }

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
        // A region without a count: the counted ones decide, and the
        // result is incomplete (not truncated) for the silent one.
        s.regions.as_mut().unwrap()[1].flows_count = None;
        assert_eq!(
            truncation(&s, 16_815, 10_000),
            Truncation {
                truncated: false,
                incomplete: true
            }
        );
        // No region reports a count: the overall count decides.
        for r in s.regions.as_mut().unwrap() {
            r.flows_count = None;
        }
        assert!(truncation(&s, 16_815, 10_000).truncated);
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
