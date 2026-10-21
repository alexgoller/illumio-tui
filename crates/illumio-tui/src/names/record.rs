//! [`Recorder`]: the [`pce::ListObserver`] that keeps `names.json` up to
//! date from the lists the CLI and the TUI read anyway (a fresh read, not
//! a cache hit). A write that fails is ignored: the names cache never
//! fails a command, and never causes a request.
use super::Update;
use pce::models::{IpList, Label, LabelGroup, Service, Workload};
use std::path::PathBuf;
use std::sync::Mutex;

pub struct Recorder {
    path: PathBuf,
    /// The label keys an app is made of (`app`, `env`): `shop/prod`.
    scope_keys: Vec<String>,
    /// One read-merge-write at a time (the TUI reads lists concurrently).
    lock: Mutex<()>,
}

impl Recorder {
    pub fn new(path: PathBuf, scope_keys: Vec<String>) -> Self {
        Self {
            path,
            scope_keys,
            lock: Mutex::new(()),
        }
    }

    fn record(&self, update: Update, complete: bool) {
        let _held = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        let _ = super::record(&self.path, update, complete, jiff::Timestamp::now());
    }

    /// The workload's app, its scope label values joined with `/`; `None`
    /// when it lacks one (such an app cannot be named on the command line).
    fn app(&self, w: &Workload) -> Option<String> {
        let values: Option<Vec<&str>> = self
            .scope_keys
            .iter()
            .map(|k| {
                w.labels
                    .iter()
                    .find(|l| l.key.as_deref() == Some(k.as_str()))
                    .and_then(|l| l.value.as_deref())
            })
            .collect();
        values.filter(|v| !v.is_empty()).map(|v| v.join("/"))
    }
}

/// Draft objects pending deletion are not offered.
fn live(update_type: &Option<String>) -> bool {
    update_type.as_deref() != Some("delete")
}

impl pce::ListObserver for Recorder {
    fn labels(&self, labels: &[Label], complete: bool) {
        let live: Vec<&Label> = labels.iter().filter(|l| !l.is_deleted()).collect();
        let update = Update::Labels {
            labels: live
                .iter()
                .map(|l| format!("{}={}", l.key, l.value))
                .collect(),
            keys: live.iter().map(|l| l.key.clone()).collect(),
        };
        self.record(update, complete);
    }

    fn label_groups(&self, groups: &[LabelGroup], complete: bool) {
        let names = groups
            .iter()
            .filter(|g| live(&g.update_type))
            .map(|g| g.name.clone())
            .collect();
        self.record(Update::LabelGroups(names), complete);
    }

    fn ip_lists(&self, lists: &[IpList], complete: bool) {
        let names = lists
            .iter()
            .filter(|l| live(&l.update_type))
            .map(|l| l.name.clone())
            .collect();
        self.record(Update::IpLists(names), complete);
    }

    fn services(&self, services: &[Service], complete: bool) {
        let names = services
            .iter()
            .filter(|s| live(&s.update_type))
            .map(|s| s.name.clone())
            .collect();
        self.record(Update::Services(names), complete);
    }

    fn workloads(&self, workloads: &[Workload], complete: bool) {
        let live: Vec<&Workload> = workloads
            .iter()
            .filter(|w| w.deleted != Some(true))
            .collect();
        let update = Update::Workloads {
            hostnames: live
                .iter()
                .filter_map(|w| {
                    let some = |s: &Option<String>| s.clone().filter(|s| !s.is_empty());
                    some(&w.hostname).or_else(|| some(&w.name))
                })
                .collect(),
            apps: live.iter().filter_map(|w| self.app(w)).collect(),
        };
        self.record(update, complete);
    }
}
