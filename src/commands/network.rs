//! The proxy's allowlist: allow, deny, allowlist, blocked, reload.

use crate::allowlist::{AllowOutcome, Allowlist};
use crate::docker::Docker;
use crate::project::Project;
use crate::ui::{Ui, table};
use anyhow::{Result, bail};
use regex::Regex;
use std::collections::HashMap;
use std::sync::LazyLock;

/// tinyproxy's log line for a host that isn't on the allowlist.
static REFUSED: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"refused on filtered (?:domain|url) "([^"]+)""#).unwrap());

pub fn allow(d: &Docker, hosts: &[String]) -> Result<i32> {
    let ui = d.ui;
    let mut list = Allowlist::load(&d.project.allowlist_path())?;
    let hosts = validate_all(hosts)?;
    let mut t = ui.new_table(&["Host", "Result"]);
    for h in hosts {
        let result = match list.allow(h) {
            AllowOutcome::AlreadyAllowed => ui.dim("already allowed"),
            AllowOutcome::Uncommented => format!(
                "{} {}",
                ui.green("allowed"),
                ui.dim("(enabled existing entry)")
            ),
            AllowOutcome::Added => format!("{} {}", ui.green("allowed"), ui.dim("(added)")),
        };
        t.add_row([h.to_string(), result]);
    }
    list.save()?;
    ui.print_table(None, &t, None);
    reload_proxy(d)
}

pub fn deny(d: &Docker, hosts: &[String]) -> Result<i32> {
    let ui = d.ui;
    let mut list = Allowlist::load(&d.project.allowlist_path())?;
    let hosts = validate_all(hosts)?;
    let mut t = ui.new_table(&["Host", "Result"]);
    for h in hosts {
        let result = if list.deny(h) {
            ui.red("denied")
        } else {
            ui.dim("was not allowed")
        };
        t.add_row([h.to_string(), result]);
    }
    list.save()?;
    ui.print_table(None, &t, None);
    reload_proxy(d)
}

/// All hosts are checked before anything changes: one bad host leaves the file untouched.
fn validate_all(hosts: &[String]) -> Result<Vec<&str>> {
    hosts.iter().map(|h| Allowlist::validate_host(h)).collect()
}

pub fn show_allowlist(ui: &Ui, project: &Project, all: bool) -> Result<i32> {
    let path = project.allowlist_path();
    let entries: Vec<_> = Allowlist::load(&path)?
        .entries()
        .into_iter()
        .filter(|e| all || e.active)
        .collect();
    if !ui.interactive {
        // plain output for scripts: one active host per line
        for e in entries.iter().filter(|e| e.active) {
            println!("{}", e.host);
        }
        return Ok(0);
    }
    let mut t = if all {
        ui.new_table(&["Host", "Group", "State", "Note"])
    } else {
        ui.new_table(&["Host", "Group"])
    };
    for e in &entries {
        let host = if e.active {
            ui.bold(&e.host)
        } else {
            ui.dim(&e.host)
        };
        if all {
            t.add_row([
                host,
                e.group.clone(),
                if e.active {
                    ui.green("allowed")
                } else {
                    ui.dim("off")
                },
                ui.dim(&e.note),
            ]);
        } else {
            t.add_row([host, e.group.clone()]);
        }
    }
    let active = entries.iter().filter(|e| e.active).count();
    let available = if all {
        format!(", {} available", entries.len() - active)
    } else {
        String::new()
    };
    let caption = ui.dim(format!("{active} allowed{available} \u{b7} {path}"));
    ui.print_table(None, &t, Some(&caption));
    Ok(0)
}

pub fn blocked(d: &Docker) -> Result<i32> {
    let ui = d.ui;
    let logs = ui.spin("Reading proxy logs…", || {
        d.compose(&["logs", "--no-log-prefix", "proxy"])
    });
    let hits = refused_hosts(&format!("{}\n{}", logs.stdout, logs.stderr));
    if hits.is_empty() {
        ui.success("Nothing was blocked (since the proxy last started).");
        return Ok(0);
    }

    let allowed = Allowlist::load(&d.project.allowlist_path())?;
    let mut t = ui.new_table(&["Count", "Host", "Status"]);
    table::align_right(&mut t, 0);
    for (host, count) in &hits {
        t.add_row([
            count.to_string(),
            host.clone(),
            if allowed.is_allowed(host) {
                ui.green("allowed")
            } else {
                ui.red("blocked")
            },
        ]);
    }
    ui.print_table(None, &t, None);
    println!(
        "{}",
        ui.dim(format!("(allow one with: {} allow <host>)", ui.self_name()))
    );
    Ok(0)
}

/// Hosts the proxy refused with how often (case-insensitive, first spelling kept), most often first, then by name.
fn refused_hosts(log: &str) -> Vec<(String, usize)> {
    let mut order: Vec<String> = Vec::new();
    let mut counts: HashMap<String, (String, usize)> = HashMap::new();
    for m in REFUSED.captures_iter(log) {
        let host = m[1].to_string();
        let key = host.to_lowercase();
        counts
            .entry(key.clone())
            .and_modify(|e| e.1 += 1)
            .or_insert_with(|| {
                order.push(key);
                (host, 1)
            });
    }
    let mut hits: Vec<(String, usize)> = order
        .into_iter()
        .filter_map(|k| counts.remove(&k))
        .collect();
    hits.sort_by(|a, b| {
        b.1.cmp(&a.1)
            .then_with(|| a.0.as_bytes().cmp(b.0.as_bytes()))
    });
    hits
}

pub fn reload_proxy(d: &Docker) -> Result<i32> {
    let ui = d.ui;
    let r = ui.spin("Reloading the proxy…", || {
        d.compose(&["restart", "proxy"])
    });
    if r.exit_code != 0 {
        ui.show_output("docker compose restart proxy", &r);
        bail!("Could not restart the proxy.");
    }
    ui.success("Proxy reloaded.");
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_refused_hosts_most_often_first() {
        let log = "NOTICE x: Proxying refused on filtered domain \"evil.example.com\"\n\
                   NOTICE x: Proxying refused on filtered domain \"EVIL.example.com\"\n\
                   NOTICE x: Proxying refused on filtered url \"api.nuget.org\"\n\
                   NOTICE x: Proxying refused on filtered domain \"a.example.com\"\n\
                   CONNECT x: Connect (file descriptor 7): 172.18.0.3\n";
        assert_eq!(
            refused_hosts(log),
            vec![
                ("evil.example.com".to_string(), 2),
                ("a.example.com".to_string(), 1),
                ("api.nuget.org".to_string(), 1)
            ]
        );
        assert!(refused_hosts("nothing").is_empty());
    }
}
