//! `allowlist.txt`: the proxy's host allowlist, edited line by line so comments and groups are kept.

use crate::paths;
use anyhow::{Result, bail};
use camino::{Utf8Path, Utf8PathBuf};
use regex::Regex;
use std::sync::LazyLock;

static HOST: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[A-Za-z0-9*][A-Za-z0-9*._:-]*$").unwrap());
static GROUP: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^#\s*---\s*(.+?)\s*-*\s*$").unwrap());
static DISABLED: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^#\s*([A-Za-z0-9*][A-Za-z0-9*._:-]*\.[A-Za-z0-9*._:-]+)\s*(?:#\s*(.*))?$").unwrap()
});

/// One host in `allowlist.txt`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AllowEntry {
    /// Host name or wildcard pattern.
    pub host: String,
    /// The `# --- group ---` header it is under ("" above the first one).
    pub group: String,
    /// Allowed (false: commented out, ready to enable).
    pub active: bool,
    /// The comment after a commented-out entry.
    pub note: String,
}

/// What [`Allowlist::allow`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AllowOutcome {
    AlreadyAllowed,
    Uncommented,
    Added,
}

#[derive(Debug)]
pub struct Allowlist {
    path: Utf8PathBuf,
    /// The edit buffer, written back by `save`.
    lines: Vec<String>,
}

impl Allowlist {
    pub fn load(path: &Utf8Path) -> Result<Self> {
        if !path.is_file() {
            bail!("{path} not found.");
        }
        Ok(Self::parse(path, &paths::read_text(path)?))
    }

    fn parse(path: &Utf8Path, text: &str) -> Self {
        let mut lines: Vec<String> = text
            .replace("\r\n", "\n")
            .split('\n')
            .map(str::to_string)
            .collect();
        if lines.last().is_some_and(String::is_empty) {
            lines.pop();
        }
        Self {
            path: path.to_path_buf(),
            lines,
        }
    }

    /// Writes the file back with LF line endings and no BOM.
    pub fn save(&self) -> Result<()> {
        paths::write_text(&self.path, &format!("{}\n", self.lines.join("\n")))
    }

    pub fn validate_host(h: &str) -> Result<&str> {
        if HOST.is_match(h) {
            Ok(h)
        } else {
            bail!("Not a valid host name: '{h}'.")
        }
    }

    pub fn allow(&mut self, host: &str) -> AllowOutcome {
        if self.lines.iter().any(|l| enables(l, host)) {
            return AllowOutcome::AlreadyAllowed;
        }
        let commented =
            Regex::new(&format!(r"(?i)^#\s*({}(\s.*)?)$", regex::escape(host))).unwrap();
        if let Some(i) = self.lines.iter().position(|l| commented.is_match(l)) {
            let enabled = commented
                .captures(&self.lines[i])
                .map(|c| c[1].to_string())
                .unwrap_or_default();
            self.lines[i] = enabled;
            return AllowOutcome::Uncommented;
        }
        self.lines.push(host.to_string());
        AllowOutcome::Added
    }

    /// Comments out every active line for the host; false if there was none.
    pub fn deny(&mut self, host: &str) -> bool {
        let mut any = false;
        for line in &mut self.lines {
            if enables(line, host) {
                *line = format!("# {line}");
                any = true;
            }
        }
        any
    }

    /// Active entries, and commented-out ones below the first `# --- group ---` header.
    pub fn entries(&self) -> Vec<AllowEntry> {
        let mut entries = Vec::new();
        let mut group: Option<String> = None; // disabled entries are only recognised below the first group header
        for raw in &self.lines {
            let line = raw.trim();
            if line.is_empty() {
                continue;
            }
            if let Some(g) = GROUP.captures(line) {
                group = Some(g[1].to_string());
                continue;
            }
            if let Some(host) = active_host(line) {
                entries.push(AllowEntry {
                    host: host.to_string(),
                    group: group.clone().unwrap_or_default(),
                    active: true,
                    note: String::new(),
                });
            } else if let (Some(g), Some(d)) = (&group, DISABLED.captures(line)) {
                entries.push(AllowEntry {
                    host: d[1].to_string(),
                    group: g.clone(),
                    active: false,
                    note: d.get(2).map_or(String::new(), |m| m.as_str().to_string()),
                });
            }
        }
        entries
    }

    /// Same matching as tinyproxy's fnmatch filter: exact host or shell-style wildcard, case-insensitive.
    pub fn is_allowed(&self, host: &str) -> bool {
        self.entries().iter().filter(|e| e.active).any(|e| {
            let pattern = regex::escape(&e.host)
                .replace("\\*", ".*")
                .replace("\\?", ".");
            Regex::new(&format!("(?i)^{pattern}$")).is_ok_and(|re| re.is_match(host))
        })
    }
}

/// The host pattern a line enables, read the way tinyproxy reads its filter file: leading whitespace is skipped, a
/// line starting with `#` is a comment, and the pattern ends at the first whitespace or `#`. `allow`, `deny`,
/// `allowlist` and `blocked` all use this, so they agree with each other and with the proxy (an indented
/// `  example.com` or `example.com#note` is active, and `deny` comments it out).
fn active_host(line: &str) -> Option<&str> {
    let line = line.trim_start();
    if line.starts_with('#') {
        return None;
    }
    let end = line
        .find(|c: char| c.is_whitespace() || c == '#')
        .unwrap_or(line.len());
    Some(&line[..end]).filter(|h| !h.is_empty())
}

/// Whether the line is an active entry for exactly this host (case-insensitive).
fn enables(line: &str, host: &str) -> bool {
    active_host(line).is_some_and(|h| h.eq_ignore_ascii_case(host))
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEXT: &str = "# header\n\n# --- Claude (required) ---\napi.anthropic.com\nclaude.ai   # trailing\n*.claudeusercontent.com\n\n# --- NuGet ---\n# api.nuget.org\n# globalcdn.nuget.org # the CDN\n#   plain comment words\n";

    fn list() -> Allowlist {
        Allowlist::parse(Utf8Path::new("allowlist.txt"), TEXT)
    }

    #[test]
    fn loads_and_saves_lines_with_lf_and_a_final_newline() {
        let l = Allowlist::parse(Utf8Path::new("x"), "a\r\nb\r\n");
        assert_eq!(l.lines, vec!["a", "b"]);
        let l = Allowlist::parse(Utf8Path::new("x"), "a\n\n\n");
        assert_eq!(l.lines, vec!["a", "", ""]);
        let l = Allowlist::parse(Utf8Path::new("x"), "");
        assert!(l.lines.is_empty());
    }

    #[test]
    fn validates_host_names() {
        for ok in ["example.com", "*.example.com", "localhost:8080", "a-b.c_d"] {
            assert!(Allowlist::validate_host(ok).is_ok(), "{ok}");
        }
        for bad in ["bad host!", "", "-x", ".x", "a/b"] {
            assert_eq!(
                Allowlist::validate_host(bad).unwrap_err().to_string(),
                format!("Not a valid host name: '{bad}'.")
            );
        }
    }

    #[test]
    fn allow_adds_uncomments_and_recognises_existing() {
        let mut l = list();
        assert_eq!(l.allow("claude.ai"), AllowOutcome::AlreadyAllowed);
        assert_eq!(l.allow("CLAUDE.AI"), AllowOutcome::AlreadyAllowed);
        assert_eq!(l.allow("api.nuget.org"), AllowOutcome::Uncommented);
        assert!(l.lines.contains(&"api.nuget.org".to_string()));
        assert!(!l.lines.contains(&"# api.nuget.org".to_string()));
        assert_eq!(l.allow("globalcdn.nuget.org"), AllowOutcome::Uncommented);
        assert!(
            l.lines
                .contains(&"globalcdn.nuget.org # the CDN".to_string())
        );
        assert_eq!(l.allow("new.example.com"), AllowOutcome::Added);
        assert_eq!(l.lines.last().unwrap(), "new.example.com");
    }

    #[test]
    fn deny_comments_out_every_active_line() {
        let mut l = list();
        assert!(l.deny("claude.ai"));
        assert!(l.lines.contains(&"# claude.ai   # trailing".to_string()));
        assert!(!l.deny("claude.ai"));
        assert!(!l.deny("not-there.example.com"));
        let mut l = Allowlist::parse(Utf8Path::new("x"), "a.com\na.com extra\nb.com\n");
        assert!(l.deny("a.com"));
        assert_eq!(l.lines, vec!["# a.com", "# a.com extra", "b.com"]);
    }

    #[test]
    fn indented_lines_and_comments_without_a_space_are_active_and_can_be_denied() {
        let mut l = Allowlist::parse(
            Utf8Path::new("x"),
            "# --- G ---\n  evil.com\nother.com#note\n\t# off.com\n",
        );
        let active: Vec<String> = l
            .entries()
            .into_iter()
            .filter(|e| e.active)
            .map(|e| e.host)
            .collect();
        assert_eq!(active, vec!["evil.com", "other.com"]);
        assert!(l.is_allowed("evil.com") && l.is_allowed("other.com"));
        assert_eq!(l.allow("EVIL.com"), AllowOutcome::AlreadyAllowed);
        assert_eq!(l.allow("other.com"), AllowOutcome::AlreadyAllowed);

        assert!(l.deny("evil.com"));
        assert!(l.deny("other.com"));
        assert!(!l.is_allowed("evil.com") && !l.is_allowed("other.com"));
        assert_eq!(l.lines[1], "#   evil.com");
        assert_eq!(l.lines[2], "# other.com#note");
        // a longer host that starts with the same text is a different entry
        let mut l = Allowlist::parse(Utf8Path::new("x"), "evil.com.example.org\n");
        assert!(!l.deny("evil.com"));
    }

    #[test]
    fn entries_know_groups_notes_and_disabled_lines_only_below_a_header() {
        let e = list().entries();
        let got: Vec<(&str, &str, bool, &str)> = e
            .iter()
            .map(|x| (x.host.as_str(), x.group.as_str(), x.active, x.note.as_str()))
            .collect();
        assert_eq!(
            got,
            vec![
                ("api.anthropic.com", "Claude (required)", true, ""),
                ("claude.ai", "Claude (required)", true, ""),
                ("*.claudeusercontent.com", "Claude (required)", true, ""),
                ("api.nuget.org", "NuGet", false, ""),
                ("globalcdn.nuget.org", "NuGet", false, "the CDN"),
            ]
        );
        let above = Allowlist::parse(Utf8Path::new("x"), "# example.com\nreal.com\n").entries();
        assert_eq!(above.len(), 1);
        assert_eq!(above[0].group, "");
    }

    #[test]
    fn is_allowed_uses_wildcards_case_insensitively() {
        let l = list();
        assert!(l.is_allowed("API.anthropic.com"));
        assert!(l.is_allowed("x.claudeusercontent.com"));
        assert!(!l.is_allowed("claudeusercontent.com"));
        assert!(!l.is_allowed("api.nuget.org"));
        assert!(!l.is_allowed("evil.example.com"));
    }
}
