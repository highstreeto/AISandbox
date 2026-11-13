//! `.state/.manifest`: what the engine sync wrote last time.

use crate::paths;
use camino::Utf8Path;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// * `app`: the app's version (for people: bug reports, `info`).
/// * `engine`: one SHA-256 over the embedded `desktop/` and `proxy/` files.
/// * `files`: SHA-256 of every written file as it is on disk, keyed by path below `.state/`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct EngineManifest {
    #[serde(default)]
    pub app: String,
    #[serde(default)]
    pub engine: String,
    #[serde(default)]
    pub files: BTreeMap<String, String>,
}

impl EngineManifest {
    /// The manifest at `path`; an empty one if it is missing or unreadable (the next sync repairs it).
    pub fn read(path: &Utf8Path) -> Self {
        if !path.is_file() {
            return Self::default();
        }
        paths::read_text(path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    /// The manifest as written to disk: indented JSON with sorted keys and a final newline.
    pub fn to_json(&self) -> String {
        let mut json = serde_json::to_string_pretty(self).unwrap_or_default();
        json.push('\n');
        json
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_sorted_indented_json_with_a_final_newline() {
        let m = EngineManifest {
            app: "0.1.0+abc".into(),
            engine: "e".into(),
            files: BTreeMap::from([
                ("proxy/b".to_string(), "2".to_string()),
                ("desktop/a".to_string(), "1".to_string()),
            ]),
        };
        assert_eq!(
            m.to_json(),
            "{\n  \"app\": \"0.1.0+abc\",\n  \"engine\": \"e\",\n  \"files\": {\n    \"desktop/a\": \"1\",\n    \"proxy/b\": \"2\"\n  }\n}\n"
        );
    }

    #[test]
    fn reads_old_escaped_plus_and_tolerates_garbage() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = Utf8Path::from_path(tmp.path()).unwrap();
        std::fs::write(
            dir.join("m"),
            "{\"app\": \"0.1.0\\u002B1\", \"files\": {\"a\": \"h\"}}",
        )
        .unwrap();
        let m = EngineManifest::read(&dir.join("m"));
        assert_eq!(m.app, "0.1.0+1");
        assert_eq!(m.files["a"], "h");
        std::fs::write(dir.join("bad"), "not json").unwrap();
        assert_eq!(
            EngineManifest::read(&dir.join("bad")),
            EngineManifest::default()
        );
        assert_eq!(
            EngineManifest::read(&dir.join("missing")),
            EngineManifest::default()
        );
    }
}
