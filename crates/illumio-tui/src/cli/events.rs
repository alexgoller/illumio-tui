//! `events`: PCE audit events with server-side severity/status/window
//! filters, and local ones over the loaded page: event-type terms
//! (`--type`, `--exclude-type`, the profile's `events.exclude_types`)
//! and a severity floor (`--severity warning+`).
use super::context::{Context, cache_label};
use super::error::CliError;
use super::output::{TableRows, Window, join_list};
use crate::tui::diff::Sign;
use crate::tui::event_diff::{DetailLine, Names, ResourceChange, body_lines, resource_changes};
use clap::Args;
use pce::models::{Event, EventFilter, SeverityFilter, TypeFilter, exclude_text};
use pce::traffic::{format_timestamp, parse_window};
use serde::Serialize;

#[derive(Args, Debug, Clone, Default)]
pub struct EventsArgs {
    /// emerg, alert, crit, err, warning, notice, info, debug (PCE-side
    /// filter); X+ is X and worse, e.g. warning+ (filtered locally)
    #[arg(long)]
    pub severity: Option<String>,
    /// Event types, local and case-insensitive: comma-separated terms,
    /// any of them (substring, or * glob over the whole type); !term
    /// excludes. E.g. 'sec_policy, rule_set', '!user.login', '*.delete'.
    /// Repeatable.
    #[arg(long = "type", value_name = "TERMS")]
    pub event_type: Vec<String>,
    /// Hide this event type (substring or * glob; a comma list hides
    /// each), as --type '!TYPE'. Repeatable.
    #[arg(long, value_name = "TYPE")]
    pub exclude_type: Vec<String>,
    /// Ignore the profile's events.exclude_types (show every type)
    #[arg(long)]
    pub all_types: bool,
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

/// The local filters of `args`: the type terms (`--type`, `--exclude-type`
/// and, without `--all-types`, the profile's excludes) and the severity;
/// with the words `meta.local_filter` names them by.
fn local_filters(
    args: &EventsArgs,
    profile_excludes: &[String],
) -> Result<(TypeFilter, SeverityFilter, Vec<String>), CliError> {
    let severity =
        SeverityFilter::parse(args.severity.as_deref().unwrap_or("")).map_err(CliError::Args)?;
    let excludes: Vec<&str> = args
        .exclude_type
        .iter()
        .flat_map(|t| t.split(','))
        .collect();
    let mut terms: Vec<String> = args.event_type.clone();
    terms.push(exclude_text(&excludes));
    if !args.all_types {
        terms.push(exclude_text(profile_excludes));
    }
    let text = terms
        .iter()
        .map(|t| t.trim())
        .filter(|t| !t.is_empty())
        .collect::<Vec<_>>()
        .join(", ");
    let types = TypeFilter::parse(&text).map_err(|e| CliError::Args(format!("--type: {e}")))?;
    let mut words = Vec::new();
    if !types.is_empty() {
        words.push(format!("type {text}"));
    }
    if severity.is_local() {
        words.push(format!(
            "severity {}",
            args.severity.as_deref().unwrap_or("").trim()
        ));
    }
    Ok((types, severity, words))
}

pub async fn run(ctx: &Context, args: &EventsArgs) -> Result<(), CliError> {
    let (types, severity, words) = local_filters(args, &ctx.config.event_exclude_types)?;
    let window = parse_window(&args.since).map_err(|e| CliError::Args(e.to_string()))?;
    let until = jiff::Timestamp::now();
    let since = jiff::Span::try_from(window)
        .and_then(|span| until.checked_sub(span))
        .map_err(|e| CliError::Args(format!("bad window {:?}: {e}", args.since)))?;
    let filter = EventFilter {
        severity: severity.pce_value(),
        event_type: None,
        status: args.status.clone(),
        since: Some(since),
        until: Some(until),
        max_results: args.max_results,
    };
    let c = ctx.pce.events(&filter, ctx.cache_mode).await?;
    let loaded = c.value.len();
    let data: Vec<Event> = c
        .value
        .into_iter()
        .filter(|e| types.keeps(e) && severity.keeps(e))
        .collect();
    let hidden = loaded - data.len();
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
    let mut meta = ctx.meta(
        cache_label(c.hit, ctx.cache_mode),
        c.truncated,
        Some(Window {
            start: format_timestamp(since),
            end: format_timestamp(until),
        }),
    );
    if !words.is_empty() {
        meta.local_filter = Some(words);
        meta.hidden = Some(hidden as u64);
    }
    ctx.emit_meta(
        &EventsOut {
            rows,
            diff: args.diff,
        },
        &meta,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_filters_join_types_excludes_and_the_profiles() {
        let args = |types: &[&str], ex: &[&str], all: bool| EventsArgs {
            event_type: types.iter().map(|s| s.to_string()).collect(),
            exclude_type: ex.iter().map(|s| s.to_string()).collect(),
            all_types: all,
            ..Default::default()
        };
        let profile = vec!["user.login".to_string()];
        let (t, s, words) = local_filters(&args(&["agent", "user"], &[], false), &profile).unwrap();
        assert!(t.keeps_type("agent.update") && t.keeps_type("user.logout"));
        assert!(!t.keeps_type("user.login"), "the profile's exclude");
        assert!(!t.keeps_type("sec_policy.create"));
        assert_eq!(s, SeverityFilter::Any);
        assert_eq!(words, ["type agent, user, !user.login"]);
        let (t, _, _) = local_filters(&args(&[], &[], true), &profile).unwrap();
        assert!(t.is_empty(), "--all-types drops the profile's excludes");
        let (t, _, _) = local_filters(&args(&[], &["agent.*,rule_set"], true), &[]).unwrap();
        assert!(!t.keeps_type("agent.update") && !t.keeps_type("rule_set.create"));
        assert!(t.keeps_type("user.login"));
        let (t, _, words) = local_filters(&args(&[], &[], false), &[]).unwrap();
        assert!(t.is_empty() && words.is_empty());
        let mut a = args(&[], &[], false);
        a.severity = Some("warning+".into());
        let (_, s, words) = local_filters(&a, &[]).unwrap();
        assert!(s.is_local());
        assert_eq!(words, ["severity warning+"]);
        assert!(local_filters(&args(&["a !b"], &[], false), &[]).is_err());
        a.severity = Some("loud+".into());
        assert!(local_filters(&a, &[]).is_err());
    }

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
