//! The files under `resources/` that are compiled into the binary: the engine (compose template, `desktop/` and
//! `proxy/` build folders) and the `init/` templates.
//!
//! Names are the path below `resources/` with forward slashes on every OS (`desktop/tools/node.sh`), sorted by
//! bytes. This is the only module that touches the embedded tree.

use anyhow::{Result, anyhow};
use include_dir::{Dir, include_dir};

static RESOURCES: Dir = include_dir!("$CARGO_MANIFEST_DIR/resources");

/// All embedded file names that start with `prefix`, sorted by bytes.
pub fn names(prefix: &str) -> Vec<String> {
    let mut names = Vec::new();
    collect(&RESOURCES, &mut names);
    names.retain(|n| n.starts_with(prefix));
    names.sort_unstable_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
    names
}

fn collect(dir: &Dir<'_>, out: &mut Vec<String>) {
    for file in dir.files() {
        out.push(file.path().to_string_lossy().replace('\\', "/"));
    }
    for sub in dir.dirs() {
        collect(sub, out);
    }
}

pub fn read_bytes(name: &str) -> Result<&'static [u8]> {
    RESOURCES
        .get_file(name)
        .map(include_dir::File::contents)
        .ok_or_else(|| anyhow!("Embedded file {name} is missing."))
}

/// UTF-8 text with line endings normalised to `\n`.
pub fn read_text(name: &str) -> Result<String> {
    let text = std::str::from_utf8(read_bytes(name)?)
        .map_err(|_| anyhow!("Embedded file {name} is not UTF-8."))?;
    Ok(text.replace("\r\n", "\n").replace('\r', "\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_nested_files_with_forward_slashes_in_byte_order() {
        let all = names("");
        assert!(all.contains(&"desktop/tools/node.sh".to_string()));
        assert!(all.contains(&"init/sandbox.toml".to_string()));
        assert!(all.iter().all(|n| !n.contains('\\')));
        let mut sorted = all.clone();
        sorted.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
        assert_eq!(all, sorted);
        assert!(names("desktop/").iter().all(|n| n.starts_with("desktop/")));
    }

    #[test]
    fn text_has_unix_line_endings() {
        assert!(!read_text("desktop/Dockerfile").unwrap().contains('\r'));
        assert!(read_text("nope").is_err());
    }
}
