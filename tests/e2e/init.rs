//! `sandbox init [folder]`: creates .sandbox/ with the two files a project owns, without Docker.

use crate::support::*;
use camino::Utf8PathBuf;
use regex::Regex;
use std::fs;

fn new_folder(f: &Fixture, name: &str) -> Utf8PathBuf {
    let dir = f.root.join(name);
    if dir.exists() {
        fs::remove_dir_all(&dir).unwrap();
    }
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn template(f: &Fixture, name: &str) -> String {
    fs::read_to_string(f.init_templates_dir().join(name))
        .unwrap()
        .replace("\r\n", "\n")
}

fn read(path: &Utf8PathBuf) -> String {
    fs::read_to_string(path).unwrap()
}

#[test]
fn an_empty_folder_gets_the_files_and_the_engine() {
    let f = Fixture::new();
    let dir = new_folder(&f, "fresh");
    let r = f.run(&["init", dir.as_str()]);
    assert_exit(&r, 0);
    assert_calls(&r, &[]);
    let sb = dir.join(".sandbox");
    assert_eq!(template(&f, "sandbox.toml"), read(&sb.join("sandbox.toml")));
    assert_eq!(
        template(&f, "allowlist.txt"),
        read(&sb.join("allowlist.txt"))
    );
    assert_eq!(read(&sb.join(".state").join(".gitignore")), "*\n");
    assert!(
        Regex::new("^[0-9a-f]{8}\n$")
            .unwrap()
            .is_match(&read(&sb.join(".state").join("id")))
    );
    assert!(
        sb.join(".state")
            .join("desktop")
            .join("Dockerfile")
            .exists()
    );
    assert!(
        sb.join(".state")
            .join("proxy")
            .join("tinyproxy.conf")
            .exists()
    );
    assert!(sb.join(".state").join(".manifest").exists());
    assert!(!sb.join("engine").exists());
    let mut files: Vec<String> = fs::read_dir(&sb)
        .unwrap()
        .map(|e| e.unwrap())
        .filter(|e| e.path().is_file())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    files.sort();
    assert_eq!(files, ["allowlist.txt", "sandbox.toml"]);
    assert_output(&r, "Created .sandbox/sandbox.toml");
    assert_output(&r, "Created .sandbox/allowlist.txt");
    assert_output(&r, "Next: edit .sandbox/sandbox.toml, then \"sandbox up\".");
    assert!(!r.output().contains("WARNING"), "{r}");
}

#[test]
fn the_new_sandbox_works_with_info() {
    let f = Fixture::new();
    let dir = new_folder(&f, "fresh");
    assert_exit(&f.run(&["init", dir.as_str()]), 0);
    let sandbox_dir = dir.join(".sandbox").to_string();
    let r = f.run_with(
        &["info"],
        RunOpts::default().sandbox_dir(sandbox_dir.clone()),
    );
    assert_exit(&r, 0);
    assert_output(&r, "Project folder");
    assert_output(&r, dir.as_str());
    let r = f.run_with(&["allowlist"], RunOpts::default().sandbox_dir(sandbox_dir));
    assert!(r.lines().iter().all(|line| line.is_empty()), "{r}");
}

#[test]
fn the_new_sandbox_header_names_the_real_commands() {
    let f = Fixture::new();
    let dir = new_folder(&f, "fresh");
    assert_exit(&f.run(&["init", dir.as_str()]), 0);
    let toml = read(&dir.join(".sandbox").join("sandbox.toml"));
    assert!(toml.contains("sandbox up"));
    assert!(toml.contains("sandbox rebuild"));
    assert!(!toml.contains(".sandbox/sandbox "));
}

#[test]
fn the_default_folder_is_the_current_directory() {
    let f = Fixture::new();
    let dir = new_folder(&f, "fresh");
    assert_exit(
        &f.run_with(&["init"], RunOpts::default().cwd(dir.clone())),
        0,
    );
    assert!(dir.join(".sandbox").join("sandbox.toml").exists());
}

#[test]
fn sandbox_dir_is_ignored() {
    let f = Fixture::new();
    let dir = new_folder(&f, "fresh");
    let before = read(&f.conf_file());
    assert_exit(
        &f.run_with(
            &["init", dir.as_str()],
            RunOpts::default().sandbox_dir(f.dir.as_str()),
        ),
        0,
    );
    assert!(dir.join(".sandbox").join("sandbox.toml").exists());
    assert_eq!(before, read(&f.conf_file()));
    assert!(!f.desktop_context().exists());
}

#[test]
fn running_it_again_keeps_the_files() {
    let f = Fixture::new();
    let dir = new_folder(&f, "fresh");
    assert_exit(&f.run(&["init", dir.as_str()]), 0);
    let toml = dir.join(".sandbox").join("sandbox.toml");
    let allow = dir.join(".sandbox").join("allowlist.txt");
    fs::write(&toml, "[project]\nname = \"mine\"\n").unwrap();
    fs::write(&allow, read(&allow) + "example.org\n").unwrap();
    let allow_before = read(&allow);
    let id = read(&dir.join(".sandbox").join(".state").join("id"));

    let r = f.run(&["init", dir.as_str()]);
    assert_exit(&r, 0);
    assert_output(&r, "sandbox.toml exists — kept");
    assert_output(&r, "allowlist.txt exists — kept");
    assert!(!r.output().contains("Created"), "{r}");
    assert_eq!(read(&toml), "[project]\nname = \"mine\"\n");
    assert_eq!(allow_before, read(&allow));
    assert_eq!(id, read(&dir.join(".sandbox").join(".state").join("id")));
}

#[test]
fn only_the_missing_file_is_created() {
    let f = Fixture::new();
    let dir = new_folder(&f, "fresh");
    assert_exit(&f.run(&["init", dir.as_str()]), 0);
    let toml = dir.join(".sandbox").join("sandbox.toml");
    fs::write(&toml, "[desktop]\ntitle = \"x\"\n").unwrap();
    fs::remove_file(dir.join(".sandbox").join("allowlist.txt")).unwrap();

    let r = f.run(&["init", dir.as_str()]);
    assert_exit(&r, 0);
    assert_output(&r, "Created .sandbox/allowlist.txt");
    assert_output(&r, "sandbox.toml exists — kept");
    assert_eq!(read(&toml), "[desktop]\ntitle = \"x\"\n");
    assert_eq!(
        template(&f, "allowlist.txt"),
        read(&dir.join(".sandbox").join("allowlist.txt"))
    );
}

#[test]
fn a_sandbox_in_a_parent_folder_is_a_warning() {
    let f = Fixture::new();
    let sub = f.project_dir.join("sub");
    fs::create_dir_all(&sub).unwrap();
    let r = f.run(&["init", sub.as_str()]);
    assert_exit(&r, 0);
    assert_output(
        &r,
        &format!(
            "{} already exists; this creates a separate sandbox for {sub}",
            f.dir
        ),
    );
    assert!(sub.join(".sandbox").join("sandbox.toml").exists());
}

#[test]
fn a_folder_that_does_not_exist_is_an_error() {
    let f = Fixture::new();
    let dir = f.root.join("nope");
    let r = f.run(&["init", dir.as_str()]);
    assert_exit(&r, 1);
    assert_output(&r, &format!("Folder {dir} does not exist."));
    assert!(!dir.exists());
}

#[test]
fn it_works_without_docker() {
    let f = Fixture::new();
    let dir = new_folder(&f, "fresh");
    let r = f.run_with(
        &["init", dir.as_str()],
        RunOpts::default().path(f.empty_bin.as_str()),
    );
    assert_exit(&r, 0);
    assert!(dir.join(".sandbox").join("sandbox.toml").exists());
}

#[test]
fn an_old_engine_folder_gets_the_usual_warning() {
    let f = Fixture::new();
    let dir = new_folder(&f, "fresh");
    fs::create_dir_all(dir.join(".sandbox").join("engine")).unwrap();
    let r = f.run(&["init", dir.as_str()]);
    assert_exit(&r, 0);
    assert_output(&r, ".sandbox/engine/ is no longer used");
    assert!(dir.join(".sandbox").join("engine").exists());
}

#[test]
fn init_is_listed_in_the_help() {
    let f = Fixture::new();
    let r = f.run(&["--help"]);
    assert_exit(&r, 0);
    assert_output(&r, "init");
    let r = f.run(&["init", "--help"]);
    assert_exit(&r, 0);
    assert_output(&r, "Project folder (default: the current directory)");
}
