//! The `?` help: key lines in titled groups, and the page shown for the
//! context the help was opened from (its group first, then related ones,
//! then the keys that work everywhere). `Tab` / `BackTab` show one group
//! at a time, `a` all of them. Rendering is in `ui::widgets::overlay`.
use crate::tui::model::{Model, Overlay, Screen, TrafficView};

/// A titled group of key lines; a line with an empty key continues the one above.
pub struct HelpGroup {
    pub title: &'static str,
    pub lines: &'static [(&'static str, &'static str)],
}

/// The groups, in the order `Tab` walks them; `HelpTopic` indexes it.
pub const HELP_GROUPS: &[HelpGroup] = &[
    HelpGroup {
        title: "Everywhere",
        lines: &[
            (
                "1 2 3 4 5",
                "Traffic · Policy · Objects · Workloads · Events",
            ),
            ("?", "this help"),
            ("q / Ctrl-C", "quit"),
            ("↑ ↓ PgUp PgDn", "move in the current table (also Home End)"),
            ("s", "cycle sort column and direction"),
            ("/", "search in the current table (Enter keep, Esc clear)"),
            ("r", "refresh the current screen (bypasses the cache)"),
            ("a", "toggle auto-refresh for the current screen"),
            ("y / n", "send / cancel a shown request"),
            ("Esc", "close overlay / cancel"),
            (
                "Units",
                "flows: flow records; conns, bytes: their lifetime counts",
            ),
        ],
    },
    HelpGroup {
        title: "Commands",
        lines: &[
            (":", "command prompt: :traffic :policy :objects :workloads"),
            ("", ":events  :iplists  :filter <expr>  :window <dur>"),
            ("", ":profile <name>  :draft  :active  :marks  :help  :q"),
            ("", ":unicast  unicast flows only (again: every kind)"),
            ("", ":focus <app>  (alone: a picker of the loaded apps)"),
            ("", ":app <app>  the app overlay (alone: the picker)"),
            ("", ":gen <app>  the policy generator (alone: the picker)"),
            ("", ":writes [on|off]  :export[!] <path>  :batch"),
            ("", "(the table on screen as CSV; ! overwrites)"),
        ],
    },
    HelpGroup {
        title: "Traffic",
        lines: &[
            ("v", "cycle Flows · App-to-app · Mesh · Blocked · Treemap"),
            ("f", "edit filter: window src dst svc decision transmission"),
            ("+ / -", "widen (re-runs the window) / narrow (no request)"),
            ("", "window steps 1h 6h 12h 1d 2d 3d 1w 2w 4w 90d"),
            ("B", "re-run a truncated result at a larger cap"),
            ("", "one rung per B: 10000 25000 … 200000; y above 10000"),
            ("] / [", "raise / lower the result cap (re-runs the query)"),
            ("", "cap steps 500 1000 2000 5000 10000 25000 … 200000"),
            ("g", "pick the label keys every view groups by"),
            (
                "F / X",
                "focus on the row's app (again: its other side) · in/out/both",
            ),
            (
                "",
                "Esc clears; truncated: r re-queries the app (1-2 queries)",
            ),
            ("A", "app overlay: peers, policy, workloads"),
            (
                "D",
                "draft decision per flow (PCE draft view; again: clear)",
            ),
            (
                "",
                "1 PUT on the query, polls, 1 download; no policy change",
            ),
            ("", "Esc cancels it; no known query: asks, then re-runs"),
            ("w", "draft allow rule from a blocked flow or row (writes)"),
            (
                "m",
                "mark the row: squelch · reviewed · critical … (marks.toml)",
            ),
            ("M", "show or hide squelched rows (~)"),
            (
                "Enter",
                "detail (flows) · drill into a row or section header",
            ),
        ],
    },
    HelpGroup {
        title: "Flows",
        lines: &[(
            "x / X",
            "flat · by src app · by dst app · back (focused: X is direction)",
        )],
    },
    HelpGroup {
        title: "App-to-app",
        lines: &[
            (
                "x / X",
                "pairs · in · out · both · back (focused: X is direction)",
            ),
            ("o", "pairs by service: one row per src, dst, port/proto"),
            (
                "d",
                "decision filter: all · blocked · blocked only (· fixed by draft)",
            ),
        ],
    },
    HelpGroup {
        title: "Sections",
        lines: &[
            (
                "Space / z",
                "fold a section · fold or unfold all (also Blocked, Flows)",
            ),
            (
                "s",
                "order: conns ↓ · app A→Z · flows ↓ (App-to-app, Flows)",
            ),
            (
                "",
                "Blocked is App-to-app with in · blocked: x / X / d are there",
            ),
        ],
    },
    HelpGroup {
        title: "Mesh",
        lines: &[("u / p", "show or hide the user · process columns")],
    },
    HelpGroup {
        title: "Treemap",
        lines: &[(
            "hjkl Enter b",
            "move · zoom, then drill · conns/bytes · Esc out",
        )],
    },
    HelpGroup {
        title: "Marks",
        lines: &[
            (
                ":marks [action]",
                "the marks: Enter edit · x delete · e +30d",
            ),
            ("", "(an action: only its marks; Esc / q back)"),
        ],
    },
    HelpGroup {
        title: "App overlay",
        lines: &[
            (
                "Tab j/k X Enter",
                "panel · row · direction · open (peer: its flows)",
            ),
            (
                "m w r Esc",
                "mark · draft allow rule · reload panels (asks) · back",
            ),
            ("G", "policy generator: the rules the app's traffic needs"),
        ],
    },
    HelpGroup {
        title: "Generator",
        lines: &[
            (
                "←/→ < > =",
                "granularity · the row's own (*) · back to the slider",
            ),
            ("j/k Tab q", "rows (on into outbound) · section · back"),
            (
                "Space o R p",
                "check a row · outbound · pick the ruleset · rule list",
            ),
            ("c", "fan-in line: one rule off · Any · All Workloads"),
            (
                "Enter r Esc",
                "plan the writes, one confirm (writes) · re-query · back",
            ),
        ],
    },
    HelpGroup {
        title: "Forms",
        lines: &[
            (
                "Ctrl-N Ctrl-P",
                "form: complete from the index · cycle back",
            ),
            (
                "Ctrl-N Ctrl-P",
                "filter: complete the filter fields · cycle back",
            ),
            (
                "Ruleset",
                "w form: found by the destination's scope · Tab/Ctrl-N names",
            ),
            (
                "Ctrl-A Ctrl-R",
                "w form: all labels · create the typed ruleset",
            ),
        ],
    },
    HelpGroup {
        title: "Policy",
        lines: &[
            ("d", "toggle draft / active"),
            ("p", "pending changes"),
            (
                "Tab / Enter",
                "pane · rule detail · on a ruleset: load deny rules",
            ),
            ("N", "new deny rule in the selected ruleset (rules pane)"),
            ("i", "rule impact: load the workloads once (1 request)"),
        ],
    },
    HelpGroup {
        title: "Pending",
        lines: &[
            ("j/k Space Enter", "select · mark · diff of a pending entry"),
            ("P", "provision the marked entries, or all (writes)"),
            ("R", "revert marked entries (draft-only ones are deleted)"),
        ],
    },
    HelpGroup {
        title: "Objects",
        lines: &[
            (
                "Tab / Enter",
                "next tab: labels, groups, services, IP lists · detail",
            ),
            (
                "f / i / u",
                "Labels: key filter · ignored keys · unused only (PCE usage)",
            ),
            (
                "/=text /k=text",
                "Labels: search values only · the values of key k",
            ),
            (
                "Enter",
                "IP lists: detail: ranges · FQDNs · size · rules using it",
            ),
        ],
    },
    HelpGroup {
        title: "Workloads",
        lines: &[
            (
                "t/T h H Enter",
                "Traffic as src/dst · unhealthy · health sort · detail",
            ),
            (
                "E m o u c",
                "enforcement · managed · offline · unlabeled · clear",
            ),
            ("", "local, no request; c clears them and h, not /"),
        ],
    },
    HelpGroup {
        title: "Events",
        lines: &[
            (
                "f / Enter / S",
                "filter · event as a change diff · sort by severity",
            ),
            ("J", "in the detail: change diff ↔ raw JSON"),
            (
                "",
                "detail: ↑/↓ j/k prev/next event · ^d/^u PgDn/PgUp ^e/^y g/G scroll",
            ),
        ],
    },
    HelpGroup {
        title: "Writes",
        lines: &[
            (
                "n / e / x",
                "new/edit/delete: Objects, IP lists, Workloads, Policy",
            ),
            ("", "with :writes on; each request is shown first (y / n)"),
        ],
    },
];

/// A group of `HELP_GROUPS`, by its position there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HelpTopic {
    Everywhere,
    Commands,
    Traffic,
    Flows,
    AppToApp,
    Sections,
    Mesh,
    Treemap,
    Marks,
    AppOverlay,
    Generator,
    Forms,
    Policy,
    Pending,
    Objects,
    Workloads,
    Events,
    Writes,
}

impl HelpTopic {
    /// In `HELP_GROUPS` order.
    pub const ALL: [HelpTopic; 18] = [
        HelpTopic::Everywhere,
        HelpTopic::Commands,
        HelpTopic::Traffic,
        HelpTopic::Flows,
        HelpTopic::AppToApp,
        HelpTopic::Sections,
        HelpTopic::Mesh,
        HelpTopic::Treemap,
        HelpTopic::Marks,
        HelpTopic::AppOverlay,
        HelpTopic::Generator,
        HelpTopic::Forms,
        HelpTopic::Policy,
        HelpTopic::Pending,
        HelpTopic::Objects,
        HelpTopic::Workloads,
        HelpTopic::Events,
        HelpTopic::Writes,
    ];

    pub fn group(self) -> &'static HelpGroup {
        &HELP_GROUPS[self as usize]
    }

    /// `Tab`: the next group, wrapping.
    pub fn next(self) -> Self {
        Self::ALL[(self as usize + 1) % Self::ALL.len()]
    }

    /// `BackTab`: the previous group, wrapping.
    pub fn prev(self) -> Self {
        Self::ALL[(self as usize + Self::ALL.len() - 1) % Self::ALL.len()]
    }
}

/// What the help was opened from: the screen (the traffic view on
/// Traffic) or the overlay it returns to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HelpContext {
    Flows,
    AppToApp,
    Mesh,
    Blocked,
    Treemap,
    Policy,
    Objects,
    Workloads,
    Events,
    AppOverlay,
    Generator,
    Pending,
    Marks,
}

impl HelpContext {
    /// The name in the help's title.
    pub fn name(self) -> &'static str {
        match self {
            HelpContext::Flows => "Flows",
            HelpContext::AppToApp => "App-to-app",
            HelpContext::Mesh => "Mesh",
            HelpContext::Blocked => "Blocked",
            HelpContext::Treemap => "Treemap",
            HelpContext::Policy => "Policy",
            HelpContext::Objects => "Objects",
            HelpContext::Workloads => "Workloads",
            HelpContext::Events => "Events",
            HelpContext::AppOverlay => "App overlay",
            HelpContext::Generator => "Generator",
            HelpContext::Pending => "Pending",
            HelpContext::Marks => "Marks",
        }
    }

    /// The groups of the context page, its own first and `Everywhere` last.
    /// Blocked has no keys of its own: its sections' keys come first.
    pub fn topics(self) -> &'static [HelpTopic] {
        use HelpTopic as T;
        match self {
            HelpContext::Flows => &[T::Flows, T::Sections, T::Traffic, T::Marks, T::Everywhere],
            HelpContext::AppToApp => &[
                T::AppToApp,
                T::Sections,
                T::Traffic,
                T::Marks,
                T::Everywhere,
            ],
            HelpContext::Blocked => &[T::Sections, T::Traffic, T::Marks, T::Everywhere],
            HelpContext::Mesh => &[T::Mesh, T::Traffic, T::Marks, T::Everywhere],
            HelpContext::Treemap => &[T::Treemap, T::Traffic, T::Everywhere],
            HelpContext::Policy => &[T::Policy, T::Pending, T::Writes, T::Everywhere],
            HelpContext::Objects => &[T::Objects, T::Writes, T::Everywhere],
            HelpContext::Workloads => &[T::Workloads, T::Writes, T::Everywhere],
            HelpContext::Events => &[T::Events, T::Everywhere],
            HelpContext::AppOverlay => &[T::AppOverlay, T::Forms, T::Marks, T::Everywhere],
            HelpContext::Generator => &[T::Generator, T::Everywhere],
            HelpContext::Pending => &[T::Pending, T::Policy, T::Everywhere],
            HelpContext::Marks => &[T::Marks, T::Forms, T::Everywhere],
        }
    }

    /// The context's own group: where `Tab` starts from.
    pub fn topic(self) -> HelpTopic {
        self.topics()[0]
    }

    /// The context of `model`'s help: the overlay it returns to, else the
    /// screen (on Traffic: the view).
    pub fn of(model: &Model) -> Self {
        match &model.help_return {
            Overlay::App => return HelpContext::AppOverlay,
            Overlay::Generator => return HelpContext::Generator,
            Overlay::EventDetail { .. } => return HelpContext::Events,
            Overlay::Pending => return HelpContext::Pending,
            Overlay::Marks => return HelpContext::Marks,
            _ => {}
        }
        match model.screen {
            Screen::Traffic => match model.traffic.view {
                TrafficView::Flows => HelpContext::Flows,
                TrafficView::Aggregate => HelpContext::AppToApp,
                TrafficView::Mesh => HelpContext::Mesh,
                TrafficView::Blocked => HelpContext::Blocked,
                TrafficView::Treemap => HelpContext::Treemap,
            },
            Screen::Policy => HelpContext::Policy,
            Screen::Objects => HelpContext::Objects,
            Screen::Workloads => HelpContext::Workloads,
            Screen::Events => HelpContext::Events,
        }
    }
}

/// Which groups the help shows: the context page, one group (`Tab`), or all (`a`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HelpPage {
    #[default]
    Context,
    Group(HelpTopic),
    All,
}

impl HelpPage {
    /// `Tab` (`forward`) / `BackTab`: one group at a time, from the context's own.
    pub fn step(self, ctx: HelpContext, forward: bool) -> Self {
        let from = match self {
            HelpPage::Group(t) => t,
            HelpPage::Context | HelpPage::All => ctx.topic(),
        };
        HelpPage::Group(if forward { from.next() } else { from.prev() })
    }

    /// `a`: all groups, or back to the context page.
    pub fn toggle_all(self) -> Self {
        match self {
            HelpPage::All => HelpPage::Context,
            _ => HelpPage::All,
        }
    }

    /// The groups shown, in order.
    pub fn topics(self, ctx: HelpContext) -> Vec<HelpTopic> {
        match self {
            HelpPage::Context => ctx.topics().to_vec(),
            HelpPage::Group(t) => vec![t],
            HelpPage::All => {
                let first = ctx.topics();
                let rest = HelpTopic::ALL.iter().filter(|t| !first.contains(t));
                first.iter().chain(rest).copied().collect()
            }
        }
    }
}

/// A line of a help page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HelpRow {
    Heading(&'static str),
    Key(&'static str, &'static str),
    Blank,
}

/// The lines of `page`: each group under its heading, a blank line between groups.
pub fn page_rows(page: HelpPage, ctx: HelpContext) -> Vec<HelpRow> {
    let mut rows = Vec::new();
    for (i, t) in page.topics(ctx).into_iter().enumerate() {
        if i > 0 {
            rows.push(HelpRow::Blank);
        }
        let g = t.group();
        rows.push(HelpRow::Heading(g.title));
        rows.extend(g.lines.iter().map(|(k, d)| HelpRow::Key(k, d)));
    }
    rows
}

/// The groups not on `page`, for its footer line.
pub fn other_topics(page: HelpPage, ctx: HelpContext) -> Vec<HelpTopic> {
    let shown = page.topics(ctx);
    HelpTopic::ALL
        .iter()
        .copied()
        .filter(|t| !shown.contains(t))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn topics_index_their_groups() {
        assert_eq!(HELP_GROUPS.len(), HelpTopic::ALL.len());
        for (i, t) in HelpTopic::ALL.iter().enumerate() {
            assert_eq!(*t as usize, i);
            assert!(!t.group().lines.is_empty(), "{}", t.group().title);
            assert_ne!(t.group().lines[0].0, "", "a group starts with a key");
        }
        let titles: Vec<&str> = HELP_GROUPS.iter().map(|g| g.title).collect();
        for t in [
            "Everywhere",
            "Commands",
            "Traffic",
            "Flows",
            "App-to-app",
            "Sections",
            "Mesh",
            "Treemap",
            "Marks",
            "App overlay",
            "Generator",
            "Forms",
            "Policy",
            "Pending",
            "Objects",
            "Workloads",
            "Events",
            "Writes",
        ] {
            assert!(titles.contains(&t), "{t}");
        }
    }

    /// Every line of the old flat help (79 lines; its key's prefix was the
    /// group): the key without the prefix and a fragment of its text.
    const OLD_LINES: &[(&str, &str)] = &[
        (
            "1 2 3 4 5",
            "Traffic · Policy · Objects · Workloads · Events",
        ),
        (":", "command prompt: :traffic :policy :objects :workloads"),
        ("", ":events  :iplists  :filter <expr>  :window <dur>"),
        ("", ":profile <name>  :draft  :active  :marks  :help  :q"),
        ("", ":unicast  unicast flows only (again: every kind)"),
        ("", ":focus <app>  (alone: a picker of the loaded apps)"),
        ("", ":app <app>  the app overlay (alone: the picker)"),
        ("", ":gen <app>  the policy generator (alone: the picker)"),
        ("", ":writes [on|off]  :export[!] <path>  :batch"),
        ("", "(the table on screen as CSV; ! overwrites)"),
        ("?", "this help"),
        ("q / Ctrl-C", "quit"),
        // Was "↑ ↓ PgUp PgDn Home End": Home End moved to the text.
        ("↑ ↓ PgUp PgDn", "move in the current table (also Home End)"),
        ("s", "cycle sort column and direction"),
        ("/", "search in the current table (Enter keep, Esc clear)"),
        ("r", "refresh the current screen (bypasses the cache)"),
        ("a", "toggle auto-refresh for the current screen"),
        ("v", "cycle Flows · App-to-app · Mesh · Blocked · Treemap"),
        ("f", "edit filter: window src dst svc decision transmission"),
        ("+ / -", "widen (re-runs the window) / narrow (no request)"),
        ("", "window steps 1h 6h 12h 1d 2d 3d 1w 2w 4w 90d"),
        ("B", "re-run a truncated result at a larger cap"),
        ("", "one rung per B: 10000 25000 … 200000; y above 10000"),
        ("] / [", "raise / lower the result cap (re-runs the query)"),
        ("", "cap steps 500 1000 2000 5000 10000 25000 … 200000"),
        ("g", "pick the label keys every view groups by"),
        (
            "F / X",
            "focus on the row's app (again: its other side) · in/out/both",
        ),
        (
            "",
            "Esc clears; truncated: r re-queries the app (1-2 queries)",
        ),
        ("A", "app overlay: peers, policy, workloads"),
        (
            "D",
            "draft decision per flow (PCE draft view; again: clear)",
        ),
        (
            "",
            "1 PUT on the query, polls, 1 download; no policy change",
        ),
        ("", "Esc cancels it; no known query: asks, then re-runs"),
        (
            "Units",
            "flows: flow records; conns, bytes: their lifetime counts",
        ),
        (
            "Tab j/k X Enter",
            "panel · row · direction · open (peer: its flows)",
        ),
        (
            "m w r Esc",
            "mark · draft allow rule · reload panels (asks) · back",
        ),
        ("G", "policy generator: the rules the app's traffic needs"),
        (
            "←/→ < > =",
            "granularity · the row's own (*) · back to the slider",
        ),
        ("j/k Tab q", "rows (on into outbound) · section · back"),
        (
            "Space o R p",
            "check a row · outbound · pick the ruleset · rule list",
        ),
        ("c", "fan-in line: one rule off · Any · All Workloads"),
        (
            "Enter r Esc",
            "plan the writes, one confirm (writes) · re-query · back",
        ),
        (
            "Enter",
            "detail (flows) · drill into a row or section header",
        ),
        (
            "x / X",
            "pairs · in · out · both · back (focused: X is direction)",
        ),
        ("o", "pairs by service: one row per src, dst, port/proto"),
        (
            "d",
            "decision filter: all · blocked · blocked only (· fixed by draft)",
        ),
        (
            "x / X",
            "flat · by src app · by dst app · back (focused: X is direction)",
        ),
        (
            "Space / z",
            "fold a section · fold or unfold all (also Blocked, Flows)",
        ),
        (
            "s",
            "order: conns ↓ · app A→Z · flows ↓ (App-to-app, Flows)",
        ),
        ("u / p", "show or hide the user · process columns"),
        (
            "hjkl Enter b",
            "move · zoom, then drill · conns/bytes · Esc out",
        ),
        ("w", "draft allow rule from a blocked flow or row (writes)"),
        (
            "m",
            "mark the row: squelch · reviewed · critical … (marks.toml)",
        ),
        ("M", "show or hide squelched rows (~)"),
        (
            ":marks [action]",
            "the marks: Enter edit · x delete · e +30d",
        ),
        ("Ctrl-N Ctrl-P", "complete from the index · cycle back"),
        ("Ctrl-N Ctrl-P", "complete the filter fields · cycle back"),
        (
            "Ruleset",
            "found by the destination's scope · Tab/Ctrl-N names",
        ),
        ("Ctrl-A Ctrl-R", "all labels · create the typed ruleset"),
        ("d", "toggle draft / active"),
        ("p", "pending changes"),
        (
            "Tab / Enter",
            "pane · rule detail · on a ruleset: load deny rules",
        ),
        ("N", "new deny rule in the selected ruleset (rules pane)"),
        ("i", "rule impact: load the workloads once (1 request)"),
        (
            "Tab / Enter",
            "next tab: labels, groups, services, IP lists · detail",
        ),
        (
            "f / i / u",
            "key filter · ignored keys · unused only (PCE usage)",
        ),
        ("/=text /k=text", "search values only · the values of key k"),
        ("Enter", "detail: ranges · FQDNs · size · rules using it"),
        (
            "t/T h H Enter",
            "Traffic as src/dst · unhealthy · health sort · detail",
        ),
        (
            "E m o u c",
            "enforcement · managed · offline · unlabeled · clear",
        ),
        ("", "local, no request; c clears them and h, not /"),
        (
            "f / Enter / S",
            "filter · event as a change diff · sort by severity",
        ),
        ("J", "in the detail: change diff ↔ raw JSON"),
        (
            "",
            "detail: ↑/↓ j/k prev/next event · ^d/^u PgDn/PgUp ^e/^y g/G scroll",
        ),
        (
            "n / e / x",
            "new/edit/delete: Objects, IP lists, Workloads, Policy",
        ),
        ("j/k Space Enter", "select · mark · diff of a pending entry"),
        ("P", "provision the marked entries, or all (writes)"),
        ("R", "revert marked entries (draft-only ones are deleted)"),
        ("y / n", "send / cancel a shown request"),
        ("Esc", "close overlay / cancel"),
    ];

    #[test]
    fn every_old_help_line_is_in_a_group() {
        assert_eq!(OLD_LINES.len(), 79);
        let all: Vec<(&str, &str)> = HELP_GROUPS
            .iter()
            .flat_map(|g| g.lines.iter().copied())
            .collect();
        for (k, d) in OLD_LINES {
            assert!(
                all.iter().any(|(gk, gd)| *gk == *k && gd.contains(d)),
                "{k:?} {d:?}"
            );
        }
        // The prefixes are gone: a key is just the keys.
        for (k, _) in &all {
            for p in ["Traffic  ", "Generate ", "App      ", "Policy   ", "w form"] {
                assert!(!k.starts_with(p), "{k}");
            }
        }
    }

    #[test]
    fn pages_put_the_context_first_and_everywhere_last_on_the_context_page() {
        use HelpContext as C;
        for (ctx, first) in [
            (C::Flows, "Flows"),
            (C::AppToApp, "App-to-app"),
            (C::Mesh, "Mesh"),
            (C::Blocked, "Sections"),
            (C::Treemap, "Treemap"),
            (C::Policy, "Policy"),
            (C::Objects, "Objects"),
            (C::Workloads, "Workloads"),
            (C::Events, "Events"),
            (C::AppOverlay, "App overlay"),
            (C::Generator, "Generator"),
            (C::Pending, "Pending"),
            (C::Marks, "Marks"),
        ] {
            let rows = page_rows(HelpPage::Context, ctx);
            assert_eq!(rows[0], HelpRow::Heading(first), "{ctx:?}");
            let last_heading = rows.iter().rev().find_map(|r| match r {
                HelpRow::Heading(h) => Some(*h),
                _ => None,
            });
            assert_eq!(last_heading, Some("Everywhere"), "{ctx:?}");
            let all = page_rows(HelpPage::All, ctx);
            assert_eq!(all[0], HelpRow::Heading(first), "{ctx:?}");
            let headings = all
                .iter()
                .filter(|r| matches!(r, HelpRow::Heading(_)))
                .count();
            assert_eq!(headings, HELP_GROUPS.len(), "every group once");
        }
        assert!(other_topics(HelpPage::Context, C::Flows).contains(&HelpTopic::Commands));
        assert!(other_topics(HelpPage::All, C::Flows).is_empty());
    }

    #[test]
    fn tab_walks_every_group_from_the_context_one() {
        let ctx = HelpContext::Mesh;
        let mut p = HelpPage::Context;
        let mut seen = Vec::new();
        for _ in 0..HelpTopic::ALL.len() {
            p = p.step(ctx, true);
            let HelpPage::Group(t) = p else { panic!() };
            seen.push(t);
        }
        assert_eq!(seen[0], HelpTopic::Treemap, "the group after Mesh");
        assert_eq!(*seen.last().unwrap(), HelpTopic::Mesh, "round to Mesh");
        for t in HelpTopic::ALL {
            assert!(seen.contains(&t), "{t:?}");
        }
        assert_eq!(
            HelpPage::Context.step(HelpContext::Flows, false),
            HelpPage::Group(HelpTopic::Traffic)
        );
        assert_eq!(
            HelpPage::Group(HelpTopic::Everywhere).step(ctx, false),
            HelpPage::Group(HelpTopic::Writes),
            "wraps"
        );
        assert_eq!(HelpPage::Context.toggle_all(), HelpPage::All);
        assert_eq!(HelpPage::All.toggle_all(), HelpPage::Context);
        assert_eq!(HelpPage::Group(HelpTopic::Mesh).toggle_all(), HelpPage::All);
    }
}
