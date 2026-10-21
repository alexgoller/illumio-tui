//! [`Recorder`]: the [`pce::ListObserver`] that keeps `names.json` up to
//! date from the lists the CLI and the TUI read anyway (a fresh read, not
//! a cache hit). The file is read, merged and written on a thread of its
//! own, in the order the reads came, never on the async runtime (a
//! workloads list can be 200000 rows). A write that fails is ignored: the
//! names cache never fails a command, and never causes a request.
use super::{Origin, Update};
use pce::models::{IpList, Label, LabelGroup, Service, Workload};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Sender};
use std::thread::JoinHandle;

enum Job {
    Record(Update, bool),
    /// Answered once every earlier job is done.
    Flush(Sender<()>),
}

pub struct Recorder {
    path: PathBuf,
    /// The label keys an app is made of (`app`, `env`): `shop/prod`.
    scope_keys: Vec<String>,
    /// To the writer thread; `None` when it could not be started (then
    /// nothing is written).
    jobs: Option<Sender<Job>>,
    writer: Option<JoinHandle<()>>,
}

impl Recorder {
    pub fn new(path: PathBuf, origin: Origin, scope_keys: Vec<String>) -> Self {
        let (tx, rx) = mpsc::channel::<Job>();
        let file = path.clone();
        let writer = std::thread::Builder::new()
            .name("names-cache".into())
            .spawn(move || {
                for job in rx {
                    match job {
                        Job::Record(update, complete) => {
                            let now = jiff::Timestamp::now();
                            let _ = super::record(&file, &origin, update, complete, now);
                        }
                        Job::Flush(done) => {
                            let _ = done.send(());
                        }
                    }
                }
            })
            .ok();
        Self {
            path,
            scope_keys,
            jobs: writer.as_ref().map(|_| tx),
            writer,
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Blocks until every update handed over so far is written (or has
    /// failed to be). Not for the async runtime's threads.
    pub fn flush(&self) {
        let Some(jobs) = &self.jobs else { return };
        let (tx, rx) = mpsc::channel();
        if jobs.send(Job::Flush(tx)).is_ok() {
            let _ = rx.recv();
        }
    }

    fn record(&self, update: Update, complete: bool) {
        if let Some(jobs) = &self.jobs {
            let _ = jobs.send(Job::Record(update, complete));
        }
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

/// The writer finishes what it was handed before the recorder goes (the
/// CLI exits right after its command).
impl Drop for Recorder {
    fn drop(&mut self) {
        drop(self.jobs.take());
        if let Some(writer) = self.writer.take() {
            let _ = writer.join();
        }
    }
}

/// Draft objects pending deletion are not offered.
fn live(update_type: &Option<String>) -> bool {
    update_type.as_deref() != Some("delete")
}

/// A value the helper can print on one line.
fn one_line(v: &str) -> bool {
    !v.contains(['\n', '\r'])
}

/// `values` without those that would not fit on one line.
fn lines(values: impl IntoIterator<Item = String>) -> Vec<String> {
    values.into_iter().filter(|v| one_line(v)).collect()
}

impl pce::ListObserver for Recorder {
    fn labels(&self, labels: &[Label], complete: bool) {
        let live: Vec<&Label> = labels
            .iter()
            .filter(|l| !l.is_deleted() && one_line(&l.key))
            .collect();
        let update = Update::Labels {
            labels: lines(live.iter().map(|l| format!("{}={}", l.key, l.value))),
            keys: live.iter().map(|l| l.key.clone()).collect(),
        };
        self.record(update, complete);
    }

    fn label_groups(&self, groups: &[LabelGroup], complete: bool) {
        let names = groups
            .iter()
            .filter(|g| live(&g.update_type))
            .map(|g| g.name.clone());
        self.record(Update::LabelGroups(lines(names)), complete);
    }

    fn ip_lists(&self, lists: &[IpList], complete: bool) {
        let names = lists
            .iter()
            .filter(|l| live(&l.update_type))
            .map(|l| l.name.clone());
        self.record(Update::IpLists(lines(names)), complete);
    }

    fn services(&self, services: &[Service], complete: bool) {
        let names = services
            .iter()
            .filter(|s| live(&s.update_type))
            .map(|s| s.name.clone());
        self.record(Update::Services(lines(names)), complete);
    }

    fn workloads(&self, workloads: &[Workload], complete: bool) {
        let live: Vec<&Workload> = workloads
            .iter()
            .filter(|w| w.deleted != Some(true))
            .collect();
        let update = Update::Workloads {
            hostnames: lines(live.iter().filter_map(|w| {
                let some = |s: &Option<String>| s.clone().filter(|s| !s.is_empty());
                some(&w.hostname).or_else(|| some(&w.name))
            })),
            apps: lines(live.iter().filter_map(|w| self.app(w))),
        };
        self.record(update, complete);
    }
}
