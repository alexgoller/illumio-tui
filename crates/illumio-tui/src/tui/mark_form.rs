//! The `m` form (`Overlay::MarkForm`): the generic `Form` for its fields,
//! keys, completion, plus what a mark needs around it: where it came from,
//! the mark it edits, each side's kinds for `Ctrl-T`, the live preview.
//! Pure: `update::marks` opens, keys and saves it; a save writes only
//! `marks.toml`, never the PCE.
use crate::tui::complete::{ANY_PCE, THIS_PCE};
use crate::tui::edit::EditIntent;
use crate::tui::form::{Field, FieldKind, Form};
use pce::marks::{
    Action, Mark, PceId, PceScope, ServiceSpec, Side, parse_decision, parse_expiry, parse_side,
    side_text,
};
use pce::models::PolicyDecision;
use pce::traffic::{Decisions, Direction, Focus, MeshColumns, ResolvedEndpoint};
use std::collections::BTreeMap;

/// The fields, by position.
pub const ACTION: usize = 0;
pub const SRC: usize = 1;
pub const DST: usize = 2;
pub const SERVICE: usize = 3;
pub const DECISION: usize = 4;
pub const DRAFT: usize = 5;
pub const EXPIRES: usize = 6;
pub const SCOPE: usize = 7;
pub const COMMENT: usize = 8;

/// Where `Esc` and a save return to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarkOrigin {
    Traffic,
    App,
    /// The `:marks` overlay (an edit or an extend).
    Marks,
}

/// The view the preview counts rows in: the rows of the view the form came
/// from that hold a matching flow.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RowsOf {
    Flows,
    Pairs(Decisions),
    Services(Decisions),
    Sections(Option<Direction>, Decisions),
    Mesh(MeshColumns),
    Peers(Box<Focus>, Direction),
}

/// One side of the prefill: the text, `Ctrl-A`'s alternate (every label
/// of the row's endpoint), `Ctrl-T`'s kinds in cycle order, a note.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SidePrefill {
    pub text: String,
    pub alt: Option<String>,
    pub kinds: Vec<String>,
    pub note: Option<&'static str>,
}

/// What `m` fills the form with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prefill {
    /// `App-to-app in · shop/prod ← monitoring/prod 5666/tcp`.
    pub what: String,
    pub action: Action,
    pub src: SidePrefill,
    pub dst: SidePrefill,
    pub service: String,
    pub decision: String,
    pub draft: String,
    pub rows: RowsOf,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MarkCtx {
    pub origin: MarkOrigin,
    /// The mark edited (`:marks` Enter) or extended (`e`); `None`: new.
    pub editing: Option<Mark>,
    /// `e`: only the comment is asked; the save adds 30 days.
    pub extend: bool,
    /// Expires follows the action's default until the user edits it.
    pub expiry_edited: bool,
    /// `Ctrl-T`: each side's kinds (source, destination).
    pub kinds: [Vec<String>; 2],
    pub rows: RowsOf,
    /// The profile's host: `this PCE`.
    pub host: String,
    /// The profile's org, when editing: a mark limited to another org of
    /// this host is not `this PCE`.
    pub org: Option<u64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MarkForm {
    pub form: Form,
    pub ctx: MarkCtx,
    /// The live preview's lines (`matches 3 rows · 41 flows …`).
    pub preview: Vec<String>,
}

fn quote_side(labels: BTreeMap<String, Vec<String>>) -> String {
    side_text(&Side::Labels(labels))
}

/// `key=value` for each of `keys` the endpoint has, in the side grammar.
fn labels_under(ep: &ResolvedEndpoint, keys: &[String]) -> String {
    let m: BTreeMap<String, Vec<String>> = keys
        .iter()
        .filter_map(|k| ep.label_value(k).map(|v| (k.clone(), vec![v.to_string()])))
        .collect();
    if m.is_empty() {
        String::new()
    } else {
        quote_side(m)
    }
}

/// Every resolved label of the endpoint (unresolved `?` left out).
fn all_labels(ep: &ResolvedEndpoint) -> String {
    let m: BTreeMap<String, Vec<String>> = ep
        .labels
        .iter()
        .filter(|l| l.key != "?")
        .map(|l| (l.key.clone(), vec![l.value.clone()]))
        .collect();
    if m.is_empty() {
        String::new()
    } else {
        quote_side(m)
    }
}

/// The kinds an endpoint allows, in `Ctrl-T` order: labels, `wl:`, `ip:`,
/// `ipl:`, then `any`; `labels` first when given.
fn endpoint_kinds(ep: Option<&ResolvedEndpoint>, labels: &str) -> Vec<String> {
    let mut k = Vec::new();
    if !labels.is_empty() {
        k.push(labels.to_string());
    }
    if let Some(ep) = ep {
        if ep.is_workload()
            && let Some(h) = ep.hostname.as_deref().or(ep.name.as_deref())
        {
            k.push(format!("wl:{h}"));
        }
        if let Some(ip) = &ep.ip {
            k.push(format!("ip:{ip}"));
        }
        if let Some(l) = ep.ip_lists.first() {
            k.push(format!("ipl:{l}"));
        }
    }
    k.push("any".into());
    k.dedup();
    k
}

/// A flow's endpoint: the labels under the keys in force it has, else
/// `wl:<host>`; a non-workload its first reported IP list, else `ip:`; an
/// FQDN alone `any` (an FQDN cannot be marked).
pub fn endpoint_side(ep: &ResolvedEndpoint, keys: &[String]) -> SidePrefill {
    let labels = if ep.is_workload() {
        labels_under(ep, keys)
    } else {
        String::new()
    };
    let all = all_labels(ep);
    let kinds = endpoint_kinds(Some(ep), &labels);
    let (text, note) = if !labels.is_empty() {
        (labels.clone(), None)
    } else if let Some(h) = ep
        .is_workload()
        .then(|| ep.hostname.as_deref().or(ep.name.as_deref()))
        .flatten()
    {
        (format!("wl:{h}"), None)
    } else if let Some(l) = ep.ip_lists.first() {
        (format!("ipl:{l}"), None)
    } else if let Some(ip) = &ep.ip {
        (format!("ip:{ip}"), None)
    } else {
        ("any".to_string(), Some("fqdn cannot be marked"))
    };
    SidePrefill {
        alt: (ep.is_workload() && !all.is_empty() && all != text).then_some(all),
        text,
        kinds,
        note,
    }
}

/// A group key (pairs, sections, mesh, the app overlay): each value under
/// its key, `-` dropped (a mark cannot require a missing key); `ipl:X` as
/// is; a name group (an IP, an unlabelled host) `ip:` when it parses as
/// an address, else `wl:`. `ep` is one of the row's endpoints: `Ctrl-A`
/// and `Ctrl-T` offer its labels and kinds.
pub fn group_side(group: &[String], keys: &[String], ep: Option<&ResolvedEndpoint>) -> SidePrefill {
    let labelled = ep.is_some_and(|e| e.is_workload() && !e.labels.is_empty());
    // An unlabelled endpoint (an IP, an IP list, an unlabelled host, an
    // FQDN) is its own side, as on the Flows view.
    if let Some(e) = ep.filter(|_| !labelled) {
        return endpoint_side(e, keys);
    }
    let first = group.first().map(String::as_str).unwrap_or("");
    let text = if let Some(e) = ep.filter(|_| labelled) {
        let m: BTreeMap<String, Vec<String>> = keys
            .iter()
            .zip(group)
            .filter(|(_, v)| v.as_str() != "-")
            .map(|(k, v)| (k.clone(), vec![v.clone()]))
            .collect();
        if m.is_empty() {
            // No key in force: the endpoint itself (`wl:<host>`, `Ctrl-A`
            // its labels), never `any`, which is wider than the row.
            return endpoint_side(e, &[]);
        }
        quote_side(m)
    } else if let Some(l) = first.strip_prefix("ipl:") {
        format!("ipl:{l}")
    } else if first.parse::<std::net::IpAddr>().is_ok() {
        format!("ip:{first}")
    } else if first.is_empty() || first == "-" {
        "any".to_string()
    } else {
        format!("wl:{first}")
    };
    let all = ep.map(all_labels).unwrap_or_default();
    let mut kinds = endpoint_kinds(ep, if labelled { &text } else { "" });
    if !kinds.contains(&text) {
        kinds.insert(0, text.clone());
    }
    SidePrefill {
        alt: (labelled && !all.is_empty() && all != text).then_some(all),
        text,
        kinds,
        note: None,
    }
}

/// The side `any`, for the peer side of a section header.
pub fn any_side() -> SidePrefill {
    SidePrefill {
        text: "any".into(),
        alt: None,
        kinds: vec!["any".into()],
        note: None,
    }
}

/// A decision filter as the Decision field writes it.
pub fn decisions_text(d: Decisions) -> (String, String) {
    match d {
        Decisions::All => (String::new(), String::new()),
        Decisions::Blocked => ("blocked, potentially_blocked".into(), String::new()),
        Decisions::BlockedOnly => ("blocked".into(), String::new()),
        Decisions::FixedByDraft => ("blocked, potentially_blocked".into(), "allowed".into()),
    }
}

/// The text of an expiry in the Expires field: `never`, `2027-01-15` for
/// the end of a UTC day, else RFC 3339.
pub fn expires_text(e: Option<jiff::Timestamp>) -> String {
    match e {
        None => "never".into(),
        Some(t) if t.strftime("%H:%M:%S").to_string() == "23:59:59" => pce::marks::expiry_date(t),
        Some(t) => t.to_string(),
    }
}

fn decision_list(d: &[PolicyDecision]) -> String {
    d.iter().map(|d| d.as_str()).collect::<Vec<_>>().join(", ")
}

/// `this PCE` only for this host and org; another org of this host is
/// `host (org N)`, as `scope_of` reads it back.
fn scope_text(scope: Option<&PceScope>, pce: &PceId) -> String {
    match scope {
        None => ANY_PCE.into(),
        Some(s) if s.covers(pce) => THIS_PCE.into(),
        Some(PceScope { host, org: Some(o) }) => format!("{host} (org {o})"),
        Some(s) => s.host.clone(),
    }
}

/// `host (org N)`: the host and the org.
fn host_org(v: &str) -> Option<(String, u64)> {
    let (host, rest) = v.split_once(" (org ")?;
    let org = rest.strip_suffix(')')?.trim().parse().ok()?;
    Some((host.trim().to_string(), org))
}

/// A row's service as a mark takes it: a port on a protocol without
/// ports (`1234/47`, `0/icmp` as reported) becomes the protocol alone.
pub fn service_prefill(service: &str) -> String {
    let t = service.trim();
    if t.is_empty() || ServiceSpec::parse(t).is_ok() {
        return t.to_string();
    }
    match t.split_once('/') {
        Some((_, proto)) if ServiceSpec::parse(proto).is_ok() => proto.trim().to_string(),
        _ => t.to_string(),
    }
}

/// The fields, filled.
#[allow(clippy::too_many_arguments)]
fn fields(
    action: Action,
    src: &SidePrefill,
    dst: &SidePrefill,
    service: &str,
    decision: &str,
    draft: &str,
    expires: &str,
    scope: &str,
    comment: &str,
    host: &str,
) -> Vec<Field> {
    let side_hint = |p: &SidePrefill| -> String {
        let mut h = "Ctrl-A all labels · Ctrl-T kind".to_string();
        if let Some(n) = p.note {
            h = format!("{n} · {h}");
        }
        h
    };
    vec![
        Field::new(
            "Action",
            action.as_str(),
            "Ctrl-N: critical important revisit…",
            true,
        )
        .kind(FieldKind::MarkAction),
        Field::new("Source", src.text.clone(), side_hint(src), true).kind(FieldKind::MarkSide),
        Field::new("Destination", dst.text.clone(), side_hint(dst), true).kind(FieldKind::MarkSide),
        Field::new(
            "Service",
            service,
            "empty: any · 8000-8010/tcp · udp",
            false,
        ),
        Field::new("Decision", decision, "empty: any · blocked …", false)
            .kind(FieldKind::Decisions),
        Field::new("Draft", draft, "empty: any · matched with D loaded", false)
            .kind(FieldKind::Decisions),
        Field::new("Expires", expires, "1d 7d 30d 90d YYYY-MM-DD never", false)
            .kind(FieldKind::MarkExpiry),
        Field::new(
            "Scope",
            scope,
            format!("this PCE = {host} · Ctrl-N: any PCE"),
            false,
        )
        .kind(FieldKind::MarkScope),
        Field::new(
            "Comment",
            comment,
            "why: CHG-1182, owner confirmed (required)",
            false,
        ),
    ]
}

impl MarkForm {
    /// A new mark from a row: the action's default expiry, this PCE.
    pub fn new(p: Prefill, origin: MarkOrigin, host: &str) -> Self {
        let mut form = Form::new(
            format!("New mark · {}", p.what),
            fields(
                p.action,
                &p.src,
                &p.dst,
                &service_prefill(&p.service),
                &p.decision,
                &p.draft,
                p.action.default_expiry(),
                THIS_PCE,
                "",
                host,
            ),
            EditIntent::Mark,
        );
        let alt: Vec<(usize, String)> = [(SRC, &p.src), (DST, &p.dst)]
            .into_iter()
            .filter_map(|(i, s)| s.alt.clone().map(|a| (i, a)))
            .collect();
        form.alt = (!alt.is_empty()).then_some(alt);
        form.active = COMMENT;
        Self {
            form,
            ctx: MarkCtx {
                origin,
                editing: None,
                extend: false,
                expiry_edited: false,
                kinds: [p.src.kinds, p.dst.kinds],
                rows: p.rows,
                host: host.to_string(),
                org: None,
            },
            preview: Vec::new(),
        }
    }

    /// The mark `m` edits (`:marks` Enter): its fields, its comment
    /// offered for change. `extend` (`e`): the comment only, prefilled.
    pub fn edit(m: &Mark, pce: &PceId, extend: bool) -> Self {
        let host = pce.host.as_str();
        let side = |s: &Side| SidePrefill {
            text: side_text(s),
            alt: None,
            kinds: vec![side_text(s), "any".into()],
            note: None,
        };
        let (src, dst) = (side(&m.src), side(&m.dst));
        let all = fields(
            m.action,
            &src,
            &dst,
            &m.service_text().unwrap_or_default(),
            &decision_list(&m.decisions),
            &decision_list(&m.draft_decisions),
            &expires_text(m.expires),
            &scope_text(m.scope.as_ref(), pce),
            &m.comment,
            host,
        );
        let (title, fields, active) = if extend {
            (
                format!("Extend {} by 30 days · why is it still needed?", m.id),
                vec![all[COMMENT].clone()],
                0,
            )
        } else {
            (format!("Edit mark {}", m.id), all, COMMENT)
        };
        let mut form = Form::new(title, fields, EditIntent::Mark);
        form.active = active;
        Self {
            form,
            ctx: MarkCtx {
                origin: MarkOrigin::Marks,
                editing: Some(m.clone()),
                extend,
                expiry_edited: true,
                kinds: [src.kinds, dst.kinds],
                rows: RowsOf::Flows,
                host: host.to_string(),
                org: Some(pce.org),
            },
            preview: Vec::new(),
        }
    }

    fn value(&self, i: usize) -> &str {
        self.form.fields.get(i).map_or("", |f| f.value.trim())
    }

    /// `Ctrl-T` on a side: the next kind the row allows (labels, `wl:`,
    /// `ip:`, `ipl:`, `any`); elsewhere nothing.
    pub fn cycle_kind(&mut self) {
        let side = match self.form.active {
            SRC => 0,
            DST => 1,
            _ => return,
        };
        let kinds = &self.ctx.kinds[side];
        if kinds.is_empty() {
            return;
        }
        let at = kinds
            .iter()
            .position(|k| k == self.form.fields[self.form.active].value.trim());
        let next = kinds[at.map_or(0, |i| (i + 1) % kinds.len())].clone();
        self.form.fields[self.form.active].value = next;
    }

    /// `Ctrl-A` on a side: its text and every label of the row's endpoint
    /// trade places (the other side stays); elsewhere nothing.
    pub fn swap_labels(&mut self) {
        let at = self.form.active;
        if let Some((_, other)) = self
            .form
            .alt
            .as_mut()
            .and_then(|alt| alt.iter_mut().find(|(i, _)| *i == at))
        {
            std::mem::swap(&mut self.form.fields[at].value, other);
        }
    }

    /// After a key: Expires follows the action's default until edited.
    pub fn sync_expiry(&mut self, expiry_before: &str) {
        if self.ctx.extend {
            return;
        }
        if self.value(EXPIRES) != expiry_before.trim() {
            self.ctx.expiry_edited = true;
        }
        if !self.ctx.expiry_edited
            && let Some(a) = Action::parse(self.value(ACTION))
        {
            self.form.fields[EXPIRES].value = a.default_expiry().into();
        }
    }

    /// The comment as typed.
    pub fn comment(&self) -> &str {
        if self.ctx.extend {
            self.value(0)
        } else {
            self.value(COMMENT)
        }
    }

    /// The Scope field: `any PCE` (or empty), `this PCE` (the edited
    /// mark's org kept when it is this org), `host (org N)`, or a host (the
    /// edited mark's org kept on the same host). Whole words only: a host
    /// named `anyhost` is a host.
    fn scope_of(&self, old: Option<&Mark>) -> Option<PceScope> {
        let v = self.value(SCOPE);
        let word = v.to_ascii_lowercase();
        let old = old.and_then(|m| m.scope.as_ref());
        if word.is_empty() || word == "any" || word == ANY_PCE.to_ascii_lowercase() {
            return None;
        }
        if word == "this" || word == THIS_PCE.to_ascii_lowercase() {
            let org = old
                .filter(|s| s.host.eq_ignore_ascii_case(&self.ctx.host))
                .and_then(|s| s.org)
                .filter(|o| Some(*o) == self.ctx.org);
            return Some(PceScope {
                host: self.ctx.host.clone(),
                org,
            });
        }
        if let Some((host, org)) = host_org(v) {
            return Some(PceScope {
                host,
                org: Some(org),
            });
        }
        let org = old
            .filter(|s| s.host.eq_ignore_ascii_case(v))
            .and_then(|s| s.org);
        Some(PceScope {
            host: v.to_string(),
            org,
        })
    }

    /// The mark the fields describe, checked as the file checks one; the
    /// first problem names its field. The edited mark keeps its id, org
    /// (on the same PCE) and creation.
    pub fn mark(&self, now: jiff::Timestamp) -> Result<Mark, String> {
        if self.ctx.extend {
            return self
                .ctx
                .editing
                .clone()
                .ok_or_else(|| "internal: nothing to extend".to_string());
        }
        let field = |name: &str, e: String| format!("{name}: {e}");
        let action = Action::parse(self.value(ACTION)).ok_or_else(|| {
            field(
                "Action",
                format!("{:?} is none of: {}", self.value(ACTION), Action::names()),
            )
        })?;
        let src = parse_side(self.value(SRC)).map_err(|e| field("Source", e))?;
        let dst = parse_side(self.value(DST)).map_err(|e| field("Destination", e))?;
        let services = self
            .value(SERVICE)
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| ServiceSpec::parse(s).map_err(|e| field("Service", e)))
            .collect::<Result<Vec<_>, _>>()?;
        let decisions = |name: &str, text: &str| {
            text.split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty() && !s.eq_ignore_ascii_case("all"))
                .map(|s| parse_decision(s).map_err(|e| field(name, e)))
                .collect::<Result<Vec<_>, _>>()
        };
        let expires = parse_expiry(
            if self.value(EXPIRES).is_empty() {
                "never"
            } else {
                self.value(EXPIRES)
            },
            now,
        )
        .map_err(|e| field("Expires", e))?;
        let old = self.ctx.editing.as_ref();
        let scope = self.scope_of(old);
        let m = Mark {
            id: old.map(|m| m.id.clone()).unwrap_or_default(),
            action,
            src,
            dst,
            services,
            decisions: decisions("Decision", self.value(DECISION))?,
            draft_decisions: decisions("Draft", self.value(DRAFT))?,
            scope,
            expires,
            comment: self.value(COMMENT).to_string(),
            created: old.and_then(|m| m.created),
            created_on: old
                .and_then(|m| m.created_on.clone())
                .or_else(|| Some(self.ctx.host.clone())),
        };
        if let Some(p) = m.problems().into_iter().next() {
            return Err(p);
        }
        Ok(m)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pce::resolve::ResolvedLabel;

    fn wl(host: &str, labels: &[(&str, &str)]) -> ResolvedEndpoint {
        ResolvedEndpoint {
            ip: Some("10.0.1.10".into()),
            hostname: Some(host.into()),
            workload_href: Some(format!("/orgs/5/workloads/{host}")),
            labels: labels
                .iter()
                .map(|(k, v)| ResolvedLabel {
                    href: format!("/orgs/5/labels/{k}{v}"),
                    key: (*k).into(),
                    value: (*v).into(),
                })
                .collect(),
            ..Default::default()
        }
    }

    fn keys() -> Vec<String> {
        vec!["app".into(), "env".into()]
    }

    fn now() -> jiff::Timestamp {
        "2026-10-17T12:00:00Z".parse().unwrap()
    }

    #[test]
    fn a_workload_endpoint_gives_the_labels_under_the_keys_and_its_kinds() {
        let ep = wl(
            "web-01",
            &[("app", "shop"), ("env", "prod"), ("role", "web")],
        );
        let p = endpoint_side(&ep, &keys());
        assert_eq!(p.text, "app=shop env=prod");
        assert_eq!(p.alt.as_deref(), Some("app=shop env=prod role=web"));
        assert_eq!(
            p.kinds,
            vec!["app=shop env=prod", "wl:web-01", "ip:10.0.1.10", "any"]
        );
        let bare = wl("nas", &[]);
        assert_eq!(endpoint_side(&bare, &keys()).text, "wl:nas");
        let corp = ResolvedEndpoint {
            ip: Some("10.1.4.9".into()),
            ip_lists: vec!["Corp".into()],
            ..Default::default()
        };
        assert_eq!(endpoint_side(&corp, &keys()).text, "ipl:Corp");
        let fqdn = ResolvedEndpoint {
            fqdn: Some("x.example".into()),
            ..Default::default()
        };
        let f = endpoint_side(&fqdn, &keys());
        assert_eq!(
            (f.text.as_str(), f.note),
            ("any", Some("fqdn cannot be marked"))
        );
    }

    #[test]
    fn a_group_key_drops_missing_keys_and_names_ip_lists_and_hosts() {
        let ep = wl("web-01", &[("app", "shop"), ("role", "web")]);
        let g = |v: &[&str], ep: Option<&ResolvedEndpoint>| {
            let v: Vec<String> = v.iter().map(|s| s.to_string()).collect();
            group_side(&v, &keys(), ep).text
        };
        assert_eq!(g(&["shop", "-"], Some(&ep)), "app=shop");
        assert_eq!(g(&["ipl:Corp", "-"], None), "ipl:Corp");
        assert_eq!(g(&["10.0.3.30", "-"], None), "ip:10.0.3.30");
        assert_eq!(g(&["unmanaged-nas", "-"], None), "wl:unmanaged-nas");
    }

    #[test]
    fn a_group_without_keys_is_its_endpoint_and_an_fqdn_is_never_a_host() {
        let ep = wl("web-01", &[("role", "web")]);
        let v: Vec<String> = vec!["-".into(), "-".into()];
        let p = group_side(&v, &keys(), Some(&ep));
        assert_eq!(p.text, "wl:web-01", "not any: the row is narrower");
        assert_eq!(p.alt.as_deref(), Some("role=web"));
        let fqdn = ResolvedEndpoint {
            fqdn: Some("x.example".into()),
            ..Default::default()
        };
        let g = group_side(&["x.example".to_string()], &keys(), Some(&fqdn));
        assert_eq!(
            (g.text.as_str(), g.note),
            ("any", Some("fqdn cannot be marked"))
        );
        let corp = ResolvedEndpoint {
            ip: Some("10.1.4.9".into()),
            ip_lists: vec!["Corp".into()],
            ..Default::default()
        };
        assert_eq!(
            group_side(&["ipl:Corp".to_string()], &keys(), Some(&corp)).text,
            "ipl:Corp"
        );
    }

    fn form() -> MarkForm {
        let ep = wl(
            "web-01",
            &[("app", "shop"), ("env", "prod"), ("role", "web")],
        );
        let db = wl("db-01", &[("app", "shop"), ("env", "prod"), ("role", "db")]);
        MarkForm::new(
            Prefill {
                what: "Flows · web-01 → db-01 5432/tcp".into(),
                action: Action::Reviewed,
                src: endpoint_side(&ep, &keys()),
                dst: endpoint_side(&db, &keys()),
                service: "5432/tcp".into(),
                decision: String::new(),
                draft: String::new(),
                rows: RowsOf::Flows,
            },
            MarkOrigin::Traffic,
            "pce.example",
        )
    }

    #[test]
    fn the_form_builds_the_mark_it_shows_and_names_the_bad_field() {
        let mut f = form();
        assert_eq!(f.form.active, COMMENT, "the cursor waits on the comment");
        assert_eq!(f.form.value("Expires"), "30d", "reviewed's default");
        f.form.fields[COMMENT].value = "checked with the owner".into();
        let m = f.mark(now()).unwrap();
        assert_eq!(m.action, Action::Reviewed);
        assert_eq!(side_text(&m.src), "app=shop env=prod");
        assert_eq!(m.services[0].text(), "5432/tcp");
        assert_eq!(
            m.scope,
            Some(PceScope {
                host: "pce.example".into(),
                org: None
            })
        );
        assert_eq!(
            m.expires,
            Some(now() + jiff::SignedDuration::from_hours(24 * 30))
        );
        assert_eq!(m.created_on.as_deref(), Some("pce.example"));
        f.form.fields[SERVICE].value = "70000/tcp".into();
        assert!(f.mark(now()).unwrap_err().starts_with("Service: "));
        f.form.fields[SERVICE].value = String::new();
        f.form.fields[SRC].value = "wl:web-01".into();
        f.form.fields[SCOPE].value = ANY_PCE.into();
        assert!(f.mark(now()).unwrap_err().contains("workload needs pce"));
    }

    #[test]
    fn expires_follows_the_action_until_edited_and_ctrl_t_cycles_kinds() {
        let mut f = form();
        f.form.fields[ACTION].value = "squelch".into();
        f.sync_expiry("30d");
        assert_eq!(f.form.value("Expires"), "90d");
        f.form.fields[EXPIRES].value = "7d".into();
        f.sync_expiry("90d");
        f.form.fields[ACTION].value = "critical".into();
        f.sync_expiry("7d");
        assert_eq!(f.form.value("Expires"), "7d", "edited: kept");
        f.form.active = SRC;
        f.cycle_kind();
        assert_eq!(f.form.value("Source"), "wl:web-01");
        f.cycle_kind();
        f.cycle_kind();
        assert_eq!(f.form.value("Source"), "any");
        f.cycle_kind();
        assert_eq!(f.form.value("Source"), "app=shop env=prod", "around");
    }

    #[test]
    fn ctrl_a_swaps_one_side_for_every_label() {
        let mut f = form();
        f.form.active = DST;
        f.swap_labels();
        assert_eq!(f.form.value("Destination"), "app=shop env=prod role=db");
        assert_eq!(
            f.form.value("Source"),
            "app=shop env=prod",
            "the other side stays"
        );
        f.swap_labels();
        assert_eq!(f.form.value("Destination"), "app=shop env=prod");
        f.form.active = SERVICE;
        f.swap_labels();
        assert_eq!(f.form.value("Service"), "5432/tcp", "not a side: nothing");
    }

    fn pce(org: u64) -> PceId {
        PceId {
            host: "pce.example".into(),
            org,
        }
    }

    #[test]
    fn a_port_on_a_protocol_without_ports_prefills_the_protocol() {
        assert_eq!(service_prefill("5432/tcp"), "5432/tcp");
        assert_eq!(service_prefill("icmp"), "icmp");
        assert_eq!(service_prefill("1234/47"), "47");
        assert_eq!(service_prefill("0/icmp"), "icmp");
        assert_eq!(service_prefill(""), "");
    }

    #[test]
    fn scope_is_whole_words_and_another_org_is_not_this_pce() {
        let mut m = form().mark(now()).unwrap();
        m.id = "m-abc123".into();
        m.comment = "why".into();
        m.scope = Some(PceScope {
            host: "pce.example".into(),
            org: Some(7),
        });
        let mut e = MarkForm::edit(&m, &pce(5), false);
        assert_eq!(e.form.value("Scope"), "pce.example (org 7)");
        assert_eq!(e.mark(now()).unwrap().scope, m.scope, "round-trips");
        e.form.fields[SCOPE].value = THIS_PCE.into();
        assert_eq!(
            e.mark(now()).unwrap().scope.unwrap().org,
            None,
            "this PCE: not org 7"
        );
        e.form.fields[SCOPE].value = "anyhost.example".into();
        assert_eq!(
            e.mark(now()).unwrap().scope.unwrap().host,
            "anyhost.example"
        );
        e.form.fields[SCOPE].value = "thishost".into();
        assert_eq!(e.mark(now()).unwrap().scope.unwrap().host, "thishost");
        e.form.fields[SCOPE].value = "any".into();
        assert_eq!(e.mark(now()).unwrap().scope, None);
    }

    #[test]
    fn editing_keeps_the_id_org_and_creation_and_extend_asks_only_why() {
        let mut m = form().mark(now()).unwrap();
        m.id = "m-abc123".into();
        m.comment = "first reason".into();
        m.scope = Some(PceScope {
            host: "pce.example".into(),
            org: Some(5),
        });
        m.created = Some(now());
        let mut e = MarkForm::edit(&m, &pce(5), false);
        assert_eq!(e.form.value("Scope"), THIS_PCE);
        assert_eq!(e.form.value("Comment"), "first reason");
        e.form.fields[COMMENT].value = "second reason".into();
        let back = e.mark(now()).unwrap();
        assert_eq!(back.id, "m-abc123");
        assert_eq!(back.scope.unwrap().org, Some(5));
        assert_eq!(back.created, Some(now()));
        assert_eq!(back.expires, m.expires, "the date round-trips");
        let x = MarkForm::edit(&m, &pce(5), true);
        assert_eq!(x.form.fields.len(), 1);
        assert_eq!(x.comment(), "first reason");
    }
}
