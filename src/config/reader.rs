//! One TOML table: typed getters remember which keys were read; `done` reports the rest as unknown.
//!
//! Not `serde` with `deny_unknown_fields` on purpose: the error texts (`unknown key 'project.workspace_mode' (known:
//! name, workspace, protect_git, ui_port)`, `must be true or false (got "maybe")`) can't be produced that way.

use super::{error, format_number};
use anyhow::Result;
use toml::{Table, Value};

#[derive(Debug)]
pub struct TableReader<'a> {
    table: &'a Table,
    path: String,
    /// Keys read, in the order they were read; that order is the "known" list in messages.
    used: Vec<String>,
}

impl<'a> TableReader<'a> {
    /// `path` is the table's dotted name in messages ("" for the root).
    pub fn new(table: &'a Table, path: &str) -> Self {
        Self {
            table,
            path: path.to_string(),
            used: Vec::new(),
        }
    }

    fn mark(&mut self, key: &str) {
        if !self.used.iter().any(|k| k == key) {
            self.used.push(key.to_string());
        }
    }

    /// All keys, in file order; they all count as read.
    pub fn keys(&mut self) -> Vec<String> {
        let keys: Vec<String> = self.table.keys().cloned().collect();
        for k in &keys {
            self.mark(k);
        }
        keys
    }

    pub fn raw(&mut self, key: &str) -> Option<&'a Value> {
        self.mark(key);
        self.table.get(key)
    }

    pub fn str(&mut self, key: &str) -> Result<Option<&'a str>> {
        match self.raw(key) {
            None => Ok(None),
            Some(Value::String(s)) => Ok(Some(s)),
            Some(v) => Err(error(format!(
                "{} must be a text in quotes (got {}).",
                self.name(key),
                show(v)
            ))),
        }
    }

    pub fn bool(&mut self, key: &str) -> Result<Option<bool>> {
        match self.raw(key) {
            None => Ok(None),
            Some(Value::Boolean(b)) => Ok(Some(*b)),
            Some(v) => Err(error(format!(
                "{} must be true or false (got {}).",
                self.name(key),
                show(v)
            ))),
        }
    }

    pub fn int(&mut self, key: &str) -> Result<Option<i64>> {
        match self.raw(key) {
            None => Ok(None),
            Some(Value::Integer(n)) => Ok(Some(*n)),
            Some(v) => Err(error(format!(
                "{} must be a whole number (got {}).",
                self.name(key),
                show(v)
            ))),
        }
    }

    pub fn number(&mut self, key: &str) -> Result<Option<f64>> {
        match self.raw(key) {
            None => Ok(None),
            Some(Value::Integer(n)) => Ok(Some(*n as f64)),
            Some(Value::Float(d)) => Ok(Some(*d)),
            Some(v) => Err(error(format!(
                "{} must be a number (got {}).",
                self.name(key),
                show(v)
            ))),
        }
    }

    /// A text, number or true/false, as text (environment values).
    pub fn scalar(&mut self, key: &str) -> Result<String> {
        match self.raw(key) {
            Some(Value::String(s)) => Ok(s.clone()),
            Some(Value::Boolean(b)) => Ok(b.to_string()),
            Some(Value::Integer(n)) => Ok(n.to_string()),
            Some(Value::Float(d)) => Ok(format_number(*d)),
            other => Err(error(format!(
                "{} must be a text, number or true/false (got {}).",
                self.name(key),
                other.map_or("nothing".to_string(), show)
            ))),
        }
    }

    pub fn str_list(&mut self, key: &str) -> Result<Vec<String>> {
        match self.raw(key) {
            None => Ok(Vec::new()),
            Some(Value::Array(a)) if a.iter().all(Value::is_str) => Ok(a
                .iter()
                .filter_map(|x| x.as_str().map(str::to_string))
                .collect()),
            Some(v) => Err(error(format!(
                "{} must be a list of texts, e.g. [\"a\", \"b\"] (got {}).",
                self.name(key),
                show(v)
            ))),
        }
    }

    pub fn opt_table(&mut self, key: &str) -> Result<Option<TableReader<'a>>> {
        match self.raw(key) {
            None => Ok(None),
            Some(Value::Table(t)) => Ok(Some(TableReader::new(t, &self.name(key)))),
            Some(v) => Err(error(format!(
                "{} must be a table (got {}).",
                self.name(key),
                show(v)
            ))),
        }
    }

    /// A table that may be missing (then it is empty).
    pub fn table(&mut self, key: &str) -> Result<TableReader<'a>> {
        Ok(match self.opt_table(key)? {
            Some(t) => t,
            None => TableReader::new(empty_table(), &self.name(key)),
        })
    }

    /// `[[key]]` blocks (or an inline list of tables, which is the same TOML value).
    pub fn table_array(&mut self, key: &str) -> Result<Vec<TableReader<'a>>> {
        match self.raw(key) {
            None => Ok(Vec::new()),
            Some(Value::Array(a)) if a.iter().all(Value::is_table) => Ok(a
                .iter()
                .filter_map(Value::as_table)
                .map(|t| TableReader::new(t, &self.name(key)))
                .collect()),
            Some(v) => Err(error(format!(
                "{} must be written as [[{key}]] blocks (got {}).",
                self.name(key),
                show(v)
            ))),
        }
    }

    /// Unknown keys are errors, with the known ones listed.
    /// `what` is "key" for a section's keys, "table" for the root's sections.
    pub fn done(&self, what: &str) -> Result<()> {
        let Some(unknown) = self.table.keys().find(|k| !self.used.contains(k)) else {
            return Ok(());
        };
        let known = if self.used.is_empty() {
            String::new()
        } else {
            format!(" (known: {})", self.used.join(", "))
        };
        Err(error(if what == "table" {
            format!("unknown setting '{unknown}'{known}.")
        } else {
            format!("unknown key '{}'{known}.", self.name(unknown))
        }))
    }

    fn name(&self, key: &str) -> String {
        if self.path.is_empty() {
            key.to_string()
        } else {
            format!("{}.{key}", self.path)
        }
    }
}

fn empty_table() -> &'static Table {
    static EMPTY: std::sync::OnceLock<Table> = std::sync::OnceLock::new();
    EMPTY.get_or_init(Table::new)
}

/// A value as it is shown in messages: `"text"`, `true`, `5`, `1.5`, `a list`, `a table`.
pub fn show(v: &Value) -> String {
    match v {
        Value::String(s) => format!("\"{s}\""),
        Value::Boolean(b) => b.to_string(),
        Value::Integer(n) => n.to_string(),
        Value::Float(d) => format_number(*d),
        Value::Array(_) => "a list".to_string(),
        Value::Table(_) => "a table".to_string(),
        Value::Datetime(d) => d.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table(text: &str) -> Table {
        text.parse().unwrap()
    }

    #[test]
    fn typed_getters_and_messages() {
        let t = table("a = \"x\"\nb = 1\nc = true\nd = 1.5\ne = [1]\n");
        let mut r = TableReader::new(&t, "p");
        assert_eq!(r.str("a").unwrap(), Some("x"));
        assert_eq!(r.str("missing").unwrap(), None);
        assert_eq!(
            r.str("b").unwrap_err().to_string(),
            "sandbox.toml: p.b must be a text in quotes (got 1)."
        );
        assert_eq!(
            r.bool("a").unwrap_err().to_string(),
            "sandbox.toml: p.a must be true or false (got \"x\")."
        );
        assert_eq!(
            r.int("d").unwrap_err().to_string(),
            "sandbox.toml: p.d must be a whole number (got 1.5)."
        );
        assert_eq!(r.number("b").unwrap(), Some(1.0));
        assert_eq!(
            r.scalar("e").unwrap_err().to_string(),
            "sandbox.toml: p.e must be a text, number or true/false (got a list)."
        );
        assert_eq!(r.scalar("c").unwrap(), "true");
        assert_eq!(r.scalar("d").unwrap(), "1.5");
    }

    #[test]
    fn unknown_keys_list_the_known_ones_in_read_order() {
        let t = table("zeta = 1\nname = \"x\"\nother = 2\n");
        let mut r = TableReader::new(&t, "project");
        r.str("name").unwrap();
        r.bool("protect_git").unwrap();
        assert_eq!(
            r.done("key").unwrap_err().to_string(),
            "sandbox.toml: unknown key 'project.zeta' (known: name, protect_git)."
        );
    }

    #[test]
    fn keys_count_as_read() {
        let t = table("b = 1\na = 2\n");
        let mut r = TableReader::new(&t, "env");
        assert_eq!(r.keys(), vec!["b", "a"]);
        r.done("key").unwrap();
    }

    #[test]
    fn table_arrays_accept_blocks_and_inline_lists() {
        let t = table("share = [{ host = \"a\" }, { host = \"b\" }]\n");
        assert_eq!(
            TableReader::new(&t, "").table_array("share").unwrap().len(),
            2
        );
        let t = table("share = { host = \"a\" }\n");
        assert_eq!(
            TableReader::new(&t, "")
                .table_array("share")
                .map(|_| ())
                .unwrap_err()
                .to_string(),
            "sandbox.toml: share must be written as [[share]] blocks (got a table)."
        );
        let t = table("share = [1]\n");
        assert!(TableReader::new(&t, "").table_array("share").is_err());
    }
}
