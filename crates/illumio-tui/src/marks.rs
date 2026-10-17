//! `marks.toml`: the serde mirror of the file (`MarkFile`, `MarkEntry`,
//! unknown fields kept in `extra`), its conversion to and from
//! `pce::marks::Mark`, and `MarksStore`: reloaded when the file changes
//! (one `stat`), written as operations (`MarkOp`) under a lock, from a
//! fresh read, atomically, mode 0600. A local file only: marks never cause
//! a PCE request.
use jiff::Timestamp;
use pce::marks::{
    Action, Mark, MarkSet, PceScope, ServiceSpec, Side, derived_id, new_id, parse_decision,
    parse_side,
};
use pce::models::PolicyDecision;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

/// The first line of every write.
pub const HEADER: &str = "# illumio-tui marks. The tool rewrites this file: comments are not kept, put text in `comment`.";

/// The whole file: its marks in file order, and the top-level keys this
/// version does not know.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct MarkFile {
    #[serde(default)]
    pub mark: Vec<MarkEntry>,
    #[serde(default, flatten)]
    pub extra: toml::Table,
}

/// One `[[mark]]` as written: every known field as a TOML value, checked
/// by [`to_mark`] with the mark's position, and unknown fields kept.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct MarkEntry {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<toml::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<toml::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub src: Option<toml::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dst: Option<toml::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service: Option<toml::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decision: Option<toml::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub draft_decision: Option<toml::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pce: Option<toml::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub org: Option<toml::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires: Option<toml::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comment: Option<toml::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created: Option<toml::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_on: Option<toml::Value>,
    #[serde(default, flatten)]
    pub extra: toml::Table,
}

impl MarkEntry {
    /// The known fields in the order they are written.
    fn fields(&self) -> [(&'static str, Option<&toml::Value>); 13] {
        [
            ("id", self.id.as_ref()),
            ("action", self.action.as_ref()),
            ("src", self.src.as_ref()),
            ("dst", self.dst.as_ref()),
            ("service", self.service.as_ref()),
            ("decision", self.decision.as_ref()),
            ("draft_decision", self.draft_decision.as_ref()),
            ("pce", self.pce.as_ref()),
            ("org", self.org.as_ref()),
            ("expires", self.expires.as_ref()),
            ("comment", self.comment.as_ref()),
            ("created", self.created.as_ref()),
            ("created_on", self.created_on.as_ref()),
        ]
    }

    /// The entry's own `id`, when it is a string.
    pub fn id_text(&self) -> Option<&str> {
        self.id.as_ref().and_then(toml::Value::as_str)
    }
}

/// One validation problem, with its mark's position (from 1; 0 is the
/// file itself).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarkProblem {
    pub index: usize,
    pub id: Option<String>,
    pub message: String,
}

impl std::fmt::Display for MarkProblem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match (self.index, &self.id) {
            (0, _) => write!(f, "marks.toml: {}", self.message),
            (i, Some(id)) => write!(f, "mark {i} ({id}): {}", self.message),
            (i, None) => write!(f, "mark {i}: {}", self.message),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MarksError {
    /// The file does not validate: marks are off and nothing writes it.
    #[error("{}", problems_text(.0))]
    Parse(Vec<MarkProblem>),
    /// The file cannot be read or written.
    #[error("{0}")]
    Io(String),
    /// Another writer holds `marks.toml.lock`.
    #[error("marks.toml is locked by another illumio-tui (marks.toml.lock): try again")]
    Locked,
    /// The mark an operation names is not in the file any more.
    #[error("mark {0} was deleted elsewhere")]
    Gone(String),
}

/// `marks.toml invalid (2 problems): mark 3 (m-c41fz0): workload needs pce`.
fn problems_text(p: &[MarkProblem]) -> String {
    let n = p.len();
    let first = p.first().map(|x| x.to_string()).unwrap_or_default();
    format!(
        "marks.toml invalid ({n} problem{}): {first}",
        if n == 1 { "" } else { "s" }
    )
}

impl MarksError {
    /// The problems of an invalid file, one per line; the error otherwise.
    pub fn lines(&self) -> Vec<String> {
        match self {
            MarksError::Parse(p) => p.iter().map(|x| x.to_string()).collect(),
            e => vec![e.to_string()],
        }
    }
}

/// A parsed, valid file.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Loaded {
    pub file: MarkFile,
    /// The marks, aligned with `file.mark`; hand-written marks without an
    /// id carry their derived `h-` id.
    pub set: MarkSet,
    /// `m-… has no comment`: loads, shows `(no comment)`.
    pub warnings: Vec<String>,
    /// A comment line other than the header: the next write keeps the
    /// file in `marks.toml.bak` first.
    pub comments: bool,
}

/// `$XDG_CONFIG_HOME/illumio-tui/marks.toml`, else
/// `~/.config/illumio-tui/marks.toml`: next to the default config file.
pub fn default_marks_path() -> PathBuf {
    marks_path(None)
}

/// `marks.toml` in the directory of the config file in use (`--config`),
/// else [`default_marks_path`].
pub fn marks_path(config: Option<&Path>) -> PathBuf {
    let config = config
        .map(Path::to_path_buf)
        .unwrap_or_else(crate::config::default_config_path);
    config
        .parent()
        .map(|d| d.join("marks.toml"))
        .unwrap_or_else(|| PathBuf::from("marks.toml"))
}

fn text_of<'v>(v: &'v toml::Value, what: &str) -> Result<&'v str, String> {
    v.as_str()
        .ok_or_else(|| format!("{what}: expected a string, found {}", v.type_str()))
}

/// A string or a list of strings, as a list.
fn strings(v: &toml::Value, what: &str) -> Result<Vec<String>, String> {
    let list: Vec<String> = match v {
        toml::Value::Array(a) => a
            .iter()
            .map(|x| text_of(x, what).map(str::to_string))
            .collect::<Result<_, _>>()?,
        other => vec![text_of(other, what)?.to_string()],
    };
    if list.is_empty() {
        return Err(format!("{what}: empty list"));
    }
    Ok(list)
}

/// `"any"`, or a table with exactly one of `labels`, `workload`, `ip`,
/// `ip_list`.
fn side_of(v: &toml::Value, what: &str) -> Result<Side, String> {
    const KINDS: [&str; 4] = ["labels", "workload", "ip", "ip_list"];
    let t = match v {
        toml::Value::String(s) if s.trim().eq_ignore_ascii_case("any") => return Ok(Side::Any),
        toml::Value::Table(t) => t,
        _ => {
            return Err(format!(
                "{what}: write \"any\" or one of {{ labels = … }}, {{ workload = … }}, {{ ip = … }}, {{ ip_list = … }}"
            ));
        }
    };
    if let Some(k) = t.keys().find(|k| !KINDS.contains(&k.as_str())) {
        return Err(format!("{what}: unknown kind {k:?}"));
    }
    let kinds: Vec<&String> = t.keys().collect();
    let (kind, value) = match kinds.as_slice() {
        [k] => (k.as_str(), &t[k.as_str()]),
        [] => return Err(format!("{what}: no kind (labels, workload, ip or ip_list)")),
        many => {
            let names: Vec<&str> = many.iter().map(|k| k.as_str()).collect();
            return Err(format!("{what}: several kinds ({})", names.join(", ")));
        }
    };
    let prefixed = |p: &str| -> Result<Side, String> {
        parse_side(&format!("{p}{}", text_of(value, what)?)).map_err(|e| format!("{what}: {e}"))
    };
    match kind {
        "workload" => prefixed("wl:"),
        "ip" => prefixed("ip:"),
        "ip_list" => prefixed("ipl:"),
        _ => {
            let labels = value
                .as_table()
                .ok_or_else(|| format!("{what}: labels must be a table of key = value"))?;
            if labels.is_empty() {
                return Err(format!("{what}: empty labels"));
            }
            let mut out = std::collections::BTreeMap::new();
            for (k, v) in labels {
                let values = strings(v, &format!("{what}: label {k}"))?;
                if values.iter().any(String::is_empty) {
                    return Err(format!("{what}: label {k}: empty value"));
                }
                out.insert(k.clone(), values);
            }
            Ok(Side::Labels(out))
        }
    }
}

fn timestamp_of(v: &toml::Value, what: &str) -> Result<Timestamp, String> {
    let text = match v {
        toml::Value::Datetime(d) => d.to_string(),
        other => text_of(other, what)?.to_string(),
    };
    text.parse::<Timestamp>().map_err(|_| {
        format!("{what}: bad timestamp {text:?} (RFC 3339 UTC, e.g. 2027-01-15T23:59:59Z)")
    })
}

fn decisions_of(v: Option<&toml::Value>, what: &str) -> Result<Vec<PolicyDecision>, String> {
    match v {
        None => Ok(Vec::new()),
        Some(v) => strings(v, what)?
            .iter()
            .map(|d| parse_decision(d).map_err(|e| format!("{what}: {e}")))
            .collect(),
    }
}

/// `r`'s value, its error kept in `problems`.
fn take<T>(problems: &mut Vec<String>, r: Result<T, String>) -> Option<T> {
    r.map_err(|e| problems.push(e)).ok()
}

/// One entry as a mark: every problem of its fields at once. A missing
/// id is left empty (the caller derives one).
pub fn to_mark(e: &MarkEntry) -> Result<Mark, Vec<String>> {
    let mut problems = Vec::new();
    let id = take(
        &mut problems,
        match &e.id {
            None => Ok(String::new()),
            Some(v) => text_of(v, "id").map(str::to_string),
        },
    );
    let action = take(
        &mut problems,
        match &e.action {
            None => Err("action is required".into()),
            Some(v) => text_of(v, "action").and_then(|a| {
                Action::parse(a).ok_or_else(|| format!("unknown action {a:?}: {}", Action::names()))
            }),
        },
    );
    let src = take(
        &mut problems,
        match &e.src {
            None => Err("src is required (\"any\" for any source)".into()),
            Some(v) => side_of(v, "src"),
        },
    );
    let dst = take(
        &mut problems,
        match &e.dst {
            None => Err("dst is required (\"any\" for any destination)".into()),
            Some(v) => side_of(v, "dst"),
        },
    );
    let services = take(
        &mut problems,
        match &e.service {
            None => Ok(Vec::new()),
            Some(v) => strings(v, "service").and_then(|l| {
                l.iter()
                    .map(|s| ServiceSpec::parse(s).map_err(|e| format!("service: {e}")))
                    .collect()
            }),
        },
    );
    let decisions = take(&mut problems, decisions_of(e.decision.as_ref(), "decision"));
    let draft_decisions = take(
        &mut problems,
        decisions_of(e.draft_decision.as_ref(), "draft_decision"),
    );
    let host = take(
        &mut problems,
        match &e.pce {
            None => Ok(None),
            Some(v) => text_of(v, "pce").and_then(|h| {
                if h.trim().is_empty() {
                    Err("pce: empty host".into())
                } else {
                    Ok(Some(h.trim().to_string()))
                }
            }),
        },
    );
    let org = take(
        &mut problems,
        match &e.org {
            None => Ok(None),
            Some(v) => v
                .as_integer()
                .and_then(|n| u64::try_from(n).ok())
                .map(Some)
                .ok_or_else(|| "org: expected an org id (a number)".to_string()),
        },
    );
    let expires = take(
        &mut problems,
        e.expires
            .as_ref()
            .map(|v| timestamp_of(v, "expires"))
            .transpose(),
    );
    let created = take(
        &mut problems,
        e.created
            .as_ref()
            .map(|v| timestamp_of(v, "created"))
            .transpose(),
    );
    let comment = take(
        &mut problems,
        match &e.comment {
            None => Ok(String::new()),
            Some(v) => text_of(v, "comment").map(str::to_string),
        },
    );
    let created_on = take(
        &mut problems,
        match &e.created_on {
            None => Ok(None),
            Some(v) => text_of(v, "created_on").map(|s| Some(s.to_string())),
        },
    );
    let scope = match (host, org) {
        (Some(Some(host)), Some(org)) => Some(PceScope { host, org }),
        (Some(None), Some(Some(_))) => {
            problems.push("org needs pce".into());
            None
        }
        _ => None,
    };
    let (
        Some(id),
        Some(action),
        Some(src),
        Some(dst),
        Some(services),
        Some(decisions),
        Some(draft_decisions),
        Some(expires),
        Some(created),
        Some(comment),
        Some(created_on),
    ) = (
        id,
        action,
        src,
        dst,
        services,
        decisions,
        draft_decisions,
        expires,
        created,
        comment,
        created_on,
    )
    else {
        return Err(problems);
    };
    if !problems.is_empty() {
        return Err(problems);
    }
    let m = Mark {
        id,
        action,
        src,
        dst,
        services,
        decisions,
        draft_decisions,
        scope,
        expires,
        comment,
        created,
        created_on,
    };
    let rules = m.problems();
    if rules.is_empty() { Ok(m) } else { Err(rules) }
}

fn side_value(s: &Side) -> toml::Value {
    use toml::Value as V;
    let one = |k: &str, v: String| {
        let mut t = toml::Table::new();
        t.insert(k.into(), V::String(v));
        V::Table(t)
    };
    match s {
        Side::Any => V::String("any".into()),
        Side::Labels(l) => {
            let mut labels = toml::Table::new();
            for (k, vs) in l {
                let v = match vs.as_slice() {
                    [one] => V::String(one.clone()),
                    many => V::Array(many.iter().cloned().map(V::String).collect()),
                };
                labels.insert(k.clone(), v);
            }
            let mut t = toml::Table::new();
            t.insert("labels".into(), V::Table(labels));
            V::Table(t)
        }
        Side::Workload(w) => one("workload", w.clone()),
        Side::Ip { .. } => one(
            "ip",
            pce::marks::side_text(s)
                .trim_start_matches("ip:")
                .to_string(),
        ),
        Side::IpList(l) => one("ip_list", l.clone()),
    }
}

/// One string, or a list of them.
fn list_value(items: Vec<String>) -> Option<toml::Value> {
    match items.len() {
        0 => None,
        1 => items.into_iter().next().map(toml::Value::String),
        _ => Some(toml::Value::Array(
            items.into_iter().map(toml::Value::String).collect(),
        )),
    }
}

/// The entry that writes `m`, with `extra` (the old entry's unknown
/// fields on a replace).
pub fn entry_of(m: &Mark, extra: toml::Table) -> MarkEntry {
    let s = |x: &str| Some(toml::Value::String(x.to_string()));
    let decisions =
        |d: &[PolicyDecision]| list_value(d.iter().map(|d| d.as_str().into()).collect());
    MarkEntry {
        id: (!m.id.is_empty()).then(|| toml::Value::String(m.id.clone())),
        action: s(m.action.as_str()),
        src: Some(side_value(&m.src)),
        dst: Some(side_value(&m.dst)),
        service: list_value(m.services.iter().map(ServiceSpec::text).collect()),
        decision: decisions(&m.decisions),
        draft_decision: decisions(&m.draft_decisions),
        pce: m.scope.as_ref().and_then(|p| s(&p.host)),
        org: m
            .scope
            .as_ref()
            .and_then(|p| p.org)
            .and_then(|o| i64::try_from(o).ok())
            .map(toml::Value::Integer),
        expires: m.expires.and_then(|t| s(&t.to_string())),
        comment: (!m.comment.is_empty()).then(|| toml::Value::String(m.comment.clone())),
        created: m.created.and_then(|t| s(&t.to_string())),
        created_on: m.created_on.as_deref().and_then(s),
        extra,
    }
}

/// Every entry as a mark, with positions, ids (derived for hand-written
/// marks without one), duplicate ids and missing comments.
fn convert(file: &MarkFile) -> Result<(MarkSet, Vec<String>), Vec<MarkProblem>> {
    let mut problems = Vec::new();
    let mut marks: Vec<Mark> = Vec::new();
    let mut warnings = Vec::new();
    for (i, e) in file.mark.iter().enumerate() {
        let index = i + 1;
        match to_mark(e) {
            Ok(mut m) => {
                let derived = m.id.is_empty();
                if derived {
                    m.id = derived_id(&m.canonical());
                }
                if let Some(j) = marks.iter().position(|x| x.id == m.id) {
                    let message = if derived {
                        // A hand-written mark without an id: the same
                        // fields as mark j, so the same derived id.
                        format!("duplicates mark {} (the same fields)", j + 1)
                    } else {
                        format!("id {} is used by mark {} too", m.id, j + 1)
                    };
                    problems.push(MarkProblem {
                        index,
                        id: Some(m.id.clone()),
                        message,
                    });
                }
                if m.comment.trim().is_empty() {
                    warnings.push(format!("{} has no comment", m.id));
                }
                marks.push(m);
            }
            Err(msgs) => {
                let id = e.id_text().map(str::to_string);
                problems.extend(msgs.into_iter().map(|message| MarkProblem {
                    index,
                    id: id.clone(),
                    message,
                }));
                // A placeholder keeps the positions of later duplicates.
                marks.push(Mark {
                    id: id.unwrap_or_default(),
                    ..placeholder()
                });
            }
        }
    }
    if problems.is_empty() {
        Ok((MarkSet { marks }, warnings))
    } else {
        Err(problems)
    }
}

fn placeholder() -> Mark {
    Mark {
        id: String::new(),
        action: Action::Reviewed,
        src: Side::Any,
        dst: Side::Any,
        services: Vec::new(),
        decisions: Vec::new(),
        draft_decisions: Vec::new(),
        scope: None,
        expires: None,
        comment: String::new(),
        created: None,
        created_on: None,
    }
}

/// A TOML comment other than the header line: a `#` outside every kind
/// of string, on its own line or after a value. A `#` inside a string (a
/// multi-line `comment` included) is text, not a comment: counting it
/// would make a later write replace the `.bak` that holds the user's
/// real comments.
fn has_comments(text: &str) -> bool {
    #[derive(PartialEq)]
    enum St {
        Normal,
        Basic,
        Literal,
        MlBasic,
        MlLiteral,
    }
    let b = text.as_bytes();
    let at = |i: usize, pat: &[u8]| b[i..].starts_with(pat);
    let mut st = St::Normal;
    let mut i = 0;
    while i < b.len() {
        match st {
            St::Normal => {
                if at(i, b"\"\"\"") {
                    st = St::MlBasic;
                    i += 3;
                    continue;
                }
                if at(i, b"'''") {
                    st = St::MlLiteral;
                    i += 3;
                    continue;
                }
                match b[i] {
                    b'"' => st = St::Basic,
                    b'\'' => st = St::Literal,
                    b'#' => {
                        let start = text[..i].rfind('\n').map_or(0, |n| n + 1);
                        let end = text[i..].find('\n').map_or(b.len(), |n| i + n);
                        if text[start..end].trim() != HEADER {
                            return true;
                        }
                        i = end;
                        continue;
                    }
                    _ => {}
                }
            }
            St::Basic | St::MlBasic if b[i] == b'\\' => {
                i += 2;
                continue;
            }
            St::Basic if b[i] == b'"' || b[i] == b'\n' => st = St::Normal,
            St::Literal if b[i] == b'\'' || b[i] == b'\n' => st = St::Normal,
            // Up to two more quotes before the closing three are content
            // (`"""say "hi""""`): the string ends after all of them.
            St::MlBasic if at(i, b"\"\"\"") => {
                st = St::Normal;
                i += closing_run(b, i, b'"');
                continue;
            }
            St::MlLiteral if at(i, b"'''") => {
                st = St::Normal;
                i += closing_run(b, i, b'\'');
                continue;
            }
            _ => {}
        }
        i += 1;
    }
    false
}

/// The length of a multi-line string's closing run at `i`: the three
/// delimiter quotes plus at most two quotes of content before them.
fn closing_run(b: &[u8], i: usize, quote: u8) -> usize {
    b[i..].iter().take(5).take_while(|&&c| c == quote).count()
}

/// The 1-based line and column (in characters) of byte `at` of `text`.
fn line_col(text: &str, at: usize) -> (usize, usize) {
    let at = (0..=at.min(text.len()))
        .rev()
        .find(|&i| text.is_char_boundary(i))
        .unwrap_or(0);
    let before = &text[..at];
    let line = before.matches('\n').count() + 1;
    let col = before
        .rfind('\n')
        .map_or(before, |n| &before[n + 1..])
        .chars()
        .count()
        + 1;
    (line, col)
}

/// A TOML syntax error as `<message> at line L, column C` (when the
/// parser gives a position): never the offending text.
pub fn toml_error_text(text: &str, e: &toml::de::Error) -> String {
    match e.span() {
        Some(span) => {
            let (line, col) = line_col(text, span.start);
            format!("{} at line {line}, column {col}", e.message().trim())
        }
        None => e.message().trim().to_string(),
    }
}

/// The file's text, every problem at once. The parser's own message is
/// reported with its line and column only, never the offending text.
pub fn parse(text: &str) -> Result<Loaded, Vec<MarkProblem>> {
    let file: MarkFile = toml::from_str(text).map_err(|e| {
        vec![MarkProblem {
            index: 0,
            id: None,
            message: toml_error_text(text, &e),
        }]
    })?;
    let (set, warnings) = convert(&file)?;
    Ok(Loaded {
        file,
        set,
        warnings,
        comments: has_comments(text),
    })
}

/// `key`, or `"key"` when it is not a bare TOML key.
fn key_text(k: &str) -> String {
    if !k.is_empty()
        && k.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        k.to_string()
    } else {
        toml::Value::String(k.to_string()).to_string()
    }
}

/// The file's text: the header, the unknown top-level keys, then each
/// mark in order, known fields first (inline values), unknown ones after.
pub fn render(file: &MarkFile) -> String {
    let mut out = format!("{HEADER}\n");
    if !file.extra.is_empty() {
        out.push('\n');
        for (k, v) in &file.extra {
            out.push_str(&format!("{} = {v}\n", key_text(k)));
        }
    }
    for e in &file.mark {
        out.push_str("\n[[mark]]\n");
        for (k, v) in e.fields() {
            if let Some(v) = v {
                out.push_str(&format!("{k} = {v}\n"));
            }
        }
        for (k, v) in &e.extra {
            out.push_str(&format!("{} = {v}\n", key_text(k)));
        }
    }
    out
}

/// One write: what changes, never the whole state, so writers in other
/// instances keep their changes.
#[derive(Debug, Clone, PartialEq)]
pub enum MarkOp {
    /// A new mark; an empty id gets a new one, an empty `created` now.
    Add(Mark),
    /// The mark with this id, new fields; its unknown fields are kept.
    Replace(Mark),
    /// These marks, all or none.
    Delete(Vec<String>),
    /// Every mark expired at `now`.
    DeleteExpired,
    /// `days` more from the later of now and the mark's expiry (a `never`
    /// mark is left as it is), with a new comment when given.
    Extend {
        id: String,
        days: i64,
        comment: Option<String>,
    },
    /// Validated marks from `marks import`: an existing id is skipped, or
    /// replaced with `replace`; an empty id gets a new one.
    Import {
        marks: Vec<(Mark, toml::Table)>,
        replace: bool,
    },
}

/// What a write did.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Applied {
    /// The marks added or changed, by id.
    pub ids: Vec<String>,
    pub added: usize,
    pub replaced: usize,
    pub removed: Vec<String>,
    pub skipped: Vec<String>,
    /// The file had comments and was kept here first.
    pub backup: Option<PathBuf>,
}

/// `op` on `loaded` (a fresh read).
fn apply_op(loaded: &mut Loaded, op: MarkOp, now: Timestamp) -> Result<Applied, MarksError> {
    let pid = std::process::id();
    let mut done = Applied::default();
    let entries = &mut loaded.file.mark;
    let marks = &mut loaded.set.marks;
    // Every entry gets its id written (hand-written ones their derived id).
    for (e, m) in entries.iter_mut().zip(marks.iter()) {
        if e.id_text().is_none() {
            e.id = Some(toml::Value::String(m.id.clone()));
        }
    }
    let find = |marks: &[Mark], id: &str| {
        marks
            .iter()
            .position(|m| m.id == id)
            .ok_or_else(|| MarksError::Gone(id.to_string()))
    };
    let fresh_id = |marks: &[Mark], m: &Mark| {
        new_id(now.as_nanosecond(), pid, &m.canonical(), |id| {
            marks.iter().any(|x| x.id == id)
        })
    };
    match op {
        MarkOp::Add(mut m) => {
            if m.id.is_empty() || marks.iter().any(|x| x.id == m.id) {
                m.id = fresh_id(marks, &m);
            }
            m.created.get_or_insert(now);
            entries.push(entry_of(&m, toml::Table::new()));
            done.ids.push(m.id.clone());
            done.added = 1;
            marks.push(m);
        }
        MarkOp::Replace(mut m) => {
            let i = find(marks, &m.id)?;
            m.created = m.created.or(marks[i].created);
            m.created_on = m.created_on.or_else(|| marks[i].created_on.clone());
            let extra = std::mem::take(&mut entries[i].extra);
            entries[i] = entry_of(&m, extra);
            done.ids.push(m.id.clone());
            done.replaced = 1;
            marks[i] = m;
        }
        MarkOp::Delete(ids) => {
            for id in &ids {
                find(marks, id)?;
            }
            let keep: Vec<bool> = marks.iter().map(|m| !ids.contains(&m.id)).collect();
            retain(entries, marks, &keep);
            done.removed = ids;
        }
        MarkOp::DeleteExpired => {
            let keep: Vec<bool> = marks.iter().map(|m| !m.is_expired(now)).collect();
            done.removed = marks
                .iter()
                .filter(|m| m.is_expired(now))
                .map(|m| m.id.clone())
                .collect();
            retain(entries, marks, &keep);
        }
        MarkOp::Extend { id, days, comment } => {
            let i = find(marks, &id)?;
            let m = &mut marks[i];
            if let Some(e) = m.expires {
                m.expires = Some(pce::marks::add_days(e.max(now), days).ok_or_else(|| {
                    MarksError::Parse(vec![MarkProblem {
                        index: i + 1,
                        id: Some(id.clone()),
                        message: format!("expires: +{days}d is too far"),
                    }])
                })?);
                if let Some(c) = comment {
                    m.comment = c;
                }
                let extra = std::mem::take(&mut entries[i].extra);
                entries[i] = entry_of(m, extra);
                done.ids.push(id);
                done.replaced = 1;
            }
        }
        MarkOp::Import {
            marks: incoming,
            replace,
        } => {
            for (mut m, extra) in incoming {
                match marks.iter().position(|x| !m.id.is_empty() && x.id == m.id) {
                    Some(i) if replace => {
                        entries[i] = entry_of(&m, extra);
                        done.ids.push(m.id.clone());
                        done.replaced += 1;
                        marks[i] = m;
                    }
                    Some(_) => done.skipped.push(m.id.clone()),
                    None => {
                        if m.id.is_empty() {
                            m.id = fresh_id(marks, &m);
                        }
                        m.created.get_or_insert(now);
                        entries.push(entry_of(&m, extra));
                        done.ids.push(m.id.clone());
                        done.added += 1;
                        marks.push(m);
                    }
                }
            }
        }
    }
    Ok(done)
}

fn retain(entries: &mut Vec<MarkEntry>, marks: &mut Vec<Mark>, keep: &[bool]) {
    let mut k = keep.iter();
    entries.retain(|_| *k.next().unwrap_or(&true));
    let mut k = keep.iter();
    marks.retain(|_| *k.next().unwrap_or(&true));
}

/// What `refresh` compares: the file's modification time and length, or
/// its absence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stamp {
    Missing,
    File {
        modified: Option<SystemTime>,
        len: u64,
    },
}

/// `marks.toml.lock`, `marks.toml.tmp-<pid>`, `marks.toml.bak`.
fn sibling(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path
        .file_name()
        .map(|n| n.to_os_string())
        .unwrap_or_else(|| "marks.toml".into());
    name.push(suffix);
    path.with_file_name(name)
}

/// The lock retries every 25 ms for this long.
const LOCK_WAIT: Duration = Duration::from_secs(1);
/// A lock file older than this is a crashed writer's.
const LOCK_STALE: Duration = Duration::from_secs(10);

/// Held while a write runs; the lock file goes when it drops.
struct Lock(PathBuf);

impl Drop for Lock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

fn io(path: &Path, what: &str, e: std::io::Error) -> MarksError {
    MarksError::Io(format!("cannot {what} {}: {e}", path.display()))
}

/// `create_new` (O_EXCL) on the lock file, std only: `File::lock` needs
/// Rust 1.89 and the workspace is 1.88.
fn lock(path: &Path) -> Result<Lock, MarksError> {
    let lp = sibling(path, ".lock");
    let start = Instant::now();
    let mut stale_removed = false;
    loop {
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&lp)
        {
            Ok(mut f) => {
                let _ = writeln!(f, "{}", std::process::id());
                return Ok(Lock(lp));
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                let stale = fs::metadata(&lp)
                    .and_then(|m| m.modified())
                    .ok()
                    .and_then(|t| t.elapsed().ok())
                    .is_some_and(|age| age > LOCK_STALE);
                if stale && !stale_removed {
                    stale_removed = true;
                    let _ = fs::remove_file(&lp);
                    continue;
                }
                if start.elapsed() >= LOCK_WAIT {
                    return Err(MarksError::Locked);
                }
                std::thread::sleep(Duration::from_millis(25));
            }
            Err(e) => return Err(io(&lp, "create", e)),
        }
    }
}

/// The directory, created 0700 on Unix when missing.
fn ensure_dir(dir: &Path) -> Result<(), MarksError> {
    if dir.as_os_str().is_empty() || dir.is_dir() {
        return Ok(());
    }
    let mut b = fs::DirBuilder::new();
    b.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        b.mode(0o700);
    }
    b.create(dir).map_err(|e| io(dir, "create", e))
}

/// Writes `text` to `path` mode 0600: a temp file in the same directory,
/// synced, renamed over it.
fn write_atomic(path: &Path, text: &str) -> Result<(), MarksError> {
    let tmp = sibling(path, &format!(".tmp-{}", std::process::id()));
    let mut opts = fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let result = (|| {
        let mut f = opts.open(&tmp)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            f.set_permissions(fs::Permissions::from_mode(0o600))?;
        }
        f.write_all(text.as_bytes())?;
        f.sync_all()?;
        fs::rename(&tmp, path)
    })();
    result.map_err(|e| {
        let _ = fs::remove_file(&tmp);
        io(path, "write", e)
    })?;
    sync_dir(path);
    Ok(())
}

/// Makes the rename durable: syncs the directory where that is supported
/// (Unix). Best effort: the file is written either way.
fn sync_dir(path: &Path) {
    #[cfg(unix)]
    if let Some(dir) = path.parent() {
        let dir = if dir.as_os_str().is_empty() {
            Path::new(".")
        } else {
            dir
        };
        if let Ok(d) = fs::File::open(dir) {
            let _ = d.sync_all();
        }
    }
    #[cfg(not(unix))]
    let _ = path;
}

/// The marks file: its path, its last stamp and the parsed result.
#[derive(Debug, Clone)]
pub struct MarksStore {
    path: PathBuf,
    stamp: Option<Stamp>,
    state: Result<Loaded, MarksError>,
    parses: u64,
}

impl MarksStore {
    /// A store that has not looked at its file yet: no marks.
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            stamp: None,
            state: Ok(Loaded::default()),
            parses: 0,
        }
    }

    /// A store without a file: no marks, nothing read or written (a model
    /// before `app::run` gives it the path, and every test that does not).
    pub fn none() -> Self {
        Self::new(PathBuf::new())
    }

    /// `new`, then one `refresh`.
    pub fn open(path: PathBuf) -> Self {
        let mut s = Self::new(path);
        s.refresh();
        s
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The valid marks; `None` while the file is invalid or unreadable.
    pub fn set(&self) -> Option<&MarkSet> {
        self.state.as_ref().ok().map(|l| &l.set)
    }

    pub fn loaded(&self) -> Option<&Loaded> {
        self.state.as_ref().ok()
    }

    pub fn error(&self) -> Option<&MarksError> {
        self.state.as_ref().err()
    }

    pub fn warnings(&self) -> &[String] {
        self.loaded().map_or(&[], |l| l.warnings.as_slice())
    }

    /// How many times the file's text was parsed (tests: no reparse after
    /// this store's own write).
    pub fn parses(&self) -> u64 {
        self.parses
    }

    fn stamp_now(&self) -> Result<Stamp, MarksError> {
        match fs::metadata(&self.path) {
            Ok(m) => Ok(Stamp::File {
                modified: m.modified().ok(),
                len: m.len(),
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Stamp::Missing),
            Err(e) => Err(io(&self.path, "read", e)),
        }
    }

    /// The file's text and its parse, `Loaded::default()` when missing.
    fn read(&mut self) -> Result<Loaded, MarksError> {
        match fs::read_to_string(&self.path) {
            Ok(text) => {
                self.parses += 1;
                parse(&text).map_err(MarksError::Parse)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Loaded::default()),
            Err(e) => Err(io(&self.path, "read", e)),
        }
    }

    /// Whether a `refresh` would read the file again (its stamp changed,
    /// or a stat error changed): one `stat`, nothing read or kept.
    pub fn is_stale(&self) -> bool {
        if self.path.as_os_str().is_empty() {
            return false;
        }
        match self.stamp_now() {
            Ok(s) => self.stamp != Some(s),
            Err(e) => self.error() != Some(&e),
        }
    }

    /// One `stat`; the file is read and parsed again only when its stamp
    /// changed. Whether it did. A local file, never a request.
    pub fn refresh(&mut self) -> bool {
        if self.path.as_os_str().is_empty() {
            return false;
        }
        let stamp = match self.stamp_now() {
            Ok(s) => s,
            Err(e) => {
                let changed = self.error() != Some(&e);
                self.stamp = None;
                self.state = Err(e);
                return changed;
            }
        };
        if self.stamp == Some(stamp) {
            return false;
        }
        self.stamp = Some(stamp);
        self.state = self.read();
        true
    }

    /// One operation under the lock: read and parse the file again
    /// (whatever its stamp says), apply `op`, keep a file with comments in
    /// `marks.toml.bak` first, write atomically (0600), unlock. An invalid
    /// file is never written: its marks would be lost.
    pub fn apply(&mut self, op: MarkOp, now: Timestamp) -> Result<Applied, MarksError> {
        if self.path.as_os_str().is_empty() {
            return Err(MarksError::Io("no marks file in this session".into()));
        }
        if let Some(dir) = self.path.parent() {
            ensure_dir(dir)?;
        }
        let _lock = lock(&self.path)?;
        let text = match fs::read_to_string(&self.path) {
            Ok(t) => Some(t),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(io(&self.path, "read", e)),
        };
        self.parses += 1;
        let mut loaded = match text.as_deref().map(parse) {
            None => Loaded::default(),
            Some(Ok(l)) => l,
            Some(Err(p)) => {
                let e = MarksError::Parse(p);
                self.stamp = self.stamp_now().ok();
                self.state = Err(e.clone());
                return Err(e);
            }
        };
        let mut done = apply_op(&mut loaded, op, now)?;
        if done.ids.is_empty() && done.removed.is_empty() {
            return Ok(done);
        }
        let (set, warnings) = convert(&loaded.file).map_err(MarksError::Parse)?;
        if loaded.comments
            && let Some(old) = &text
        {
            let bak = sibling(&self.path, ".bak");
            write_atomic(&bak, old)?;
            done.backup = Some(bak);
        }
        write_atomic(&self.path, &render(&loaded.file))?;
        // The write succeeded: a failed stat after it only means the next
        // refresh parses the file once more.
        self.stamp = self.stamp_now().ok();
        self.state = Ok(Loaded {
            file: loaded.file,
            set,
            warnings,
            comments: false,
        });
        Ok(done)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::SignedDuration;

    const SPEC: &str = r#"# illumio-tui marks. The tool rewrites this file: comments are not kept, put text in `comment`.

[[mark]]
id = "m-7k2q9x"
action = "squelch"
src = { labels = { app = "monitoring" } }
dst = "any"
service = "5666/tcp"
expires = "2027-01-15T23:59:59Z"
comment = "Nagios NRPE, known"
created = "2026-10-17T09:12:00Z"
created_on = "pce.example.com"

[[mark]]
id = "m-c41fz0"
action = "accept-blocked"
src = { ip_list = "rfc1918" }
dst = { labels = { app = "shop", env = ["prod", "dr"] } }
service = "8000-8010/tcp"
decision = "blocked"
pce = "pce.example.com"
comment = "legacy batch, blocked on purpose (CHG-1182)"
created = "2026-10-17T09:20:00Z"
created_on = "pce.example.com"

[[mark]]
id = "m-q9e2aa"
action = "critical"
src = "any"
dst = { workload = "db-01" }
pce = "pce.example.com"
org = 1
created = "2026-10-17T09:31:00Z"
created_on = "pce.example.com"
"#;

    fn now() -> Timestamp {
        "2026-10-17T12:00:00Z".parse().unwrap()
    }

    fn store(dir: &tempfile::TempDir) -> MarksStore {
        MarksStore::open(dir.path().join("illumio-tui").join("marks.toml"))
    }

    fn write(s: &MarksStore, text: &str) {
        fs::create_dir_all(s.path().parent().unwrap()).unwrap();
        fs::write(s.path(), text).unwrap();
    }

    fn new_mark(action: Action, src: &str, comment: &str) -> Mark {
        Mark {
            id: String::new(),
            action,
            src: parse_side(src).unwrap(),
            dst: Side::Any,
            services: Vec::new(),
            decisions: Vec::new(),
            draft_decisions: Vec::new(),
            scope: None,
            expires: None,
            comment: comment.into(),
            created: None,
            created_on: None,
        }
    }

    #[test]
    fn the_spec_example_parses_and_renders_back_unchanged() {
        let l = parse(SPEC).unwrap();
        assert_eq!(l.set.marks.len(), 3);
        assert_eq!(l.set.marks[1].action, Action::AcceptBlocked);
        assert_eq!(
            l.set.marks[2].scope,
            Some(PceScope {
                host: "pce.example.com".into(),
                org: Some(1)
            })
        );
        assert_eq!(l.warnings, vec!["m-q9e2aa has no comment"]);
        assert!(!l.comments, "the header is not a comment");
        assert_eq!(render(&l.file), SPEC);
    }

    #[test]
    fn a_round_trip_keeps_unknown_fields_and_order() {
        let text = format!(
            "{HEADER}\n\nversion = 3\n\n[[mark]]\nid = \"m-b\"\naction = \"reviewed\"\nsrc = \"any\"\ndst = \"any\"\ncomment = \"b\"\nowner = {{ team = \"net\" }}\n\n[[mark]]\nid = \"m-a\"\naction = \"revisit\"\nsrc = \"any\"\ndst = \"any\"\ncomment = \"a\"\n"
        );
        let dir = tempfile::tempdir().unwrap();
        let mut s = store(&dir);
        write(&s, &text);
        s.refresh();
        let done = s
            .apply(
                MarkOp::Add(new_mark(Action::Squelch, "ipl:Corp", "c")),
                now(),
            )
            .unwrap();
        let back = fs::read_to_string(s.path()).unwrap();
        assert!(back.contains("version = 3"), "{back}");
        assert!(back.contains("owner = { team = \"net\" }"), "{back}");
        let l = parse(&back).unwrap();
        let ids: Vec<&str> = l.set.marks.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(
            ids,
            vec!["m-b", "m-a", done.ids[0].as_str()],
            "file order, new last"
        );
        assert_eq!(l.set.marks[2].created, Some(now()));
    }

    #[test]
    fn comments_are_kept_once_in_the_backup_then_the_header_says_so() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = store(&dir);
        write(
            &s,
            "# my notes\n[[mark]]\naction = \"reviewed\"\nsrc = \"any\"\ndst = \"any\"\ncomment = \"x\"\n",
        );
        s.refresh();
        assert!(s.loaded().unwrap().comments);
        let done = s
            .apply(MarkOp::Add(new_mark(Action::Revisit, "app=a", "y")), now())
            .unwrap();
        let bak = sibling(s.path(), ".bak");
        assert_eq!(done.backup.as_deref(), Some(bak.as_path()));
        assert!(fs::read_to_string(&bak).unwrap().starts_with("# my notes"));
        let text = fs::read_to_string(s.path()).unwrap();
        assert!(text.starts_with(HEADER) && !text.contains("my notes"));
        // The id-less hand-written mark got its derived id written.
        assert!(text.contains("id = \"h-"), "{text}");
        fs::write(&bak, "old").unwrap();
        let again = s
            .apply(MarkOp::Add(new_mark(Action::Revisit, "app=b", "z")), now())
            .unwrap();
        assert_eq!(again.backup, None, "no comment left: no second backup");
        assert_eq!(fs::read_to_string(&bak).unwrap(), "old");
    }

    #[test]
    fn is_stale_stats_without_reading_and_refresh_clears_it() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = store(&dir);
        assert!(!s.is_stale(), "missing, and seen missing");
        assert!(!MarksStore::none().is_stale());
        write(
            &s,
            "[[mark]]\naction = \"reviewed\"\nsrc = \"any\"\ndst = \"any\"\ncomment = \"x\"\n",
        );
        let parses = s.parses();
        assert!(s.is_stale());
        assert_eq!(s.parses(), parses, "a stat, no parse");
        assert!(s.refresh());
        assert!(!s.is_stale());
    }

    #[test]
    fn quotes_before_a_multi_line_closing_delimiter_are_content() {
        // TOML: up to two quotes next to the closing `"""` / `'''` are
        // content, so the string ends after them and a trailing comment
        // is still a comment.
        assert!(has_comments("c = \"\"\"he said \"hi\"\"\"\" # note\n"));
        assert!(has_comments("c = \"\"\"x\"\"\"\"\" # note\n"));
        assert!(has_comments("c = '''it's 'q''''' # note\n"));
        assert!(has_comments("c = '''x'''' # note\n"));
        assert!(!has_comments(
            "c = \"\"\"he said \"hi\"\"\"\"\nd = \"# no\"\n"
        ));
        assert!(!has_comments("c = '''x'''''\nd = '# no'\n"));
    }

    #[test]
    fn a_hash_inside_a_string_is_not_a_comment_and_the_backup_survives() {
        assert!(!has_comments(&format!("{HEADER}\nx = \"a # b\"\n")));
        assert!(!has_comments("c = '''\n# not a comment\n'''\n"));
        assert!(!has_comments("c = \"\"\"\n# text \\\"\"\" still\n\"\"\"\n"));
        assert!(!has_comments("c = 'it''s'\n"));
        assert!(has_comments("x = 1 # trailing\n"));
        assert!(has_comments("c = '''a'''\n  # real\n"));
        // A file with real comments: one backup. A later write of a mark
        // whose comment has a `#` line keeps that backup.
        let dir = tempfile::tempdir().unwrap();
        let mut s = store(&dir);
        write(
            &s,
            "# my notes\n[[mark]]\naction = \"reviewed\"\nsrc = \"any\"\ndst = \"any\"\ncomment = \"x\"\n",
        );
        s.refresh();
        s.apply(
            MarkOp::Add(new_mark(Action::Revisit, "app=a", "line one\n# line two")),
            now(),
        )
        .unwrap();
        let bak = sibling(s.path(), ".bak");
        let text = fs::read_to_string(s.path()).unwrap();
        assert!(
            text.contains("\n# line two"),
            "a `#` line in a string: {text}"
        );
        let again = s
            .apply(MarkOp::Add(new_mark(Action::Revisit, "app=b", "z")), now())
            .unwrap();
        assert_eq!(again.backup, None);
        assert!(fs::read_to_string(&bak).unwrap().starts_with("# my notes"));
        let set = s.set().unwrap();
        assert!(
            set.marks
                .iter()
                .any(|m| m.comment == "line one\n# line two")
        );
    }

    #[test]
    fn same_hand_marks_say_duplicate_and_syntax_errors_give_line_and_column() {
        let one =
            "[[mark]]\naction = \"reviewed\"\nsrc = \"Any\"\ndst = \"any\"\ncomment = \"x\"\n";
        let p = parse(&format!("{one}{one}")).unwrap_err();
        assert_eq!(p.len(), 1);
        assert!(
            p[0].to_string()
                .ends_with("duplicates mark 1 (the same fields)"),
            "{}",
            p[0]
        );
        let p = parse("[[mark]]\naction = \"reviewed\"\nsrc = = 1\n").unwrap_err();
        assert!(p[0].message.contains("at line 3, column 7"), "{}", p[0]);
        assert_eq!(line_col("ab\ncdé", 7), (2, 4));
        assert_eq!(line_col("ab\ncdé", 6), (2, 3), "inside a character");
    }

    #[cfg(unix)]
    #[test]
    fn writes_go_through_a_temp_file_and_are_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let mut s = store(&dir);
        s.apply(MarkOp::Add(new_mark(Action::Squelch, "any", "x")), now())
            .unwrap();
        let mode = |p: &Path| fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(s.path()), 0o600);
        assert_eq!(mode(s.path().parent().unwrap()), 0o700);
        let left: Vec<String> = fs::read_dir(s.path().parent().unwrap())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .collect();
        assert_eq!(left, vec!["marks.toml"], "no temp or lock file left");
        // A wider existing file keeps working; the next write sets 0600.
        fs::set_permissions(s.path(), fs::Permissions::from_mode(0o644)).unwrap();
        assert!(s.refresh() || s.set().is_some());
        s.apply(MarkOp::Add(new_mark(Action::Squelch, "app=x", "y")), now())
            .unwrap();
        assert_eq!(mode(s.path()), 0o600);
    }

    #[test]
    fn an_invalid_file_is_never_written() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = store(&dir);
        let bad = "[[mark]]\naction = \"squelch\"\nsrc = \"any\"\n";
        write(&s, bad);
        assert!(s.refresh());
        assert!(s.set().is_none());
        let e = s
            .apply(MarkOp::Add(new_mark(Action::Squelch, "any", "x")), now())
            .unwrap_err();
        assert!(matches!(e, MarksError::Parse(_)), "{e}");
        assert_eq!(
            e.to_string(),
            "marks.toml invalid (1 problem): mark 1: dst is required (\"any\" for any destination)"
        );
        assert_eq!(fs::read_to_string(s.path()).unwrap(), bad);
    }

    #[test]
    fn refresh_parses_only_when_the_file_changed_and_never_after_its_own_write() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = store(&dir);
        assert!(s.set().unwrap().marks.is_empty(), "missing: no marks");
        assert!(!s.refresh());
        s.apply(MarkOp::Add(new_mark(Action::Squelch, "any", "x")), now())
            .unwrap();
        let parses = s.parses();
        assert!(!s.refresh(), "its own write is not read again");
        assert_eq!(s.parses(), parses);
        let mut text = fs::read_to_string(s.path()).unwrap();
        text.push_str("\n[[mark]]\naction = \"revisit\"\nsrc = \"any\"\ndst = \"any\"\ncomment = \"by hand\"\n");
        fs::write(s.path(), text).unwrap();
        assert!(s.refresh(), "an edit elsewhere is picked up");
        assert_eq!(s.set().unwrap().marks.len(), 2);
        fs::remove_file(s.path()).unwrap();
        assert!(s.refresh());
        assert!(s.set().unwrap().marks.is_empty());
    }

    #[test]
    fn two_stores_adding_to_one_file_keep_both_marks() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = store(&dir);
        let mut b = store(&dir);
        a.apply(MarkOp::Add(new_mark(Action::Squelch, "app=a", "a")), now())
            .unwrap();
        // b has not seen a's write: the operation reloads first.
        b.apply(MarkOp::Add(new_mark(Action::Revisit, "app=b", "b")), now())
            .unwrap();
        assert_eq!(b.set().unwrap().marks.len(), 2);
        a.refresh();
        assert_eq!(a.set().unwrap().marks.len(), 2);
    }

    #[test]
    fn a_stale_lock_is_removed_and_a_live_one_refuses() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = store(&dir);
        fs::create_dir_all(s.path().parent().unwrap()).unwrap();
        let lp = sibling(s.path(), ".lock");
        let f = fs::File::create(&lp).unwrap();
        f.set_modified(SystemTime::now() - Duration::from_secs(60))
            .unwrap();
        s.apply(MarkOp::Add(new_mark(Action::Squelch, "any", "x")), now())
            .unwrap();
        assert!(!lp.exists());
        fs::File::create(&lp).unwrap();
        let e = s
            .apply(MarkOp::Add(new_mark(Action::Squelch, "app=y", "y")), now())
            .unwrap_err();
        assert_eq!(e, MarksError::Locked);
        assert_eq!(
            e.to_string(),
            "marks.toml is locked by another illumio-tui (marks.toml.lock): try again"
        );
        assert!(lp.exists(), "another writer's lock is left alone");
    }

    #[test]
    fn an_edit_of_a_mark_deleted_elsewhere_is_gone() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = store(&dir);
        let id = a
            .apply(MarkOp::Add(new_mark(Action::Squelch, "any", "x")), now())
            .unwrap()
            .ids
            .remove(0);
        let mut b = store(&dir);
        b.apply(MarkOp::Delete(vec![id.clone()]), now()).unwrap();
        let mut m = a.set().unwrap().marks[0].clone();
        m.comment = "changed".into();
        assert_eq!(
            a.apply(MarkOp::Replace(m), now()).unwrap_err(),
            MarksError::Gone(id.clone())
        );
        let ext = MarkOp::Extend {
            id: id.clone(),
            days: 30,
            comment: None,
        };
        assert_eq!(a.apply(ext, now()).unwrap_err(), MarksError::Gone(id));
    }

    #[test]
    fn delete_takes_all_or_none_and_extend_counts_from_the_later_of_now_and_the_expiry() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = store(&dir);
        let mut soon = new_mark(Action::Squelch, "app=a", "a");
        soon.expires = Some(now() + SignedDuration::from_hours(48));
        let mut old = new_mark(Action::Reviewed, "app=b", "b");
        old.expires = Some(now() - SignedDuration::from_hours(24));
        let a = s.apply(MarkOp::Add(soon), now()).unwrap().ids.remove(0);
        let b = s.apply(MarkOp::Add(old), now()).unwrap().ids.remove(0);
        let never = s
            .apply(MarkOp::Add(new_mark(Action::Critical, "app=c", "c")), now())
            .unwrap()
            .ids
            .remove(0);
        assert_eq!(
            s.apply(MarkOp::Delete(vec![a.clone(), "m-nope00".into()]), now())
                .unwrap_err(),
            MarksError::Gone("m-nope00".into())
        );
        assert_eq!(s.set().unwrap().marks.len(), 3, "nothing removed");
        let ext = |id: &str| MarkOp::Extend {
            id: id.into(),
            days: 30,
            comment: Some("still needed".into()),
        };
        s.apply(ext(&a), now()).unwrap();
        s.apply(ext(&b), now()).unwrap();
        let unchanged = s.apply(ext(&never), now()).unwrap();
        assert!(unchanged.ids.is_empty(), "a never mark stays never");
        let set = s.set().unwrap();
        let get = |id: &str| &set.marks[set.find(id).unwrap()];
        assert_eq!(
            get(&a).expires,
            Some(now() + SignedDuration::from_hours(24 * 32))
        );
        assert_eq!(
            get(&b).expires,
            Some(now() + SignedDuration::from_hours(24 * 30)),
            "revived"
        );
        assert_eq!(get(&a).comment, "still needed");
        assert_eq!(get(&never).expires, None);
        let huge = s.apply(
            MarkOp::Extend {
                id: a.clone(),
                days: i64::MAX,
                comment: None,
            },
            now(),
        );
        assert!(
            matches!(&huge, Err(MarksError::Parse(p)) if p[0].message.contains("too far")),
            "{huge:?}"
        );
        let later = now() + SignedDuration::from_hours(24 * 29);
        let done = s.apply(MarkOp::DeleteExpired, later).unwrap();
        assert!(done.removed.is_empty());
        let done = s
            .apply(
                MarkOp::DeleteExpired,
                now() + SignedDuration::from_hours(24 * 40),
            )
            .unwrap();
        assert_eq!(done.removed.len(), 2);
    }

    #[test]
    fn every_validation_rule_names_the_mark() {
        let cases: [(&str, &str); 18] = [
            ("src = { }", "src: no kind"),
            (
                "src = { labels = { app = \"a\" }, ip = \"10.0.0.1\" }",
                "several kinds (ip, labels)",
            ),
            ("src = { fqdn = \"x\" }", "unknown kind \"fqdn\""),
            ("src = { labels = { } }", "empty labels"),
            ("src = { labels = { app = [] } }", "empty list"),
            ("src = { ip = \"10.0.0.0/40\" }", "at most 32"),
            ("service = \"70000/tcp\"", "is not a port"),
            ("service = \"90-80/tcp\"", "ends before"),
            ("service = \"80/bogus\"", "unknown protocol"),
            ("service = \"22/icmp\"", "has no ports"),
            ("src = { workload = \"db-01\" }", "workload needs pce"),
            ("org = 1", "org needs pce"),
            (
                "action = \"accept-blocked\"",
                "accept-blocked needs decision",
            ),
            (
                "action = \"accept-blocked\"\ndecision = [\"blocked\", \"allowed\"]",
                "cannot include allowed",
            ),
            ("action = \"nope\"", "unknown action \"nope\""),
            ("decision = \"maybe\"", "unknown decision \"maybe\""),
            ("expires = \"tomorrow\"", "bad timestamp"),
            ("src = \"everything\"", "write \"any\""),
        ];
        for (field, want) in cases {
            let key = field.split(' ').next().unwrap();
            let mut body = format!("[[mark]]\nid = \"m-1\"\n{field}\n");
            for (k, v) in [
                ("action", "\"reviewed\""),
                ("src", "\"any\""),
                ("dst", "\"any\""),
                ("comment", "\"c\""),
            ] {
                if k != key {
                    body.push_str(&format!("{k} = {v}\n"));
                }
            }
            let e = parse(&body).unwrap_err();
            let text = e
                .iter()
                .map(|p| p.to_string())
                .collect::<Vec<_>>()
                .join("; ");
            assert!(text.contains("mark 1 (m-1): "), "{field}: {text}");
            assert!(text.contains(want), "{field}: {text}");
        }
        let dup = "[[mark]]\nid = \"m-1\"\naction = \"reviewed\"\nsrc = \"any\"\ndst = \"any\"\n[[mark]]\nid = \"m-1\"\naction = \"revisit\"\nsrc = \"any\"\ndst = \"any\"\n";
        assert_eq!(
            parse(dup).unwrap_err()[0].to_string(),
            "mark 2 (m-1): id m-1 is used by mark 1 too"
        );
        let unknown = "future = true\n[[mark]]\naction = \"reviewed\"\nsrc = \"any\"\ndst = \"any\"\nwhy = \"kept\"\n";
        assert!(parse(unknown).is_ok(), "unknown fields are not errors");
    }

    #[test]
    fn hand_written_marks_get_a_stable_derived_id() {
        let text = "[[mark]]\naction = \"squelch\"\nsrc = { ip_list = \"Corp\" }\ndst = \"any\"\ncomment = \"x\"\n";
        let a = parse(text).unwrap().set.marks[0].id.clone();
        assert!(a.starts_with("h-"), "{a}");
        assert_eq!(parse(text).unwrap().set.marks[0].id, a);
    }

    #[test]
    fn truncated_and_garbage_files_never_panic() {
        let mut inputs: Vec<String> = (0..SPEC.len())
            .filter(|i| SPEC.is_char_boundary(*i))
            .map(|i| SPEC[..i].to_string())
            .collect();
        inputs.extend(
            [
                "\u{0}\u{1}\u{2}",
                "[[mark]",
                "[[mark]]\nsrc = { labels = { app = 1 } }",
                "mark = 5",
                "[[mark]]\norg = -1\npce = \"h\"",
                "[[mark]]\nexpires = 1979-05-27",
            ]
            .map(String::from),
        );
        for text in inputs {
            let _ = parse(&text);
        }
        let e = parse("[[mark]\n").unwrap_err();
        assert_eq!(e[0].index, 0);
        assert!(e[0].to_string().starts_with("marks.toml: "), "{}", e[0]);
    }

    #[test]
    fn an_unreadable_path_is_an_io_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("marks.toml");
        fs::create_dir_all(&path).unwrap();
        let s = MarksStore::open(path);
        assert!(
            matches!(s.error(), Some(MarksError::Io(_))),
            "{:?}",
            s.error()
        );
    }

    #[test]
    fn a_store_without_a_file_reads_and_writes_nothing() {
        let mut s = MarksStore::none();
        assert!(!s.refresh());
        assert!(s.set().unwrap().marks.is_empty());
        let e = s
            .apply(MarkOp::Add(new_mark(Action::Squelch, "any", "x")), now())
            .unwrap_err();
        assert!(matches!(e, MarksError::Io(_)), "{e}");
    }

    #[test]
    fn the_path_is_next_to_the_config_file() {
        assert_eq!(
            marks_path(Some(Path::new("/etc/x/config.toml"))),
            PathBuf::from("/etc/x/marks.toml")
        );
        assert!(default_marks_path().ends_with("illumio-tui/marks.toml"));
    }
}
