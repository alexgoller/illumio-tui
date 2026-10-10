//! Developer tasks. `cargo xtask record` pulls a small, anonymised sample
//! of each object type from the configured PCE into fixtures/recorded/.
use clap::{Parser, Subcommand};
use pce::client::{ClientConfig, PceClient};
use pce::models::PolicyScope;
use pce::traffic::TrafficQueryBuilder;
use pce::{CacheMode, Credentials, Pce};
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio_util::sync::CancellationToken;

#[derive(Parser)]
#[command(name = "xtask")]
struct Xtask {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Record anonymised fixtures from the PCE named by PCE_HOST etc.
    Record {
        #[arg(long)]
        env_file: Option<PathBuf>,
        #[arg(long, default_value = "fixtures/recorded")]
        out: PathBuf,
    },
}

/// Replaces hostnames, IPs, FQDNs, workload names, process user names and
/// descriptions with stable placeholders. Label keys/values and rule names
/// are kept.
#[derive(Default)]
struct Anonymiser {
    hosts: HashMap<String, String>,
    ips: HashMap<String, String>,
    fqdns: HashMap<String, String>,
    users: HashMap<String, String>,
}

impl Anonymiser {
    fn map(
        table: &mut HashMap<String, String>,
        raw: &str,
        make: impl Fn(usize) -> String,
    ) -> String {
        let n = table.len() + 1;
        table
            .entry(raw.to_string())
            .or_insert_with(|| make(n))
            .clone()
    }

    fn ip(&mut self, raw: &str) -> String {
        if raw == "0.0.0.0/0" || raw == "::/0" || raw.is_empty() {
            return raw.to_string();
        }
        let (ip, suffix) = raw
            .split_once('/')
            .map(|(a, b)| (a, format!("/{b}")))
            .unwrap_or((raw, String::new()));
        let is_v6 = ip.contains(':');
        let mapped = Self::map(&mut self.ips, ip, |n| {
            if is_v6 {
                format!("fd00::{n:x}")
            } else {
                format!("10.{}.{}.{}", (n >> 16) & 255, (n >> 8) & 255, n & 255)
            }
        });
        format!("{mapped}{suffix}")
    }

    /// Windows process paths embed the account: `C:\Users\<name>\...`.
    fn process_name(&mut self, raw: &str) -> String {
        const MARK: &str = "\\Users\\";
        let Some(i) = raw.find(MARK) else {
            return raw.to_string();
        };
        let start = i + MARK.len();
        let end = raw[start..].find('\\').map_or(raw.len(), |j| start + j);
        let user = Self::map(&mut self.users, &raw[start..end], |n| format!("user-{n}"));
        format!("{}{user}{}", &raw[..start], &raw[end..])
    }

    fn walk(&mut self, v: &mut Value) {
        match v {
            Value::Object(map) => {
                let has_hostname = map.contains_key("hostname");
                let has_user = map.contains_key("username") || map.contains_key("email");
                let keys: Vec<String> = map.keys().cloned().collect();
                for k in keys {
                    let child = map.get_mut(&k).expect("key exists");
                    match (k.as_str(), &mut *child) {
                        ("hostname", Value::String(s)) if !s.is_empty() => {
                            *s = Self::map(&mut self.hosts, s, |n| format!("host-{n}.example"))
                        }
                        ("fqdn" | "active_pce_fqdn" | "target_pce_fqdn", Value::String(s))
                            if !s.is_empty() =>
                        {
                            *s = Self::map(&mut self.fqdns, s, |n| format!("fqdn-{n}.example"))
                        }
                        (
                            "ip"
                            | "address"
                            | "from_ip"
                            | "to_ip"
                            | "ip_address"
                            | "default_gateway_address",
                            Value::String(s),
                        ) => *s = self.ip(s),
                        ("name", Value::String(s)) if has_hostname && !s.is_empty() => {
                            *s = Self::map(&mut self.hosts, s, |n| format!("host-{n}.example"))
                        }
                        ("username" | "email", Value::String(s)) if !s.is_empty() => {
                            *s = Self::map(&mut self.users, s, |n| format!("user-{n}"))
                        }
                        ("name", Value::String(s)) if has_user && !s.is_empty() => {
                            *s = Self::map(&mut self.users, s, |n| format!("user-{n}"))
                        }
                        ("user_name", Value::String(s)) if !s.is_empty() => {
                            *s = Self::map(&mut self.users, s, |n| format!("user-{n}"))
                        }
                        ("process_name", Value::String(s)) => *s = self.process_name(s),
                        ("description", Value::String(s)) => s.clear(),
                        ("public_ip", Value::String(s)) => *s = self.ip(s),
                        _ => self.walk(child),
                    }
                }
            }
            Value::Array(items) => {
                for item in items {
                    self.walk(item);
                }
            }
            _ => {}
        }
    }
}

fn env_map(env_file: Option<&PathBuf>) -> HashMap<String, String> {
    let mut m: HashMap<String, String> = HashMap::new();
    if let Some(p) = env_file {
        for item in dotenvy::from_path_iter(p).expect("env file") {
            let (k, v) = item.expect("env line");
            m.insert(k, v);
        }
    }
    m.extend(std::env::vars());
    m
}

fn write(out: &Path, name: &str, value: &Value) {
    let path = out.join(name);
    std::fs::write(&path, serde_json::to_string_pretty(value).unwrap())
        .unwrap_or_else(|e| panic!("write {}: {e}", path.display()));
    let rows = value.as_array().map(Vec::len).unwrap_or(1);
    println!("wrote {} ({rows} rows)", path.display());
}

async fn raw_list(pce: &Pce, path: &str, max: u32) -> Value {
    pce.client()
        .get::<Value>(path, &[("max_results", max.to_string())])
        .await
        .expect(path)
        .body
}

#[tokio::main]
async fn main() {
    let args = Xtask::parse();
    match args.cmd {
        Cmd::Record { env_file, out } => {
            let env = env_map(env_file.as_ref());
            let get = |k: &str| env.get(k).cloned().unwrap_or_else(|| panic!("{k} not set"));
            let mut cfg = ClientConfig::new(
                get("PCE_HOST"),
                get("PCE_ORG_ID").parse().expect("PCE_ORG_ID"),
                Credentials::new(get("API_KEY"), get("API_SECRET")),
            );
            if let Some(p) = env.get("PCE_PORT") {
                cfg.port = p.parse().expect("PCE_PORT");
            }
            let pce = Pce::new(PceClient::new(cfg).expect("client"));
            let org = pce.client().org_path("");
            std::fs::create_dir_all(&out).expect("out dir");
            let mut anon = Anonymiser::default();

            let mut sets: Vec<(&str, Value)> = vec![
                (
                    "labels.json",
                    raw_list(&pce, &format!("{org}labels"), 500).await,
                ),
                (
                    "label_groups.json",
                    raw_list(&pce, &format!("{org}sec_policy/draft/label_groups"), 100).await,
                ),
                (
                    "ip_lists.json",
                    raw_list(&pce, &format!("{org}sec_policy/draft/ip_lists"), 100).await,
                ),
                (
                    "services.json",
                    raw_list(&pce, &format!("{org}sec_policy/draft/services"), 100).await,
                ),
                (
                    "workloads.json",
                    raw_list(&pce, &format!("{org}workloads"), 25).await,
                ),
                (
                    "events.json",
                    raw_list(&pce, &format!("{org}events"), 100).await,
                ),
                (
                    "rule_sets.json",
                    raw_list(
                        &pce,
                        &format!("{org}sec_policy/{}/rule_sets", PolicyScope::Draft.segment()),
                        10,
                    )
                    .await,
                ),
            ];
            let mut deny = Vec::new();
            if let Some(rs) = sets[6].1.as_array() {
                for r in rs.iter().take(3) {
                    if let Some(href) = r.get("href").and_then(Value::as_str) {
                        match pce.deny_rules(href, CacheMode::Bypass).await {
                            Ok(d) => deny.extend(
                                d.value
                                    .into_iter()
                                    .map(|x| serde_json::to_value(x).unwrap()),
                            ),
                            Err(e) => eprintln!("deny rules for {href}: {e}"),
                        }
                    }
                }
            }
            sets.push(("deny_rules.json", Value::Array(deny)));
            let pending = pce
                .client()
                .get::<Value>(&format!("{org}sec_policy/pending"), &[])
                .await
                .map(|r| r.body)
                .unwrap_or(Value::Array(vec![]));
            sets.push(("pending.json", pending));

            let q = TrafficQueryBuilder::last(Duration::from_secs(3600))
                .unwrap()
                .max_results(50)
                .build()
                .unwrap();
            let flows = pce
                .traffic(&q, CacheMode::Bypass, &CancellationToken::new(), None)
                .await
                .expect("traffic")
                .value
                .flows;
            sets.push(("traffic_flows.json", serde_json::to_value(flows).unwrap()));

            for (name, mut value) in sets {
                anon.walk(&mut value);
                write(&out, name, &value);
            }
            let mut summary = Map::new();
            summary.insert("api_calls".into(), Value::from(pce.stats().calls));
            summary.insert("bytes".into(), Value::from(pce.stats().bytes));
            println!("{}", Value::Object(summary));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn user_names_and_windows_account_paths_share_placeholders() {
        let mut v = json!([
            {"service": {"user_name": "alice", "process_name": "C:\\Users\\alice\\a.exe"}},
            {"service": {"user_name": "NT AUTHORITY\\SYSTEM", "process_name": "svchost.exe"}},
            {"service": {"user_name": "alice", "process_name": "/usr/bin/sshd"}},
            {"service": {"user_name": "", "process_name": "C:\\Users\\bob\\b.exe"}}
        ]);
        let mut anon = Anonymiser::default();
        anon.walk(&mut v);
        assert_eq!(v[0]["service"]["user_name"], "user-1");
        assert_eq!(v[0]["service"]["process_name"], "C:\\Users\\user-1\\a.exe");
        assert_eq!(v[1]["service"]["user_name"], "user-2");
        assert_eq!(v[1]["service"]["process_name"], "svchost.exe");
        assert_eq!(v[2]["service"]["user_name"], "user-1");
        assert_eq!(v[2]["service"]["process_name"], "/usr/bin/sshd");
        assert_eq!(v[3]["service"]["user_name"], "");
        assert_eq!(v[3]["service"]["process_name"], "C:\\Users\\user-3\\b.exe");
    }

    #[test]
    fn event_actors_are_anonymised() {
        let mut v = json!([
            {"created_by": {"user": {"href": "/users/1", "username": "alice@corp.com", "name": "Alice A"}}},
            {"created_by": {"user": {"username": "alice@corp.com"}}},
            {"created_by": {"agent": {"hostname": "web01"}}},
            {"resource_changes": [{"resource": {"label": {"href": "/orgs/5/labels/1", "name": "Web"}}}]}
        ]);
        let mut anon = Anonymiser::default();
        anon.walk(&mut v);
        let username = v[0]["created_by"]["user"]["username"].clone();
        let name = v[0]["created_by"]["user"]["name"].clone();
        assert!(username.as_str().unwrap().starts_with("user-"));
        assert!(name.as_str().unwrap().starts_with("user-"));
        assert_ne!(username, name);
        assert_eq!(v[1]["created_by"]["user"]["username"], username);
        assert_eq!(v[2]["created_by"]["agent"]["hostname"], "host-1.example");
        let label = &v[3]["resource_changes"][0]["resource"]["label"]["name"];
        assert_eq!(label, "Web", "plain names are left alone");
    }
}
