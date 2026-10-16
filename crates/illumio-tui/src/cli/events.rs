//! `events`: PCE audit events with server-side severity/status/window filters
//! and a local event-type substring filter.
use super::context::{Context, cache_label};
use super::error::CliError;
use super::output::{TableRows, Window, join_list};
use crate::tui::diff::Sign;
use crate::tui::event_diff::{DetailLine, Names, ResourceChange, change_lines, resource_changes};
use clap::Args;
use pce::models::{Event, EventFilter};
use pce::traffic::{format_timestamp, parse_window};
use serde::Serialize;

#[derive(Args, Debug, Clone, Default)]
pub struct EventsArgs {
    /// emerg, alert, crit, err, warning, notice, info, debug (PCE-side filter)
    #[arg(long)]
    pub severity: Option<String>,
    /// Event type substring, case-insensitive, e.g. agent. or sec_policy (local filter)
    #[arg(long = "type")]
    pub event_type: Option<String>,
    /// success | failure (PCE-side filter)
    #[arg(long)]
    pub status: Option<String>,
    /// Window ending now, e.g. 1h, 24h, 7d
    #[arg(long, default_value = "24h")]
    pub since: String,
    /// Row cap sent to the PCE (default 500)
    #[arg(long)]
    pub max_results: Option<u32>,
    /// Each event's resource changes as a diff (JSON: a `changes` array per
    /// event). No extra request: hrefs the event does not name stay hrefs.
    #[arg(long)]
    pub diff: bool,
}

impl TableRows for Vec<Event> {
    fn headers(&self) -> Vec<&'static str> {
        vec![
            "timestamp",
            "severity",
            "type",
            "actor",
            "resource",
            "status",
        ]
    }
    fn rows(&self) -> Vec<Vec<String>> {
        self.iter()
            .map(|e| {
                vec![
                    e.timestamp.clone().unwrap_or_default(),
                    e.severity.clone().unwrap_or_default(),
                    e.event_type.clone().unwrap_or_default(),
                    e.actor(),
                    e.resource(),
                    e.status.clone().unwrap_or_default(),
                ]
            })
            .collect()
    }
    /// Every resource change, not just the first.
    fn full_rows(&self) -> Vec<Vec<String>> {
        self.iter()
            .zip(self.rows())
            .map(|(e, mut row)| {
                row[4] = join_list(e.resources());
                row
            })
            .collect()
    }
}

/// One event; with `--diff` also its read resource changes (`changes` is
/// left out of the JSON otherwise, so the shape without the flag is unchanged).
#[derive(Serialize)]
struct EventRow {
    #[serde(flatten)]
    event: Event,
    #[serde(skip_serializing_if = "Option::is_none")]
    changes: Option<Vec<ResourceChange>>,
}

/// `events` as printed: JSON is the rows themselves; `diff` says whether
/// `--diff` asked for the change lines (under each row, a CSV `diff` column).
#[derive(Serialize)]
#[serde(transparent)]
struct EventsOut {
    rows: Vec<EventRow>,
    #[serde(skip)]
    diff: bool,
}

impl EventsOut {
    fn events(&self) -> Vec<Event> {
        self.rows.iter().map(|r| r.event.clone()).collect()
    }

    /// The change lines of each row as text: a title, then `+`/`-` lines.
    fn lines(&self, indent: &str) -> Vec<Vec<String>> {
        self.rows
            .iter()
            .map(|r| {
                change_lines(r.changes.as_deref().unwrap_or_default())
                    .into_iter()
                    .map(|l| match l {
                        DetailLine::Title(t) => format!("{indent}{t}"),
                        DetailLine::Line(d) => {
                            let g = match d.sign {
                                Sign::Added => "+",
                                Sign::Removed => "-",
                                Sign::Same => " ",
                            };
                            format!("{indent}  {g} {}", d.text)
                        }
                    })
                    .collect()
            })
            .collect()
    }
}

impl TableRows for EventsOut {
    fn headers(&self) -> Vec<&'static str> {
        self.events().headers()
    }
    fn rows(&self) -> Vec<Vec<String>> {
        self.events().rows()
    }
    fn notes(&self) -> Vec<Vec<String>> {
        if self.diff {
            self.lines("  ")
        } else {
            Vec::new()
        }
    }
    fn full_headers(&self) -> Vec<&'static str> {
        let mut h = self.headers();
        if self.diff {
            h.push("diff");
        }
        h
    }
    /// With `--diff`, the lines one per line of the `diff` cell.
    fn full_rows(&self) -> Vec<Vec<String>> {
        let mut rows = self.events().full_rows();
        if self.diff {
            for (row, lines) in rows.iter_mut().zip(self.lines("")) {
                row.push(lines.join("\n"));
            }
        }
        rows
    }
}

pub async fn run(ctx: &Context, args: &EventsArgs) -> Result<(), CliError> {
    let window = parse_window(&args.since).map_err(|e| CliError::Args(e.to_string()))?;
    let until = jiff::Timestamp::now();
    let since = jiff::Span::try_from(window)
        .and_then(|span| until.checked_sub(span))
        .map_err(|e| CliError::Args(format!("bad window {:?}: {e}", args.since)))?;
    let filter = EventFilter {
        severity: args.severity.clone(),
        event_type: None,
        status: args.status.clone(),
        since: Some(since),
        until: Some(until),
        max_results: args.max_results,
    };
    let c = ctx.pce.events(&filter, ctx.cache_mode).await?;
    let needle = args.event_type.as_deref().map(str::to_ascii_lowercase);
    let data: Vec<Event> = c
        .value
        .into_iter()
        .filter(|e| {
            needle.as_deref().is_none_or(|n| {
                e.event_type
                    .as_deref()
                    .unwrap_or("")
                    .to_ascii_lowercase()
                    .contains(n)
            })
        })
        .collect();
    let names = Names::default();
    let rows = data
        .into_iter()
        .map(|event| EventRow {
            changes: args.diff.then(|| resource_changes(&event, &names)),
            event,
        })
        .collect();
    ctx.emit(
        &EventsOut {
            rows,
            diff: args.diff,
        },
        cache_label(c.hit, ctx.cache_mode),
        c.truncated,
        Some(Window {
            start: format_timestamp(since),
            end: format_timestamp(until),
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csv_names_every_resource_change() {
        let events: Vec<Event> = serde_json::from_value(serde_json::json!([{
            "href": "/orgs/5/events/1",
            "resource_changes": [
                {"resource": {"workload": {"hostname": "web01"}}},
                {"resource": {"label": {"href": "/orgs/5/labels/7"}}}
            ]
        }]))
        .unwrap();
        let resource = events
            .headers()
            .iter()
            .position(|h| *h == "resource")
            .unwrap();
        assert_eq!(events.rows()[0][resource], "workload web01");
        assert_eq!(events.full_rows()[0][resource], "workload web01; label 7");
    }

    #[test]
    fn diff_adds_changes_and_lines_only_when_asked() {
        let event: Event = serde_json::from_value(serde_json::json!({
            "href": "/orgs/5/events/1",
            "event_type": "label.update",
            "resource_changes": [{
                "change_type": "update",
                "changes": {"value": {"before": "shop", "after": "store"}},
                "resource": {"label": {"href": "/orgs/5/labels/3", "key": "app", "value": "store"}}
            }]
        }))
        .unwrap();
        let out = |diff: bool| EventsOut {
            rows: vec![EventRow {
                changes: diff.then(|| resource_changes(&event, &Names::default())),
                event: event.clone(),
            }],
            diff,
        };
        let plain = serde_json::to_value(out(false)).unwrap();
        assert_eq!(plain[0], serde_json::to_value(&event).unwrap());
        assert!(out(false).notes().is_empty());
        let json = serde_json::to_value(out(true)).unwrap();
        assert_eq!(
            json[0]["changes"],
            serde_json::json!([{
                "change_type": "update",
                "resource_type": "label",
                "resource": "label app=store",
                "href": "/orgs/5/labels/3",
                "fields": [{"field": "value", "before": "shop", "after": "store"}]
            }])
        );
        assert_eq!(
            out(true).notes(),
            [[
                "  update label app=store",
                "    - value: shop",
                "    + value: store"
            ]]
        );
        let full = out(true);
        assert_eq!(full.full_headers().last(), Some(&"diff"));
        assert_eq!(
            full.full_rows()[0].last().unwrap(),
            "update label app=store\n  - value: shop\n  + value: store"
        );
    }
}
