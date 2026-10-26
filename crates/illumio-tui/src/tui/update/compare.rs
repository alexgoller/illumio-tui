//! Compare mode on the Traffic screen: `c` and its prompt, the baseline's
//! answer, `n`, `C`, and `B` on a truncated baseline. Every baseline
//! request is the user's (`c`, `B`); nothing re-queries it on its own.
use super::*;
use crate::tui::model::{BaselineData, CompareState, ShowChanges};
use crate::tui::msg::BaselineOutcome;

/// The baseline's length `c` offers first.
pub(super) const DEFAULT_BASELINE: &str = "7d";

/// The status when a filter or window change ended compare mode.
pub(super) const COMPARE_ENDED: &str = "filter changed: c compares again";

/// `c`: the prompt for the baseline's length (the last one, else `7d`).
/// Needs an unfocused result on screen; asks nothing of the PCE yet.
pub(super) fn compare_key(model: &mut Model) -> Vec<Cmd> {
    let t = &model.traffic;
    if t.is_running() {
        model.status = StatusLine::info("query running: c once it is done");
        return Vec::new();
    }
    if t.data.as_ref().and_then(|d| d.range).is_none() {
        model.status = StatusLine::info("nothing to compare: r runs the query first");
        return Vec::new();
    }
    if t.focus_result.is_some() {
        model.status =
            StatusLine::info("compare works on the unfocused result: Esc clears the focus, then c");
        return Vec::new();
    }
    let input = t
        .compare
        .as_ref()
        .map_or(DEFAULT_BASELINE, |c| c.length.as_str())
        .to_string();
    model.overlay = Overlay::Baseline { input };
    Vec::new()
}

/// Keys in the baseline prompt: Enter runs it, Ctrl-N / Ctrl-P step
/// through the window steps, Ctrl-U clears, Esc closes.
pub(super) fn baseline_prompt_key(model: &mut Model, mut input: String, k: KeyEvent) -> Vec<Cmd> {
    let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
    match k.code {
        KeyCode::Esc => {
            model.overlay = Overlay::None;
            return Vec::new();
        }
        KeyCode::Enter => {
            model.overlay = Overlay::None;
            return request_baseline(model, input.trim());
        }
        KeyCode::Char(c @ ('n' | 'p')) if ctrl => {
            let steps = pce::traffic::WINDOW_STEPS;
            let next = match pce::traffic::parse_window(&input) {
                Ok(cur) if c == 'n' => pce::traffic::next_window(cur),
                Ok(cur) => pce::traffic::prev_window(cur),
                Err(_) => None,
            };
            input = next
                .or_else(|| {
                    // An unreadable length starts at an end of the steps.
                    pce::traffic::parse_window(&input).is_err().then(|| {
                        if c == 'n' {
                            steps[0]
                        } else {
                            steps[steps.len() - 1]
                        }
                    })
                })
                .map(str::to_string)
                .unwrap_or(input);
        }
        KeyCode::Char('u') if ctrl => input.clear(),
        KeyCode::Backspace => {
            input.pop();
        }
        KeyCode::Char(c) if !ctrl => input.push(c),
        _ => {}
    }
    model.overlay = Overlay::Baseline { input };
    Vec::new()
}

/// The baseline of `length` before the window of the result on screen,
/// through the cost guard (above 7d or above a 10k cap it asks first).
pub(super) fn request_baseline(model: &mut Model, length: &str) -> Vec<Cmd> {
    let n = match pce::traffic::parse_window(length) {
        Ok(n) if !n.is_zero() => n,
        Ok(_) => {
            model.status = StatusLine::error("baseline: give a length above 0, e.g. 7d");
            return Vec::new();
        }
        Err(e) => {
            model.status = StatusLine::error(format!("baseline: {e}"));
            return Vec::new();
        }
    };
    let Some(d) = model.traffic.data.as_ref() else {
        model.status = StatusLine::info("nothing to compare: r runs the query first");
        return Vec::new();
    };
    let Some(range) = d.range else {
        model.status = StatusLine::info("nothing to compare: r runs the query first");
        return Vec::new();
    };
    let end = range.start;
    let start = match jiff::Span::try_from(n).map(|span| end.checked_sub(span)) {
        Ok(Ok(start)) => start,
        _ => {
            model.status = StatusLine::error(format!("baseline {length}: too long"));
            return Vec::new();
        }
    };
    // The recent filter with the baseline's length as its window: what the
    // cost guard judges; the cap is the recent one's.
    let filter = TrafficFilter {
        window: length.to_string(),
        ..d.filter.clone()
    };
    ask_or_start(model, filter, start, end, None)
}

/// Asks first when `filter` is past a cost threshold (`what`: the reason's
/// tail, for `B`), else starts the baseline query.
fn ask_or_start(
    model: &mut Model,
    filter: TrafficFilter,
    start: jiff::Timestamp,
    end: jiff::Timestamp,
    what: Option<String>,
) -> Vec<Cmd> {
    if let Some(reason) = filter.needs_confirmation() {
        let tail = what.unwrap_or_else(|| format!("Run the {} baseline (1 query)", filter.window));
        model.overlay = Overlay::Confirm {
            reason: format!("{reason}. {tail}? (y/n)"),
            action: PendingAction::RunBaseline { filter, start, end },
        };
        return Vec::new();
    }
    start_baseline(model, filter, start, end)
}

/// Starts the baseline query (compare mode on, for the result on screen):
/// a baseline already loaded for the same filter and length stays shown
/// until the answer replaces it (`B`).
pub(super) fn start_baseline(
    model: &mut Model,
    filter: TrafficFilter,
    start: jiff::Timestamp,
    end: jiff::Timestamp,
) -> Vec<Cmd> {
    let t = &mut model.traffic;
    let Some(recent) = t.data.as_ref().map(|d| d.filter.clone()) else {
        model.status = StatusLine::info("nothing to compare: r runs the query first");
        return Vec::new();
    };
    if t.is_running() {
        model.status = StatusLine::info("query running: c once it is done");
        return Vec::new();
    }
    let length = filter.window.clone();
    let keep = t
        .compare
        .as_ref()
        .is_some_and(|c| c.length == length && c.same_filter(&recent));
    if !keep {
        t.compare = Some(CompareState {
            length: length.clone(),
            filter: recent,
            baseline: None,
            show: ShowChanges::All,
            counts: Default::default(),
        });
        t.rebuild_views();
    }
    t.compare_ended = false;
    let (seq, cancel) = t.start_baseline_run(Instant::now());
    model.status = StatusLine::info(format!(
        "baseline query running: {length} before the window"
    ));
    vec![Cmd::RunBaseline {
        seq,
        filter,
        start,
        end,
        cancel,
    }]
}

/// The baseline's answer, if compare mode is still on for it: every view
/// gains its `Δ`; the status names the counts, or warns of a truncated
/// side.
pub(super) fn baseline_done(
    model: &mut Model,
    seq: u64,
    result: Result<BaselineOutcome, String>,
) -> Vec<Cmd> {
    let t = &mut model.traffic;
    if t.seq != seq {
        return Vec::new(); // cancelled, or the result changed
    }
    t.query = QueryState::Idle;
    match result {
        Ok(o) => {
            let Some(cmp) = t.compare.as_mut() else {
                return Vec::new();
            };
            cmp.baseline = Some(BaselineData {
                flows: o.flows,
                truncated: o.truncated,
                incomplete: o.incomplete,
                hit: o.hit,
                filter: o.filter,
                range: Range {
                    start: o.start,
                    end: o.end,
                },
            });
            t.rebuild_views();
            model.status = compare_status(&model.traffic);
        }
        Err(e) => {
            // A first baseline that failed leaves compare mode off.
            if t.compare.as_ref().is_some_and(|c| c.baseline.is_none()) {
                t.compare = None;
                t.rebuild_views();
            }
            fail(model, "baseline", &e, false);
        }
    }
    Vec::new()
}

/// `vs 7d before · 12 new · 3 changed · 5 gone` for the view on screen;
/// a warning when either side is truncated.
pub(super) fn compare_status(t: &TrafficState) -> StatusLine {
    let Some(text) = t.compare_meta() else {
        return StatusLine::info("compare off");
    };
    match t.compare_warning() {
        Some(w) => StatusLine::warn(format!("{text} · {w}; B raises the cap")),
        None => StatusLine::info(text),
    }
}

/// `n`: all → new → changed → gone, every view rebuilt. No request.
pub(super) fn cycle_changes(model: &mut Model) {
    let t = &mut model.traffic;
    let Some(cmp) = t.compare.as_mut().filter(|c| c.baseline.is_some()) else {
        model.status = StatusLine::info("n: compare first (c)");
        return;
    };
    cmp.show = cmp.show.next();
    let show = cmp.show;
    t.rebuild_views();
    model.status = StatusLine::info(format!("compare: {} rows", show.label()));
}

/// `C`: compare mode off (a baseline run cancelled). No request.
pub(super) fn leave_compare(model: &mut Model) {
    model.status = StatusLine::info(if model.traffic.end_compare() {
        "compare off"
    } else {
        "not comparing: c compares with a baseline"
    });
}

/// `B` with a complete recent result and a truncated baseline: the
/// baseline again at the next cap, through the cost guard. `None` when
/// there is no truncated baseline (`B` then says what it says today).
pub(super) fn backfill_baseline(model: &mut Model) -> Option<Vec<Cmd>> {
    let t = &model.traffic;
    let cmp = t.compare.as_ref()?;
    let b = cmp.baseline.as_ref().filter(|b| b.truncated)?;
    let old = b.filter.max_results;
    let Some(cap) = pce::traffic::backfill_cap(old) else {
        model.status = StatusLine::info(format!(
            "baseline cap already at the maximum ({old}): narrow the filter"
        ));
        return Some(Vec::new());
    };
    let filter = TrafficFilter {
        max_results: cap,
        ..b.filter.clone()
    };
    let (start, end, n) = (b.range.start, b.range.end, b.flows.len());
    let what = format!(
        "Re-run the baseline at cap {cap} (1 query, replaces its {n} flows capped at {old})"
    );
    Some(ask_or_start(model, filter, start, end, Some(what)))
}
