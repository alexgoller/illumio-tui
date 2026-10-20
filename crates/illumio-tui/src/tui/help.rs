//! The `?` help: key lines in titled groups, and the page shown for the
//! context the help was opened from (its group first, then related ones,
//! then the keys that work everywhere). `Tab` / `BackTab` show one group
//! at a time, `a` all of them. Rendering is in `ui::widgets::overlay`.
use crate::tui::model::{Model, Overlay, Screen, TrafficView};

/// Where a help line shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Show {
    /// On every page with its group.
    Always,
    /// Only on a group's own page and the all page (`Tab`, `a`): steps,
    /// costs and other detail that would clutter the context page.
    Detail,
    /// Only on the context page: a compact stand-in for detail lines.
    Brief,
}

/// One help line: its keys (empty: it continues the line above) and text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HelpLine {
    pub key: &'static str,
    pub text: &'static str,
    pub show: Show,
}

const fn key(key: &'static str, text: &'static str) -> HelpLine {
    HelpLine {
        key,
        text,
        show: Show::Always,
    }
}

const fn more(key: &'static str, text: &'static str) -> HelpLine {
    HelpLine {
        key,
        text,
        show: Show::Detail,
    }
}

const fn brief(key: &'static str, text: &'static str) -> HelpLine {
    HelpLine {
        key,
        text,
        show: Show::Brief,
    }
}

/// A titled group of help lines.
pub struct HelpGroup {
    pub title: &'static str,
    pub lines: &'static [HelpLine],
}

/// Every group; `HelpTopic::group` finds one by its title.
pub const HELP_GROUPS: &[HelpGroup] = &[
    HelpGroup {
        title: "Everywhere",
        lines: &[
            key(
                "1 2 3 4 5",
                "Traffic · Policy · Objects · Workloads · Events",
            ),
            brief(":", "command prompt (Tab → Commands)"),
            key("↑ ↓ PgUp PgDn", "move in the current table (also Home End)"),
            brief("s / r a", "sort · search · refresh · auto-refresh"),
            more("s", "cycle sort column and direction"),
            more("/", "search in the current table (Enter keep, Esc clear)"),
            more(
                "Ctrl-U",
                "empty the line being typed (prompt, search, forms)",
            ),
            more("r", "refresh the current screen (bypasses the cache)"),
            more("a", "toggle auto-refresh for the current screen"),
            key("y / n", "send / cancel a shown request"),
            key("Esc", "close overlay / cancel"),
            key("q / Ctrl-C", "quit"),
        ],
    },
    HelpGroup {
        title: "Overlay",
        lines: &[
            key("?", "this help"),
            key("Esc / q", "back"),
            key("Ctrl-C", "quit"),
        ],
    },
    HelpGroup {
        title: "Commands",
        lines: &[
            key(":", "command prompt: :traffic :policy :objects :workloads"),
            key("", ":events  :iplists  :filter <expr>  :window <dur>"),
            key("", ":profile <name>  :draft  :active  :marks  :help  :q"),
            key("", ":unicast  unicast flows only (again: every kind)"),
            key("", ":focus <app>  (alone: a picker of the loaded apps)"),
            key("", ":app <app>  the app overlay (alone: the picker)"),
            key("", ":gen <app>  the policy generator (alone: the picker)"),
            key("", ":readiness  :enforce <app>  (alone: the picker)"),
            key("", ":writes [on|off]  :export[!] <path>  :batch"),
            key("", "(the table on screen as CSV; ! overwrites)"),
        ],
    },
    HelpGroup {
        title: "Traffic",
        lines: &[
            key("v", "cycle Flows · App-to-app · Mesh · Blocked · Treemap"),
            key("f", "edit filter: window src dst svc decision transmission"),
            more("", "in f: , and · | or · ! excludes from the whole field"),
            more("", "svc 8000-8100/tcp · svc:NAME its ports · src: no fqdn:"),
            more(
                "Ctrl-N Ctrl-P",
                "in f: complete the filter fields · cycle back",
            ),
            key("+ / -", "widen (re-runs the window) / narrow (no request)"),
            more("", "window steps 1h 6h 12h 1d 2d 3d 1w 2w 4w 90d"),
            key("B", "re-run a truncated result at a larger cap"),
            more("", "one rung per B: 10000 25000 … 200000; y above 10000"),
            key("] / [", "raise / lower the result cap (re-runs the query)"),
            more("", "cap steps 500 1000 2000 5000 10000 25000 … 200000"),
            key("g", "pick the label keys every view groups by"),
            key(
                "F / X",
                "focus on the row's app (again: other side) · in/out/both",
            ),
            more(
                "",
                "Esc clears; truncated: r re-queries the app (1-2 queries)",
            ),
            key("A", "app overlay: peers, policy, workloads"),
            key(
                "D",
                "draft decision per flow (PCE draft view; again: clear)",
            ),
            more(
                "",
                "1 PUT on the query, polls, 1 download; no policy change",
            ),
            more("", "Esc cancels it; no known query: asks, then re-runs"),
            key("w", "draft allow rule from a blocked flow or row (writes)"),
            key(
                "m",
                "mark the row: squelch · reviewed · critical … (marks.toml)",
            ),
            key("M", "show or hide squelched rows (~)"),
            key("Enter", "drill into a row or section header"),
            more(
                "Units",
                "flows: flow records; conns, bytes: their lifetime counts",
            ),
        ],
    },
    HelpGroup {
        title: "Flows",
        lines: &[
            key(
                "x / X",
                "flat · by src app · by dst app · back (focused: X: direction)",
            ),
            key("Enter", "the flow's detail"),
            key("E", "explain: the rule that allows or blocks the flow"),
            more(
                "",
                "no request; Enter: the rule in Policy (draft); D asks first",
            ),
        ],
    },
    HelpGroup {
        title: "App-to-app",
        lines: &[
            key(
                "x / X",
                "pairs · in · out · both · back (focused: X: direction)",
            ),
            key("o", "pairs by service: one row per src, dst, port/proto"),
            key(
                "d",
                "decision: all · blocked · blocked only (· fixed by draft)",
            ),
        ],
    },
    HelpGroup {
        title: "Blocked",
        lines: &[key(
            "x / X / d",
            "none here: Blocked is App-to-app's in · blocked (v to it)",
        )],
    },
    HelpGroup {
        title: "Sections",
        lines: &[
            key(
                "Space / z",
                "fold a section · fold or unfold all (also Blocked, Flows)",
            ),
            key(
                "s",
                "order: conns ↓ · app A→Z · flows ↓ (App-to-app, Flows)",
            ),
        ],
    },
    HelpGroup {
        title: "Mesh",
        lines: &[key("u / p", "show or hide the user · process columns")],
    },
    HelpGroup {
        title: "Treemap",
        lines: &[key(
            "hjkl Enter b",
            "move · zoom, then drill · conns/bytes · Esc out",
        )],
    },
    HelpGroup {
        title: "Marks",
        lines: &[
            key(
                ":marks [action]",
                "the marks: Enter edit · x delete · e +30d",
            ),
            more("", "(an action: only its marks; Esc / q back)"),
        ],
    },
    HelpGroup {
        title: "App overlay",
        lines: &[
            key(
                "Tab j/k X Enter",
                "panel · row · direction · open (peer: its flows)",
            ),
            key(
                "m w r Esc",
                "mark · draft allow rule · reload panels (asks) · back",
            ),
            key("G", "policy generator: the rules the app's traffic needs"),
            key("E", "explain the row's flow: the rule behind its decision"),
            key("e", "enforce the app: readiness, rules, provision, move"),
        ],
    },
    HelpGroup {
        title: "Enforce",
        lines: &[
            key(
                ":readiness",
                "every app of the result: verdict, coverage, gaps",
            ),
            key(
                "Enter s / D",
                "the app's steps · sort · search · draft decision",
            ),
            key("Tab 1-5 h/l", "steps (also ←/→): Readiness Rules … Watch"),
            key("j/k q Esc", "rows · back (to the list or the app overlay)"),
            key(
                "G w m",
                "Rules: generator · rule for the gap · accept-blocked",
            ),
            key(
                "P",
                "Provision: the app's pending changes (preview; writes)",
            ),
            key(
                "j/k v Enter",
                "Enforce: full/selective · visibility · confirm",
            ),
            more("", "not ready: type enforce first; a change record is kept"),
            key("r R", "Watch: blocked since the move (1 query) · rollback"),
            more(
                "",
                "rollback: re-read, preview, y; changed ones are skipped",
            ),
        ],
    },
    HelpGroup {
        title: "Generator",
        lines: &[
            key(
                "←/→ < > =",
                "granularity · the row's own (*) · back to the slider",
            ),
            key("j/k Tab q", "rows (on into outbound) · section · back"),
            key(
                "Space o R p",
                "check a row · outbound · pick the ruleset · rule list",
            ),
            key("c", "fan-in line: one rule off · Any · All Workloads"),
            key(
                "Enter r Esc",
                "plan the writes, one confirm (writes) · re-query · back",
            ),
        ],
    },
    HelpGroup {
        title: "Forms",
        lines: &[
            key("Ctrl-N Ctrl-P", "complete from the index · cycle back"),
            key(
                "Ruleset",
                "w form: by the destination's scope · Tab/Ctrl-N names",
            ),
            key(
                "Ctrl-A Ctrl-R",
                "w form: all labels · create the typed ruleset",
            ),
        ],
    },
    HelpGroup {
        title: "Policy",
        lines: &[
            key("d", "toggle draft / active"),
            key("p", "pending changes"),
            key(
                "Tab / Enter",
                "pane · rule detail · on a ruleset: load deny rules",
            ),
            key("N", "new deny rule in the selected ruleset (rules pane)"),
            key("t", "enable ↔ disable the rule or ruleset (draft, writes)"),
            key("f / c", "rule search across rulesets (local) · clear it"),
            more(
                "",
                "app=shop,crm ipl:NAME svc:NAME|443/tcp|tcp wl:NAME href:TEXT",
            ),
            more("", "disabled deny · space: all of · Ctrl-N completes"),
            more("", "names: exact or a unique prefix · wl: needs i first"),
            more("", "labels: scope/actors/groups, not exclusions or All"),
            more(
                "Enter",
                "search list: open the rule in its ruleset (f: back)",
            ),
            more(
                "",
                "enabled: the rule's flag, (ruleset off) if its ruleset is",
            ),
            key("i", "rule impact: load the workloads once (1 request)"),
            key("I", "rule impact: the matched workloads, per side (Tab)"),
            more(
                "",
                "/ search · Enter: the workload on Workloads · :export <path>",
            ),
        ],
    },
    HelpGroup {
        title: "Pending",
        lines: &[
            key("j/k Space Enter", "select · mark · diff of a pending entry"),
            key("P", "provision the marked entries, or all (writes)"),
            key("R", "revert marked entries (draft-only ones are deleted)"),
        ],
    },
    HelpGroup {
        title: "Objects",
        lines: &[
            key(
                "Tab / Enter",
                "next tab: labels, groups, services, IP lists · detail",
            ),
            key(
                "f / i / u",
                "Labels: key filter · ignored keys · unused only",
            ),
            more("", "(unused: by the PCE's usage flags)"),
            key(
                "/=text /k=text",
                "Labels: search values only · the values of key k",
            ),
            key(
                "Enter",
                "IP lists: detail: ranges · FQDNs · size · rules using it",
            ),
        ],
    },
    HelpGroup {
        title: "Workloads",
        lines: &[
            key(
                "t/T h H Enter",
                "Traffic as src/dst · unhealthy · health sort · detail",
            ),
            key(
                "E m o u c",
                "enforcement · managed · offline · unlabeled · clear",
            ),
            more("", "local, no request; c clears them and h, not /"),
            key("f", "label filter: app=shop,crm env=prod !role=db app=-"),
            more(
                "",
                "comma: any of · space: all of · ! excludes · =- no label",
            ),
            more("", "Ctrl-N completes; also :filter <expr>; c clears it too"),
            key("x", "group by the g keys (app | env): health per group"),
            key("Space / z", "grouped: fold a group · fold or unfold all"),
            more("", "Space folds on a group header; on a workload it marks"),
            key(
                "Space V Esc",
                "mark the row (●) · the group or all shown · clear",
            ),
            key("L / M", "labels · enforcement of the marked (writes)"),
            more(
                "",
                "none marked: the group under the cursor (header), else the row",
            ),
            more(
                "",
                "marks survive filters, search, sort and x; a sent change clears",
            ),
            key(
                "s / Enter",
                "grouped: size ↓ · name A→Z · unhealthy ↓ · narrow",
            ),
            more("", "Enter on a header sets the label filter to its group"),
        ],
    },
    HelpGroup {
        title: "Events",
        lines: &[
            key(
                "f / Enter / S",
                "filter · event as a change diff · sort by severity",
            ),
            key("J", "in the detail: change diff ↔ raw JSON"),
            key("↑/↓ j/k", "in the detail: prev/next event"),
            key("^d ^u ^e ^y g G", "in the detail: scroll (also PgDn/PgUp)"),
        ],
    },
    HelpGroup {
        title: "Writes",
        lines: &[
            key("n / e / x", "new/edit/delete: Objects, IP lists, Policy"),
            key("X", "Workloads: delete (x groups there)"),
            more("", "with :writes on; each request is shown first (y / n)"),
            more(
                "L / M",
                "Workloads: labels · enforcement of many, one confirm",
            ),
            more(
                "",
                "full/selective on 2+ or any change on 26+: type the count",
            ),
            more(
                "",
                "d on the confirm: draft check (1 Explorer query, asked)",
            ),
            more(
                "",
                "sent: 1 re-read (changed ones skipped), bulk_update by 100",
            ),
        ],
    },
];

/// A group of `HELP_GROUPS`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HelpTopic {
    Everywhere,
    Overlay,
    Commands,
    Traffic,
    Flows,
    AppToApp,
    Blocked,
    Sections,
    Mesh,
    Treemap,
    Marks,
    AppOverlay,
    Generator,
    Enforce,
    Forms,
    Policy,
    Pending,
    Objects,
    Workloads,
    Events,
    Writes,
}

impl HelpTopic {
    /// The order `Tab` walks (and the footer names).
    pub const ALL: [HelpTopic; 21] = [
        HelpTopic::Everywhere,
        HelpTopic::Commands,
        HelpTopic::Traffic,
        HelpTopic::Flows,
        HelpTopic::AppToApp,
        HelpTopic::Blocked,
        HelpTopic::Sections,
        HelpTopic::Mesh,
        HelpTopic::Treemap,
        HelpTopic::Marks,
        HelpTopic::AppOverlay,
        HelpTopic::Generator,
        HelpTopic::Enforce,
        HelpTopic::Forms,
        HelpTopic::Policy,
        HelpTopic::Pending,
        HelpTopic::Objects,
        HelpTopic::Workloads,
        HelpTopic::Events,
        HelpTopic::Writes,
        HelpTopic::Overlay,
    ];

    pub fn title(self) -> &'static str {
        match self {
            HelpTopic::Everywhere => "Everywhere",
            HelpTopic::Overlay => "Overlay",
            HelpTopic::Commands => "Commands",
            HelpTopic::Traffic => "Traffic",
            HelpTopic::Flows => "Flows",
            HelpTopic::AppToApp => "App-to-app",
            HelpTopic::Blocked => "Blocked",
            HelpTopic::Sections => "Sections",
            HelpTopic::Mesh => "Mesh",
            HelpTopic::Treemap => "Treemap",
            HelpTopic::Marks => "Marks",
            HelpTopic::AppOverlay => "App overlay",
            HelpTopic::Generator => "Generator",
            HelpTopic::Enforce => "Enforce",
            HelpTopic::Forms => "Forms",
            HelpTopic::Policy => "Policy",
            HelpTopic::Pending => "Pending",
            HelpTopic::Objects => "Objects",
            HelpTopic::Workloads => "Workloads",
            HelpTopic::Events => "Events",
            HelpTopic::Writes => "Writes",
        }
    }

    /// The group of the same title (a test checks every topic has one).
    pub fn group(self) -> &'static HelpGroup {
        let title = self.title();
        HELP_GROUPS
            .iter()
            .find(|g| g.title == title)
            .unwrap_or(&HELP_GROUPS[0])
    }

    fn position(self) -> usize {
        Self::ALL.iter().position(|t| *t == self).unwrap_or(0)
    }

    /// `Tab`: the next group, wrapping.
    pub fn next(self) -> Self {
        Self::ALL[(self.position() + 1) % Self::ALL.len()]
    }

    /// `BackTab`: the previous group, wrapping.
    pub fn prev(self) -> Self {
        Self::ALL[(self.position() + Self::ALL.len() - 1) % Self::ALL.len()]
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
    EventDetail,
    AppOverlay,
    Generator,
    Pending,
    Marks,
    /// `:readiness` and the enforcement overlay.
    Enforce,
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
            HelpContext::EventDetail => "Event detail",
            HelpContext::AppOverlay => "App overlay",
            HelpContext::Generator => "Generator",
            HelpContext::Pending => "Pending",
            HelpContext::Marks => "Marks",
            HelpContext::Enforce => "Enforce",
        }
    }

    /// The groups of the context page, its own first; a screen's page ends
    /// with `Everywhere`, an overlay's with the short `Overlay`.
    pub fn topics(self) -> &'static [HelpTopic] {
        use HelpTopic as T;
        match self {
            HelpContext::Flows => &[T::Flows, T::Sections, T::Traffic, T::Everywhere],
            HelpContext::AppToApp => &[T::AppToApp, T::Sections, T::Traffic, T::Everywhere],
            HelpContext::Blocked => &[T::Blocked, T::Sections, T::Traffic, T::Everywhere],
            HelpContext::Mesh => &[T::Mesh, T::Traffic, T::Everywhere],
            HelpContext::Treemap => &[T::Treemap, T::Traffic, T::Everywhere],
            HelpContext::Policy => &[T::Policy, T::Pending, T::Writes, T::Everywhere],
            HelpContext::Objects => &[T::Objects, T::Writes, T::Everywhere],
            HelpContext::Workloads => &[T::Workloads, T::Writes, T::Everywhere],
            HelpContext::Events => &[T::Events, T::Everywhere],
            HelpContext::EventDetail => &[T::Events, T::Overlay],
            HelpContext::AppOverlay => &[T::AppOverlay, T::Forms, T::Overlay],
            HelpContext::Generator => &[T::Generator, T::Overlay],
            HelpContext::Pending => &[T::Pending, T::Overlay],
            HelpContext::Marks => &[T::Marks, T::Overlay],
            HelpContext::Enforce => &[T::Enforce, T::Overlay],
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
            Overlay::EventDetail { .. } => return HelpContext::EventDetail,
            Overlay::Pending => return HelpContext::Pending,
            Overlay::Marks => return HelpContext::Marks,
            Overlay::Readiness | Overlay::Enforce => return HelpContext::Enforce,
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

    /// Whether a line shows on this page.
    fn shows(self, line: &HelpLine) -> bool {
        match (self, line.show) {
            (_, Show::Always) => true,
            (HelpPage::Context, s) => s == Show::Brief,
            (_, s) => s == Show::Detail,
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
        rows.extend(
            g.lines
                .iter()
                .filter(|l| page.shows(l))
                .map(|l| HelpRow::Key(l.key, l.text)),
        );
    }
    rows
}

/// The key column of `rows`: the longest key and a gap, at most 24.
pub fn key_width(rows: &[HelpRow]) -> usize {
    let widest = rows
        .iter()
        .map(|r| match r {
            HelpRow::Key(k, _) => k.chars().count(),
            _ => 0,
        })
        .max()
        .unwrap_or(0);
    (widest + 2).min(24)
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

/// `prefix` and as many `names` as fit in `width` columns, joined by ` · `;
/// the rest counted as ` +N`. A name is never cut; `None` when not even
/// the first fits.
pub fn fit_names(prefix: &str, names: &[&str], width: usize) -> Option<String> {
    let mut out = prefix.to_string();
    for (i, n) in names.iter().enumerate() {
        let sep = if i == 0 { "" } else { " · " };
        let left = names.len() - i - 1;
        let tail = if left > 0 {
            format!(" +{left}")
        } else {
            String::new()
        };
        let len = out.chars().count() + sep.chars().count() + n.chars().count();
        if len + tail.chars().count() > width {
            if i == 0 {
                return None;
            }
            out.push_str(&format!(" +{}", names.len() - i));
            return Some(out);
        }
        out.push_str(sep);
        out.push_str(n);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_topic_has_its_group() {
        assert_eq!(HELP_GROUPS.len(), HelpTopic::ALL.len());
        for t in HelpTopic::ALL {
            let g = t.group();
            assert_eq!(g.title, t.title());
            assert!(!g.lines.is_empty(), "{}", g.title);
            assert_ne!(g.lines[0].key, "", "a group starts with a key");
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
        ("F / X", "focus on the row's app"),
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
        // Split: Enter's flow detail moved to the Flows group.
        ("Enter", "drill into a row or section header"),
        ("x / X", "pairs · in · out · both · back (focused: X"),
        ("o", "pairs by service: one row per src, dst, port/proto"),
        ("d", "all · blocked · blocked only (· fixed by draft)"),
        ("x / X", "flat · by src app · by dst app · back (focused: X"),
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
        ("Ruleset", "the destination's scope · Tab/Ctrl-N names"),
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
        ("f / i / u", "key filter · ignored keys · unused only"),
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
        // Was one continuation line; the scroll keys are a line of their own.
        ("↑/↓ j/k", "in the detail: prev/next event"),
        ("n / e / x", "new/edit/delete: Objects, IP lists, Policy"),
        ("j/k Space Enter", "select · mark · diff of a pending entry"),
        ("P", "provision the marked entries, or all (writes)"),
        ("R", "revert marked entries (draft-only ones are deleted)"),
        ("y / n", "send / cancel a shown request"),
        ("Esc", "close overlay / cancel"),
    ];

    #[test]
    fn every_old_help_line_is_in_a_group() {
        assert_eq!(OLD_LINES.len(), 79);
        let all: Vec<&HelpLine> = HELP_GROUPS.iter().flat_map(|g| g.lines).collect();
        for (k, d) in OLD_LINES {
            assert!(
                all.iter()
                    .any(|l| l.key == *k && l.show != Show::Brief && l.text.contains(d)),
                "{k:?} {d:?}"
            );
        }
        // The prefixes are gone: a key is just the keys.
        for l in &all {
            for p in ["Traffic  ", "Generate ", "App      ", "Policy   ", "w form"] {
                assert!(!l.key.starts_with(p), "{}", l.key);
            }
        }
    }

    const CONTEXTS: [(HelpContext, &str); 15] = [
        (HelpContext::Flows, "Flows"),
        (HelpContext::AppToApp, "App-to-app"),
        (HelpContext::Mesh, "Mesh"),
        (HelpContext::Blocked, "Blocked"),
        (HelpContext::Treemap, "Treemap"),
        (HelpContext::Policy, "Policy"),
        (HelpContext::Objects, "Objects"),
        (HelpContext::Workloads, "Workloads"),
        (HelpContext::Events, "Events"),
        (HelpContext::EventDetail, "Events"),
        (HelpContext::AppOverlay, "App overlay"),
        (HelpContext::Generator, "Generator"),
        (HelpContext::Pending, "Pending"),
        (HelpContext::Marks, "Marks"),
        (HelpContext::Enforce, "Enforce"),
    ];

    #[test]
    fn pages_put_the_context_first_and_the_shared_keys_last() {
        use HelpContext as C;
        for (ctx, first) in CONTEXTS {
            let rows = page_rows(HelpPage::Context, ctx);
            assert_eq!(rows[0], HelpRow::Heading(first), "{ctx:?}");
            let last_heading = rows.iter().rev().find_map(|r| match r {
                HelpRow::Heading(h) => Some(*h),
                _ => None,
            });
            let overlay = matches!(
                ctx,
                C::EventDetail | C::AppOverlay | C::Generator | C::Pending | C::Marks | C::Enforce
            );
            let shared = if overlay { "Overlay" } else { "Everywhere" };
            assert_eq!(last_heading, Some(shared), "{ctx:?}");
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

    /// The context page is short: no detail lines, at most 34 rows.
    #[test]
    fn context_pages_are_short_and_leave_detail_out() {
        let details: Vec<(&str, &str)> = HELP_GROUPS
            .iter()
            .flat_map(|g| g.lines)
            .filter(|l| l.show == Show::Detail)
            .map(|l| (l.key, l.text))
            .collect();
        for (ctx, _) in CONTEXTS {
            let rows = page_rows(HelpPage::Context, ctx);
            assert!(rows.len() <= 34, "{ctx:?}: {} rows", rows.len());
            for r in &rows {
                if let HelpRow::Key(k, t) = r {
                    assert!(!details.contains(&(*k, *t)), "{ctx:?}: {t}");
                }
            }
        }
        let flows = page_rows(HelpPage::Context, HelpContext::Flows);
        assert!(flows.contains(&HelpRow::Key(":", "command prompt (Tab → Commands)")));
        assert!(!flows.contains(&HelpRow::Heading("Marks")));
        assert!(!flows.iter().any(|r| matches!(r, HelpRow::Key("?", _))));
        // A group page has the detail and not the brief stand-ins.
        let traffic = page_rows(HelpPage::Group(HelpTopic::Traffic), HelpContext::Flows);
        assert!(
            traffic
                .iter()
                .any(|r| matches!(r, HelpRow::Key("Units", _)))
        );
        let every = page_rows(HelpPage::Group(HelpTopic::Everywhere), HelpContext::Flows);
        assert!(every.iter().any(|r| matches!(r, HelpRow::Key("r", _))));
        assert!(
            !every
                .iter()
                .any(|r| matches!(r, HelpRow::Key("s / r a", _)))
        );
    }

    /// Every line of a context page fits an 80-column terminal uncut.
    #[test]
    fn context_pages_fit_80_columns() {
        for (ctx, _) in CONTEXTS {
            let rows = page_rows(HelpPage::Context, ctx);
            let kw = key_width(&rows);
            for r in &rows {
                if let HelpRow::Key(_, t) = r {
                    assert!(kw + t.chars().count() <= 78, "{ctx:?}: {t}");
                }
            }
        }
    }

    #[test]
    fn fit_names_never_cuts_a_name() {
        let names = ["Commands", "Mesh", "Treemap"];
        assert_eq!(
            fit_names("more: ", &names, 80).as_deref(),
            Some("more: Commands · Mesh · Treemap")
        );
        assert_eq!(
            fit_names("more: ", &names, 25).as_deref(),
            Some("more: Commands · Mesh +1")
        );
        assert_eq!(
            fit_names("more: ", &names, 17).as_deref(),
            Some("more: Commands +2")
        );
        assert_eq!(fit_names("more: ", &names, 10), None);
        for w in 0..40 {
            if let Some(s) = fit_names("more: ", &names, w) {
                assert!(s.chars().count() <= w, "{w}: {s}");
            }
        }
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
            HelpPage::Group(HelpTopic::Overlay),
            "wraps"
        );
        assert_eq!(HelpPage::Context.toggle_all(), HelpPage::All);
        assert_eq!(HelpPage::All.toggle_all(), HelpPage::Context);
        assert_eq!(HelpPage::Group(HelpTopic::Mesh).toggle_all(), HelpPage::All);
    }
}
