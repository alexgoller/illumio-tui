use super::context::Context;
use super::error::CliError;
use super::output::TableRows;
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct HealthReport {
    pub ok: bool,
    pub host: String,
    pub org_id: u64,
    pub status: serde_json::Value,
    pub writes: bool,
}

impl TableRows for HealthReport {
    fn headers(&self) -> Vec<&'static str> {
        vec!["field", "value"]
    }

    fn rows(&self) -> Vec<Vec<String>> {
        vec![
            vec!["ok".into(), self.ok.to_string()],
            vec!["host".into(), self.host.clone()],
            vec!["org_id".into(), self.org_id.to_string()],
            vec!["status".into(), self.status.to_string()],
            vec!["writes".into(), self.writes.to_string()],
        ]
    }
}

pub async fn run(ctx: &Context) -> Result<(), CliError> {
    let status = ctx.pce.health().await?;
    let report = HealthReport {
        ok: true,
        host: ctx.config.host.clone(),
        org_id: ctx.config.org_id,
        status,
        writes: ctx.config.writes,
    };
    ctx.emit(&report, "none", false, None)
}
