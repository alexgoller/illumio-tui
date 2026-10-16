//! `events`: PCE audit events with server-side severity/status/window filters
//! and a local event-type substring filter.
use super::context::{Context, cache_label};
use super::error::CliError;
use super::output::{TableRows, Window, join_list};
use crate::tui::diff::Sign;
use crate::tui::event_diff::{DetailLine, Names, ResourceChange, body_lines, resource_changes};
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
    /// Each event's resource changes as a diff, with its errors and
    /// notifications (JSON: a `changes` array per event). Reads the name
    /// index (cached) to show names for hrefs.
    #[arg(long)]
    pub diff: bool,
}

const HEADERS: [&str; 6] = [
    "timestamp",
    "severity",
    "type",
    "actor",
    "resource",
    "status",
];

fn row(e: &Event) -> Vec<String> {
    vec![
        e.timestamp.clone().unwrap_or_default(),
        e.severity.clone().unwrap_or_default(),
        e.event_type.clone().unwrap_or_default(),
        e.actor(),
        e.resource(),
        e.status.clone().unwrap_or_default(),
    ]
}

/// Every resource change, not just the first.
fn full_row(e: &Event) -> Vec<String> {
    let mut r = row(e);
    r[4] = join_list(e.resources());
    r
}

impl TableRows for Vec<Event> {
    fn headers(&self) -> Vec<&'static str> {
        HEADERS.to_vec()
    }
    fn rows(&self) -> Vec<Vec<String>> {
        self.iter().map(row).collect()
    }
    fn full_rows(&self) -> Vec<Vec<String>> {
        self.iter().map(full_row).collect()
    }
}

/// One event; with `--diff` also its read resource changes (`changes` is
/// left out of the JSON otherwise, so the shape without the flag is
/// unchanged) and the lines printed under it.
#[derive(Serialize)]
struct EventRow {
    #[serde(flatten)]
    event: Event,
    #[serde(skip_serializing_if = "Option::is_none")]
    changes: Option<Vec<ResourceChange>>,
    /// Errors, change blocks and notifications; the JSON has them in the event.
    #[serde(skip)]
    lines: Vec<DetailLine>,
}

impl EventRow {
    /// `names` with `--diff`: the event is read into changes and lines.
    fn new(event: Event, names: Option<&Names>) -> Self {
        let Some(names) = names else {
            return Self {
                event,
                changes: None,
                lines: Vec::new(),
            };
        };
        let names = names.for_event(&event);
        let changes = resource_changes(&event, &names);
        let lines = body_lines(&event, &changes, &names);
        Self {
            event,
            changes: Some(changes),
            lines,
        }
    }

    /// The lines as text: a title, then `+`/`-` lines.
    fn text(&self, indent: &str) -> Vec<String> {
        self.lines
            .iter()
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
    }
}

/// `events` as printed: JSON is the rows themselves; `diff` says whether
/// `--diff` asked for the lines (under each row, a CSV `diff` column).
#[derive(Serialize)]
#[serde(transparent)]
struct EventsOut {
    rows: Vec<EventRow>,
    #[serde(skip)]
    diff: bool,
}

impl TableRows for EventsOut {
    fn headers(&self) -> Vec<&'static str> {
        HEADERS.to_vec()
    }
    fn rows(&self) -> Vec<Vec<String>> {
        self.rows.iter().map(|r| row(&r.event)).collect()
    }
    fn notes(&self) -> Vec<Vec<String>> {
        if self.diff {
            self.rows.iter().map(|r| r.text("  ")).collect()
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
        self.rows
            .iter()
            .map(|r| {
                let mut cells = full_row(&r.event);
                if self.diff {
                    cells.push(r.text("").join("\n"));
                }
                cells
            })
            .collect()
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
    // Names as `policy pending --diff` has them: the index, cached, only with `--diff`.
    let index = match args.diff {
        true => Some(ctx.pce.name_index(ctx.cache_mode).await?),
        false => None,
    };
    let names = index.as_deref().map(|idx| Names {
        idx: Some(idx),
        ..Names::default()
    });
    let rows = data
        .into_iter()
        .map(|event| EventRow::new(event, names.as_ref()))
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
            rows: vec![EventRow::new(
                event.clone(),
                diff.then(Names::default).as_ref(),
            )],
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

    #[test]
    fn diff_shows_why_a_request_failed() {
        let event: Event = serde_json::from_value(serde_json::json!({
            "href": "/orgs/5/events/2",
            "event_type": "sec_rule.create",
            "status": "failure",
            "resource_changes": [],
            "notifications": [{"notification_type": "request.authorization_failed", "info": {"src_ip": "198.51.100.7"}}],
            "action": {"api_method": "POST", "http_status_code": 406, "errors": [{"token": "invalid_rule", "message": "No such label"}]}
        }))
        .unwrap();
        let out = EventsOut {
            rows: vec![EventRow::new(event, Some(&Names::default()))],
            diff: true,
        };
        assert_eq!(
            out.notes(),
            [[
                "    - error    invalid_rule: No such label",
                "  notifications",
                "      request.authorization_failed",
                "        src_ip       198.51.100.7",
            ]]
        );
    }
}
