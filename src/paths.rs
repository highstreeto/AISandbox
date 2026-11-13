//! Paths as the .NET app saw them: lexical `full_path`, `physical` (last component only), UTF-8 text files.

use anyhow::{Context, Result, anyhow};
use camino::{Utf8Component, Utf8Path, Utf8PathBuf};
use std::fs;
use std::path::PathBuf;

/// Converts a path from the OS to UTF-8; project paths are hashed, printed and written into YAML, so they must be text.
pub fn to_utf8(p: PathBuf) -> Result<Utf8PathBuf> {
    Utf8PathBuf::from_path_buf(p)
        .map_err(|p| anyhow!("The path {} is not valid UTF-8.", p.display()))
}

/// The current directory as UTF-8.
pub fn current_dir() -> Result<Utf8PathBuf> {
    to_utf8(std::env::current_dir().context("Could not read the current directory")?)
}

/// Like `Path.GetFullPath(Path.Combine(base, p))`: joins and removes `.` and `..` **without touching the disk**.
/// (`std::path::absolute` keeps `..`, `fs::canonicalize` resolves symlinks.)
pub fn full_path(base: &Utf8Path, p: &str) -> Utf8PathBuf {
    normalize(&base.join(p))
}

/// Folds `.` and `..` over the components of a path, lexically.
pub fn normalize(p: &Utf8Path) -> Utf8PathBuf {
    let mut out = Utf8PathBuf::new();
    for c in p.components() {
        match c {
            Utf8Component::CurDir => {}
            Utf8Component::ParentDir => match out.components().next_back() {
                Some(Utf8Component::Normal(_)) => {
                    out.pop();
                }
                Some(Utf8Component::RootDir | Utf8Component::Prefix(_)) => {}
                _ => out.push(".."),
            },
            other => out.push(other.as_str()),
        }
    }
    if out.as_str().is_empty() {
        out.push(".");
    }
    out
}

/// Like `pwd -P` for the last component only: while the path is a symlink, follows it (a relative target is taken
/// against the link's folder). Parent folders are not resolved.
pub fn physical(path: &Utf8Path) -> Utf8PathBuf {
    let mut current = normalize(path);
    for _ in 0..40 {
        let is_link = fs::symlink_metadata(&current).is_ok_and(|m| m.file_type().is_symlink());
        if !is_link {
            break;
        }
        let Ok(target) = fs::read_link(&current)
            .map_err(anyhow::Error::from)
            .and_then(to_utf8)
        else {
            break;
        };
        let parent = current
            .parent()
            .map(Utf8Path::to_path_buf)
            .unwrap_or_default();
        current = full_path(&parent, target.as_str());
    }
    current
}

/// Reads a UTF-8 text file; a leading BOM is dropped (as `File.ReadAllText` does). Invalid UTF-8 is an error.
pub fn read_text(path: &Utf8Path) -> Result<String> {
    let bytes = fs::read(path).with_context(|| format!("Could not read {path}"))?;
    let text = String::from_utf8(bytes).map_err(|_| anyhow!("{path} is not valid UTF-8."))?;
    Ok(match text.strip_prefix('\u{feff}') {
        Some(rest) => rest.to_string(),
        None => text,
    })
}

/// Writes UTF-8 text without BOM.
pub fn write_text(path: &Utf8Path, text: &str) -> Result<()> {
    fs::write(path, text).with_context(|| format!("Could not write {path}"))
}

/// Creates a folder and its parents.
pub fn create_dir_all(path: &Utf8Path) -> Result<()> {
    fs::create_dir_all(path).with_context(|| format!("Could not create {path}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_path_is_lexical() {
        let base = Utf8Path::new("/tmp/x/demo/.sandbox");
        assert_eq!(
            full_path(base, "../docs"),
            Utf8PathBuf::from("/tmp/x/demo/docs")
        );
        assert_eq!(
            full_path(base, "./a/./b/../c"),
            Utf8PathBuf::from("/tmp/x/demo/.sandbox/a/c")
        );
        assert_eq!(full_path(base, "/abs/../etc"), Utf8PathBuf::from("/etc"));
        assert_eq!(
            full_path(base, "../../../../../../.."),
            Utf8PathBuf::from("/")
        );
        assert_eq!(
            full_path(base, "."),
            Utf8PathBuf::from("/tmp/x/demo/.sandbox")
        );
        assert_eq!(
            full_path(base, "dir/"),
            Utf8PathBuf::from("/tmp/x/demo/.sandbox/dir")
        );
    }

    #[cfg(unix)]
    #[test]
    fn physical_follows_only_the_last_component() {
        let tmp = tempfile::tempdir().unwrap();
        let root = to_utf8(tmp.path().canonicalize().unwrap()).unwrap();
        fs::create_dir_all(root.join("real/sub")).unwrap();
        std::os::unix::fs::symlink("real", root.join("link")).unwrap();
        std::os::unix::fs::symlink("link", root.join("link2")).unwrap();
        assert_eq!(physical(&root.join("link")), root.join("real"));
        assert_eq!(physical(&root.join("link2")), root.join("real"));
        // a link in the middle is kept as it is
        assert_eq!(physical(&root.join("link/sub")), root.join("link/sub"));
        assert_eq!(physical(&root.join("missing")), root.join("missing"));
    }

    #[test]
    fn read_text_drops_the_bom_and_rejects_invalid_utf8() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = Utf8Path::from_path(tmp.path()).unwrap();
        fs::write(dir.join("a"), "\u{feff}hello").unwrap();
        assert_eq!(read_text(&dir.join("a")).unwrap(), "hello");
        fs::write(dir.join("b"), [0xff, 0xfe, 0x41]).unwrap();
        assert!(
            read_text(&dir.join("b"))
                .unwrap_err()
                .to_string()
                .contains("not valid UTF-8")
        );
    }
}
