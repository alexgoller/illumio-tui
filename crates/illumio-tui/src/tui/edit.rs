//! Editing in the TUI. A key produces an [`EditIntent`]: a pure value that
//! names what the user wants, with the selected object cloned in.
use pce::models::{IpList, Label, Service, Workload};

/// What `n`, `e` or `x` on Objects or Workloads asks for.
#[derive(Debug, Clone, PartialEq)]
pub enum EditIntent {
    CreateLabel,
    UpdateLabel(Label),
    DeleteLabel(Label),
    CreateIpList,
    UpdateIpList(IpList),
    DeleteIpList(IpList),
    CreateService,
    UpdateService(Service),
    DeleteService(Service),
    CreateWorkload,
    UpdateWorkload(Workload),
    DeleteWorkload(Workload),
}
