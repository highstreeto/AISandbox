//! `compose ps --format json`: which containers the project has and how they are.

use anyhow::{Context, Result};

/// One container as `compose ps` reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceState {
    pub service: String,
    pub state: String,
    pub health: String,
    pub status: String,
    pub ports: String,
}

/// The services sorted by name, or an error about the unreadable output.
pub fn sorted(services: Result<Vec<ServiceState>>) -> Result<Vec<ServiceState>> {
    let mut services = services.context("Could not read the output of docker compose ps")?;
    services.sort_by(|a, b| a.service.as_bytes().cmp(b.service.as_bytes()));
    Ok(services)
}

/// Reads the output of `docker compose ps --format json`: newer compose prints one JSON object per line, older
/// versions a single array.
pub fn parse_ps(json: &str) -> Result<Vec<ServiceState>> {
    Ok(parse_entries(json)?.iter().map(to_state).collect())
}

/// The JSON objects of a `ps --format json` output: one per line, or one array.
pub fn parse_entries(json: &str) -> Result<Vec<serde_json::Value>> {
    let text = json.trim();
    let docs: Vec<&str> = if text.starts_with('[') {
        vec![text]
    } else {
        text.split('\n')
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .collect()
    };
    let mut result = Vec::new();
    for d in docs {
        match serde_json::from_str::<serde_json::Value>(d)? {
            serde_json::Value::Array(items) => result.extend(items),
            other => result.push(other),
        }
    }
    Ok(result)
}

/// One entry of compose's own JSON.
pub fn to_state(e: &serde_json::Value) -> ServiceState {
    ServiceState {
        service: text(e, "Service"),
        state: text(e, "State"),
        health: text(e, "Health"),
        status: text(e, "Status"),
        ports: format_ports(e),
    }
}

/// A missing or non-text field is "".
fn text(e: &serde_json::Value, key: &str) -> String {
    e.get(key)
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string()
}

/// The published ports of one entry, e.g. "127.0.0.1:49153→3001".
fn format_ports(e: &serde_json::Value) -> String {
    let Some(publishers) = e.get("Publishers").and_then(|p| p.as_array()) else {
        return String::new();
    };
    let mut ports: Vec<String> = Vec::new();
    for x in publishers {
        let published = x
            .get("PublishedPort")
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(0);
        if published <= 0 {
            continue;
        }
        let target = x
            .get("TargetPort")
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(0);
        let entry = format!("{}:{published}→{target}", text(x, "URL"));
        if !ports.contains(&entry) {
            ports.push(entry);
        }
    }
    ports.join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_one_object_per_line() {
        let json = "{\"Service\":\"b\",\"State\":\"running\",\"Health\":\"healthy\",\"Status\":\"Up\",\"Publishers\":[{\"URL\":\"127.0.0.1\",\"TargetPort\":3001,\"PublishedPort\":49153},{\"URL\":\"127.0.0.1\",\"TargetPort\":3001,\"PublishedPort\":49153},{\"URL\":\"::\",\"TargetPort\":80,\"PublishedPort\":0}]}\n\n{\"Service\":\"a\"}\n";
        let s = parse_ps(json).unwrap();
        assert_eq!(s.len(), 2);
        assert_eq!(s[0].ports, "127.0.0.1:49153→3001");
        assert_eq!(s[0].health, "healthy");
        assert_eq!(
            s[1],
            ServiceState {
                service: "a".into(),
                state: "".into(),
                health: "".into(),
                status: "".into(),
                ports: "".into()
            }
        );
    }

    #[test]
    fn parses_a_single_array_and_ignores_non_text_fields() {
        let s = parse_ps("[{\"Service\":\"x\",\"State\":1,\"Publishers\":null},{\"Service\":\"y\",\"Publishers\":[]}]").unwrap();
        assert_eq!(
            s.iter().map(|x| x.service.as_str()).collect::<Vec<_>>(),
            vec!["x", "y"]
        );
        assert_eq!(s[0].state, "");
        assert_eq!(s[0].ports, "");
    }

    #[test]
    fn invalid_json_is_an_error() {
        assert!(parse_ps("not json").is_err());
    }
}
