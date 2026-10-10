//! Runs one Explorer query: POST, poll with backoff until completed, GET the
//! download. Cancellable between polls. Reports progress over a channel so a
//! UI can show elapsed time without owning the future.
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
}

impl Default for PollConfig {
    fn default() -> Self {
        Self {
            initial: Duration::from_secs(1),
            factor: 1.5,
            cap: Duration::from_secs(10),
            grace: Duration::from_secs(30),
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
    pub matches_count: Option<u64>,
    /// Rows hit `max_results`, or the PCE matched more than it returned.
    pub truncated: bool,
    pub query_href: String,
    pub elapsed: Duration,
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
    let mut matches_count = submitted.matches_count;
    let result_href = loop {
        tokio::select! {
            () = cancel.cancelled() => return Err(PceError::Cancelled),
            () = tokio::time::sleep(delay) => {}
        }
        let status: AsyncQueryStatus = client.get(&query_href, &[]).await?.body;
        let state = status.status.clone().unwrap_or_else(|| "unknown".into());
        matches_count = status.matches_count.or(matches_count);
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
            "completed" | "done" => match status.result {
                Some(r) => break r.href().to_string(),
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
    let flows: Vec<Flow> = client.get(&result_href, &[]).await?.body;
    let rows = flows.len();
    let truncated = rows as u64 >= u64::from(query.max_results)
        || matches_count.is_some_and(|m| m > rows as u64);
    report(progress, QueryProgress::Done { rows });
    Ok(TrafficResult {
        flows,
        matches_count,
        truncated,
        query_href,
        elapsed: started.elapsed(),
    })
}
