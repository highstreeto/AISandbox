//! The project this sandbox belongs to: where its `.sandbox` folder is, its identity and its settings.

use crate::config::{self, Settings};
use crate::engine::embedded;
use crate::paths::{self, full_path};
use crate::ui::Ui;
use anyhow::{Result, anyhow, bail};
use camino::{Utf8Path, Utf8PathBuf};
use regex::Regex;
use sha2::{Digest, Sha256};
use std::sync::LazyLock;

/// Set by the launchers to the `.sandbox` folder next to them.
pub const DIR_VARIABLE: &str = "SANDBOX_DIR";

const OLD_CONFIG_FILE_NAME: &str = "sandbox.yml";
const OLD_LOCAL_CONFIG_FILE_NAME: &str = "sandbox.local.yml";

static INVALID_NAME_CHARS: LazyLock<Regex> = LazyLock::new(|| Regex::new("[^a-z0-9_-]+").unwrap());
static LEADING_SEPARATORS: LazyLock<Regex> = LazyLock::new(|| Regex::new("^[-_]+").unwrap());
static TRAILING_DASHES: LazyLock<Regex> = LazyLock::new(|| Regex::new("-+$").unwrap());

#[derive(Debug, Clone)]
pub struct Project {
    /// The `.sandbox` folder.
    pub dir: Utf8PathBuf,
    /// The project folder, the parent of `dir`.
    pub project_dir: Utf8PathBuf,
    /// Created once and stored in `.state/id`: stable even if the project is moved.
    pub id: String,
    /// The docker compose project name.
    pub name: String,
    /// Where the project is mounted in the desktop.
    pub work_dir: String,
    /// From `sandbox.toml`.
    pub settings: Settings,
}

impl Project {
    /// Locates the folder, loads or creates the id, checks for old layouts and reads `sandbox.toml`.
    pub fn load(ui: &Ui) -> Result<Self> {
        let dir = locate_sandbox_dir()?;
        let project_dir = dir
            .parent()
            .map(Utf8Path::to_path_buf)
            .unwrap_or_else(|| dir.clone());
        let id = load_or_create_id(&state_dir_of(&dir), &project_dir)?;

        check_old_config(ui, &dir)?;
        check_old_layout(ui, &dir);
        let settings = config::read(
            &dir.join(config::FILE_NAME),
            &project_dir,
            &embedded::read_text("desktop/Dockerfile")?,
        )?;
        let base_name = sanitize(project_dir.file_name().unwrap_or(""));
        let name = match settings.project_name.as_deref() {
            Some(n) if !n.trim().is_empty() => sanitize(n),
            _ => sanitize(&base_name),
        };
        Ok(Self {
            work_dir: format!("/work/{base_name}-{id}"),
            dir,
            project_dir,
            id,
            name,
            settings,
        })
    }

    pub fn allowlist_path(&self) -> Utf8PathBuf {
        self.dir.join("allowlist.txt")
    }

    pub fn state_dir(&self) -> Utf8PathBuf {
        state_dir_of(&self.dir)
    }

    /// `.state/compose.yml`, the one file docker compose runs with.
    pub fn gen_path(&self) -> Utf8PathBuf {
        self.state_dir().join("compose.yml")
    }
}

/// A docker compose project name: lower case letters, digits, `_` and `-`.
pub fn sanitize(s: &str) -> String {
    let s = INVALID_NAME_CHARS
        .replace_all(&s.to_lowercase(), "-")
        .into_owned();
    let s = LEADING_SEPARATORS.replace(&s, "").into_owned();
    let s = TRAILING_DASHES.replace(&s, "").into_owned();
    if s.is_empty() {
        "sandbox".to_string()
    } else {
        s
    }
}

pub fn state_dir_of(dir: &Utf8Path) -> Utf8PathBuf {
    dir.join(".state")
}

/// The id stored in `.state/id`, created from the project folder if it isn't there yet:
/// the first 8 hex characters of SHA-256 over the project path.
pub fn load_or_create_id(state_dir: &Utf8Path, project_dir: &Utf8Path) -> Result<String> {
    paths::create_dir_all(state_dir)?;
    let id_file = state_dir.join("id");
    let mut id = if id_file.is_file() {
        paths::read_text(&id_file)?.trim().to_string()
    } else {
        String::new()
    };
    if id.is_empty() {
        let digest = Sha256::digest(project_dir.as_str().as_bytes());
        id = crate::engine::hex(&digest)[..8].to_string();
        paths::write_text(&id_file, &format!("{id}\n"))?;
    }
    Ok(id)
}

/// `.state/.gitignore` containing `*`: the state folder ignores itself, so the project's own `.gitignore` needs no
/// entry.
pub fn write_state_ignore(state_dir: &Utf8Path) -> Result<()> {
    paths::create_dir_all(state_dir)?;
    let path = state_dir.join(".gitignore");
    if !path.is_file() || paths::read_text(&path)? != "*\n" {
        paths::write_text(&path, "*\n")?;
    }
    Ok(())
}

/// `$SANDBOX_DIR` if set; otherwise searching upwards from the current directory for a folder that is (or has) a
/// `.sandbox` folder.
fn locate_sandbox_dir() -> Result<Utf8PathBuf> {
    let cwd = paths::current_dir()?;
    if let Some(from_env) = std::env::var(DIR_VARIABLE).ok().filter(|v| !v.is_empty()) {
        let full = full_path(&cwd, &from_env);
        if is_sandbox_dir(&full) {
            return Ok(paths::physical(&full));
        }
        bail!(
            "{DIR_VARIABLE} is {full}, which is not a .sandbox folder (with {}).",
            config::FILE_NAME
        );
    }
    for d in cwd.ancestors() {
        if is_sandbox_dir(d) {
            return Ok(paths::physical(d));
        }
        let sub = d.join(".sandbox");
        if is_sandbox_dir(&sub) {
            return Ok(paths::physical(&sub));
        }
    }
    Err(anyhow!(
        "Could not find the .sandbox folder (with {}).",
        config::FILE_NAME
    ))
}

fn is_sandbox_dir(dir: &Utf8Path) -> bool {
    dir.join(config::FILE_NAME).is_file() || dir.join(OLD_CONFIG_FILE_NAME).is_file()
}

/// `.sandbox/engine/` from before the engine was built into the app: a warning, nothing is deleted (the files are
/// committed and may hold the project's own edits).
pub fn check_old_layout(ui: &Ui, dir: &Utf8Path) {
    if dir.join("engine").is_dir() {
        ui.warn(
            ".sandbox/engine/ is no longer used \u{2014} the engine is built into the app (see .sandbox/.state/desktop/ and proxy/). \
             Delete it, together with .sandbox/README.md, .gitattributes and .gitignore.",
        );
    }
}

/// sandbox.yml / sandbox.local.yml from before sandbox.toml: an error with the key mapping, or a warning.
fn check_old_config(ui: &Ui, dir: &Utf8Path) -> Result<()> {
    let has_toml = dir.join(config::FILE_NAME).is_file();
    let has_yml = dir.join(OLD_CONFIG_FILE_NAME).is_file();
    if has_yml && !has_toml {
        bail!(
            "sandbox.yml is no longer read. Move its settings to sandbox.toml (see README.md):\n\
             \x20 x-sandbox.project_name / workspace_mode / protect_git / ui_port\n\
             \x20                             \u{2192} [project] name / workspace / protect_git / ui_port\n\
             \x20 build.args INSTALL_<TOOL>   \u{2192} [tools] <tool> (lower case, e.g. INSTALL_CLAUDE_CODE \u{2192} claude_code)\n\
             \x20 environment TITLE / SANDBOX_SUDO / TZ / CUSTOM_USER + PASSWORD\n\
             \x20                             \u{2192} [desktop] title / sudo / timezone / auth\n\
             \x20 other environment entries   \u{2192} [env]\n\
             \x20 mem_limit / cpus / devices / security_opt\n\
             \x20                             \u{2192} [desktop] memory / cpus / devices / security_opt\n\
             \x20 volumes (extra folders)     \u{2192} [[share]] host / path / readonly\n\
             Then delete sandbox.yml."
        );
    }
    if has_yml {
        ui.warn("sandbox.yml is ignored (settings are read from sandbox.toml). Delete it.");
    }
    if dir.join(OLD_LOCAL_CONFIG_FILE_NAME).is_file() {
        ui.warn(
            "sandbox.local.yml is no longer read. Move its settings to sandbox.toml and delete it.",
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_makes_a_compose_project_name() {
        assert_eq!(sanitize("Other Name"), "other-name");
        assert_eq!(sanitize("demo-project"), "demo-project");
        assert_eq!(sanitize("__My.Project!!"), "my-project");
        assert_eq!(sanitize("a--b__c"), "a--b__c");
        assert_eq!(sanitize("a  b"), "a-b");
        assert_eq!(sanitize("..."), "sandbox");
        assert_eq!(sanitize(""), "sandbox");
        assert_eq!(sanitize("-_-x-"), "x");
    }

    #[test]
    fn a_new_id_is_eight_hex_characters_of_the_path_hash_and_kept() {
        let tmp = tempfile::tempdir().unwrap();
        let state = Utf8Path::from_path(tmp.path()).unwrap().join(".state");
        let id = load_or_create_id(&state, Utf8Path::new("/work/demo")).unwrap();
        // SHA-256("/work/demo") starts with this
        let expected = crate::engine::hex(&Sha256::digest(b"/work/demo"))[..8].to_string();
        assert_eq!(id, expected);
        assert_eq!(
            std::fs::read_to_string(state.join("id")).unwrap(),
            format!("{id}\n")
        );
        std::fs::write(state.join("id"), "\u{feff}cafebabe\r\n").unwrap();
        assert_eq!(
            load_or_create_id(&state, Utf8Path::new("/other")).unwrap(),
            "cafebabe"
        );
    }
}
