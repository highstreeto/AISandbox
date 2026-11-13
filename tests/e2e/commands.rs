//! Every command, run against a healthy fake docker: the exact docker calls, in order.

use crate::support::*;
use regex::Regex;
use rstest::rstest;
use std::fs;

const CLAUDE_SCRIPT: &str = "command -v claude >/dev/null || { echo \"Claude Code CLI not installed: set claude_code under [tools] in sandbox.toml and rebuild.\"; exit 1; }; exec claude \"$@\"";

fn up_wait(f: &Fixture) -> Call {
    f.compose(&["up", "-d", "--wait", "--wait-timeout", "300"])
}

fn port(f: &Fixture) -> Call {
    f.compose(&["port", "gateway", "3001"])
}

fn ps(f: &Fixture) -> Call {
    f.compose(&["ps", "-a", "--format", "json"])
}

fn lines_of(path: &camino::Utf8Path) -> Vec<String> {
    fs::read_to_string(path)
        .unwrap()
        .lines()
        .map(str::to_string)
        .collect()
}

// --- lifecycle --------------------------------------------------------------

#[rstest]
#[case("up")]
#[case("start")]
fn up_builds_starts_and_prints_the_url(#[case] cmd: &str) {
    let f = Fixture::new();
    let r = f.run(&[cmd, "--no-open"]);
    assert_exit(&r, 0);
    assert_calls(
        &r,
        &[
            docker_info(),
            f.collision_check(PROJECT_NAME),
            f.compose_interactive(&["build"]),
            up_wait(&f),
            port(&f),
        ],
    );
    assert_output(&r, URL);
    assert_output(&r, &f.work_dir());
    assert!(f.generated_file().exists());
}

#[cfg(unix)] // Windows opens URLs through `cmd /C start`
#[test]
fn up_opens_the_browser() {
    let f = Fixture::new();
    let r = f.run_with(&["up"], RunOpts::default().wait_for("xdg-open"));
    assert_exit(&r, 0);
    assert_calls(
        &r,
        &[
            docker_info(),
            f.collision_check(PROJECT_NAME),
            f.compose_interactive(&["build"]),
            up_wait(&f),
            port(&f),
            call("xdg-open", &[URL]),
        ],
    );
}

#[rstest]
#[case("stop")]
#[case("down")]
#[case("restart")]
fn stop_down_restart(#[case] cmd: &str) {
    let f = Fixture::new();
    let r = f.run(&[cmd]);
    assert_exit(&r, 0);
    assert_calls(&r, &[docker_info(), f.collision_check(PROJECT_NAME), f.compose(&[cmd])]);
    assert_output(&r, ": done.");
}

#[rstest]
#[case("status")]
#[case("ps")]
fn status_lists_services_and_url(#[case] cmd: &str) {
    let f = Fixture::new();
    let r = f.run(&[cmd]);
    assert_exit(&r, 0);
    assert_calls(&r, &[docker_info(), f.collision_check(PROJECT_NAME), ps(&f), port(&f)]);
    for s in [
        "desktop",
        "gateway",
        "proxy",
        "healthy",
        "127.0.0.1:49153→3001",
    ] {
        assert_output(&r, s);
    }
    assert_output(&r, &format!("URL: {URL}"));
}

#[test]
fn status_without_containers() {
    let f = Fixture::new();
    let r = f.run_with(
        &["status"],
        RunOpts::default().rules(vec![rule("ps -a --format json")]),
    );
    assert_exit(&r, 0);
    assert_calls(&r, &[docker_info(), f.collision_check(PROJECT_NAME), ps(&f)]);
    assert_output(&r, "No containers for 'demo-project'");
}

#[cfg(unix)] // Windows opens URLs through `cmd /C start`
#[test]
fn open_asks_for_the_port_and_opens_the_browser() {
    let f = Fixture::new();
    let r = f.run_with(&["open"], RunOpts::default().wait_for("xdg-open"));
    assert_exit(&r, 0);
    assert_calls(&r, &[docker_info(), f.collision_check(PROJECT_NAME), port(&f), call("xdg-open", &[URL])]);
}

#[test]
fn shell_execs_bash_in_the_project_folder() {
    let f = Fixture::new();
    let r = f.run(&["shell"]);
    assert_exit(&r, 0);
    assert_calls(
        &r,
        &[
            docker_info(),
            f.collision_check(PROJECT_NAME),
            f.compose_interactive(&[
                "exec",
                "-u",
                "abc",
                "-w",
                &f.work_dir(),
                "desktop",
                "bash",
                "-l",
            ]),
        ],
    );
}

#[rstest]
#[case(&[])]
#[case(&["--resume", "-p", "hi there"])]
#[case(&["--help"])]
#[case(&["-v", "help", "--", "x y"])]
fn claude_forwards_all_arguments_verbatim(#[case] args: &[&str]) {
    let f = Fixture::new();
    let mut cmd = vec!["claude"];
    cmd.extend_from_slice(args);
    let r = f.run(&cmd);
    assert_exit(&r, 0);
    let work_dir = f.work_dir();
    let mut expected = vec![
        "exec",
        "-u",
        "abc",
        "-w",
        &work_dir,
        "desktop",
        "sh",
        "-c",
        CLAUDE_SCRIPT,
        "claude",
    ];
    expected.extend_from_slice(args);
    assert_calls(&r, &[docker_info(), f.collision_check(PROJECT_NAME), f.compose_interactive(&expected)]);
}

#[test]
fn logs_follow_by_default() {
    let f = Fixture::new();
    let r = f.run(&["logs"]);
    assert_exit(&r, 0);
    assert_calls(
        &r,
        &[
            docker_info(),
            f.collision_check(PROJECT_NAME),
            f.compose_interactive(&["logs", "--tail=100", "-f"]),
        ],
    );
}

#[test]
fn logs_with_services_tail_and_no_follow() {
    let f = Fixture::new();
    let r = f.run(&["logs", "proxy", "desktop", "-n", "5", "--no-follow"]);
    assert_exit(&r, 0);
    assert_calls(
        &r,
        &[
            docker_info(),
            f.collision_check(PROJECT_NAME),
            f.compose_interactive(&["logs", "--tail=5", "proxy", "desktop"]),
        ],
    );
}

// --- network ----------------------------------------------------------------

#[test]
fn allow_uncomments_an_entry_and_reloads_the_proxy() {
    let f = Fixture::new();
    assert!(lines_of(&f.allowlist_file()).contains(&"# api.nuget.org".to_string()));
    let r = f.run(&["allow", "api.nuget.org", "new.example.com", "claude.ai"]);
    assert_exit(&r, 0);
    assert_calls(&r, &[docker_info(), f.collision_check(PROJECT_NAME), f.compose(&["restart", "proxy"])]);
    let lines = lines_of(&f.allowlist_file());
    assert!(lines.contains(&"api.nuget.org".to_string()));
    assert!(!lines.contains(&"# api.nuget.org".to_string()));
    assert_eq!(lines.last().unwrap(), "new.example.com");
    assert_eq!(lines.iter().filter(|l| *l == "claude.ai").count(), 1);
    assert_output(&r, "enabled existing entry");
    assert_output(&r, "added");
    assert_output(&r, "already allowed");
    assert_output(&r, "Proxy reloaded.");
}

#[test]
fn deny_comments_an_entry_and_reloads_the_proxy() {
    let f = Fixture::new();
    let r = f.run(&["deny", "claude.ai", "not-there.example.com"]);
    assert_exit(&r, 0);
    assert_calls(&r, &[docker_info(), f.collision_check(PROJECT_NAME), f.compose(&["restart", "proxy"])]);
    let lines = lines_of(&f.allowlist_file());
    assert!(lines.contains(&"# claude.ai".to_string()));
    assert!(!lines.contains(&"claude.ai".to_string()));
    assert_output(&r, "denied");
    assert_output(&r, "was not allowed");
}

#[test]
fn reload_restarts_the_proxy() {
    let f = Fixture::new();
    let r = f.run(&["reload"]);
    assert_exit(&r, 0);
    assert_calls(&r, &[docker_info(), f.collision_check(PROJECT_NAME), f.compose(&["restart", "proxy"])]);
}

#[test]
fn blocked_counts_refused_hosts_from_the_proxy_log() {
    let f = Fixture::new();
    let log = [
        "NOTICE    Nov 11 10:00:00 [1]: Proxying refused on filtered domain \"evil.example.com\"",
        "NOTICE    Nov 11 10:00:01 [1]: Proxying refused on filtered domain \"evil.example.com\"",
        "NOTICE    Nov 11 10:00:02 [1]: Proxying refused on filtered url \"api.nuget.org\"",
        "CONNECT   Nov 11 10:00:03 [1]: Connect (file descriptor 7): 172.18.0.3",
    ]
    .join("\n");
    let r = f.run_with(
        &["blocked"],
        RunOpts::default().rules(vec![rule("logs --no-log-prefix proxy").stdout(log)]),
    );
    assert_exit(&r, 0);
    assert_calls(
        &r,
        &[
            docker_info(),
            f.collision_check(PROJECT_NAME),
            f.compose(&["logs", "--no-log-prefix", "proxy"]),
        ],
    );
    assert!(
        Regex::new(r"2\s*│\s*evil\.example\.com\s*│\s*blocked")
            .unwrap()
            .is_match(&r.stdout),
        "{r}"
    );
    assert!(
        Regex::new(r"1\s*│\s*api\.nuget\.org\s*│\s*blocked")
            .unwrap()
            .is_match(&r.stdout),
        "{r}"
    );
}

#[test]
fn blocked_with_nothing_blocked() {
    let f = Fixture::new();
    let r = f.run(&["blocked"]);
    assert_exit(&r, 0);
    assert_calls(
        &r,
        &[
            docker_info(),
            f.collision_check(PROJECT_NAME),
            f.compose(&["logs", "--no-log-prefix", "proxy"]),
        ],
    );
    assert_output(&r, "Nothing was blocked");
}

#[rstest]
#[case(&[])]
#[case(&["-a"])]
#[case(&["--all"])]
fn allowlist_does_not_call_docker(#[case] args: &[&str]) {
    let f = Fixture::new();
    let mut cmd = vec!["allowlist"];
    cmd.extend_from_slice(args);
    let r = f.run(&cmd);
    assert_exit(&r, 0);
    assert_calls(&r, &[]);
    assert!(!f.generated_file().exists());
    // non-interactive: one active host per line
    assert!(r.lines().contains(&"api.anthropic.com".to_string()));
    assert!(!r.lines().contains(&"api.nuget.org".to_string()));
}

// --- maintenance ------------------------------------------------------------

#[test]
fn rebuild_builds_and_starts() {
    let f = Fixture::new();
    let r = f.run(&["rebuild"]);
    assert_exit(&r, 0);
    assert_calls(
        &r,
        &[
            docker_info(),
            f.collision_check(PROJECT_NAME),
            f.compose_interactive(&["build"]),
            up_wait(&f),
        ],
    );
    assert_output(&r, "Rebuilt and running.");
}

#[test]
fn update_pulls_and_busts_the_cache() {
    let f = Fixture::new();
    let r = f.run(&["update"]);
    assert_exit(&r, 0);
    assert_eq!(r.calls.len(), 4, "{r}");
    assert_eq!(r.calls[0].to_string(), docker_info().to_string());
    assert_eq!(r.calls[1], f.collision_check(PROJECT_NAME));
    let build = &r.calls[2].args;
    let prefix = f
        .compose_interactive(&["build", "--pull", "--build-arg"])
        .args;
    assert_eq!(&prefix[..], &build[..prefix.len()]);
    assert_eq!(build[prefix.len()..].len(), 1);
    assert!(
        Regex::new(r"^CACHE_BUST=\d{10,}$")
            .unwrap()
            .is_match(build.last().unwrap()),
        "{r}"
    );
    assert_eq!(r.calls[3].to_string(), up_wait(&f).to_string());
}

#[test]
fn reset_home_removes_containers_and_the_volume() {
    let f = Fixture::new();
    let r = f.run(&["reset-home", "--yes"]);
    assert_exit(&r, 0);
    assert_calls(
        &r,
        &[
            docker_info(),
            f.collision_check(PROJECT_NAME),
            f.compose(&["down"]),
            docker(&["volume", "rm", "demo-project_home"]),
        ],
    );
    assert_output(&r, "Home volume demo-project_home deleted.");
}

#[test]
fn info_does_not_call_docker() {
    let f = Fixture::new();
    let r = f.run(&["info"]);
    assert_exit(&r, 0);
    assert_calls(&r, &[]);
    assert!(!f.generated_file().exists());
    assert_output(&r, &f.work_dir());
    assert_output(&r, ID);
    assert!(
        Regex::new(r"dotnet\s*│\s*10\.0")
            .unwrap()
            .is_match(&r.stdout),
        "{r}"
    );
}

#[rstest]
#[case(&["help"])]
#[case(&["--help"])]
#[case(&["help", "up"])]
#[case(&["up", "--help"])]
#[case(&["help", "logs"])]
fn help_does_not_call_docker(#[case] args: &[&str]) {
    let f = Fixture::new();
    let r = f.run(args);
    assert_exit(&r, 0);
    assert_calls(&r, &[]);
    assert!(!f.generated_file().exists());
    assert_output(&r, "Usage:");
}

/// The shipped sandbox.toml and allowlist.txt (as currently in the repo) load without errors.
#[test]
fn repo_config_files_are_valid() {
    let f = Fixture::new();
    fs::copy(f.init_templates_dir().join("sandbox.toml"), f.conf_file()).unwrap();
    fs::copy(
        f.init_templates_dir().join("allowlist.txt"),
        f.allowlist_file(),
    )
    .unwrap();
    for args in [vec!["info"], vec!["allowlist", "-a"], vec!["status"]] {
        let r = f.run(&args);
        assert_exit(&r, 0);
        assert!(!r.output().contains("ERROR"), "{r}");
    }

    println!("{}", f.run(&["allowlist"]).lines().join("\n"));
    assert!(
        f.run(&["allowlist"])
            .lines()
            .iter()
            .all(|line| line.is_empty()),
    );
}

// --- options and configuration ---------------------------------------------------

#[test]
fn verbose_prints_the_docker_commands() {
    let f = Fixture::new();
    let r = f.run(&["status", "-v"]);
    assert_exit(&r, 0);
    assert_calls(&r, &[docker_info(), f.collision_check(PROJECT_NAME), ps(&f), port(&f)]);
    assert_output(&r, "$ docker info --format {{.ServerVersion}}");
    assert_output(
        &r,
        &format!(
            "$ docker compose -p demo-project --project-directory {} -f ",
            f.dir
        ),
    );
    // the line may be wrapped to the terminal width
    let collapsed = Regex::new(r"\s+")
        .unwrap()
        .replace_all(&r.output(), " ")
        .into_owned();
    assert!(collapsed.contains(" ps -a --format json"), "{r}");
}

#[test]
fn every_compose_call_uses_only_the_generated_file() {
    let f = Fixture::new();
    f.write_conf(&(f.read_conf() + "memory = \"4g\"\n"));
    let r = f.run(&["up", "--no-open"]);
    assert_exit(&r, 0);
    assert_calls(
        &r,
        &[
            docker_info(),
            f.collision_check(PROJECT_NAME),
            f.compose_interactive(&["build"]),
            up_wait(&f),
            port(&f),
        ],
    );
    for c in r.calls.iter().filter(|c| c.args[0] == "compose") {
        assert_eq!(c.args.iter().filter(|a| *a == "-f").count(), 1, "{c}");
    }
    assert!(
        fs::read_to_string(f.generated_file())
            .unwrap()
            .contains("mem_limit: \"4g\"")
    );
}

#[test]
fn project_name_sets_the_container_group() {
    let f = Fixture::new();
    f.write_conf(
        &f.read_conf()
            .replace("# name = \"myproject\"", "name = \"Other Name\""),
    );
    let r = f.run_with(
        &["up", "--no-open"],
        RunOpts::default().rules(vec![rule("ps -a --filter")]),
    );
    assert_exit(&r, 0);
    assert_eq!(
        r.calls[1].to_string(),
        f.collision_check("other-name").to_string()
    );
    for c in r.calls.iter().filter(|c| c.args[0] == "compose") {
        assert_eq!(c.args[2], "other-name");
    }

    let r = f.run(&["reset-home", "-y"]);
    assert_exit(&r, 0);
    assert_eq!(
        r.calls.last().unwrap().to_string(),
        docker(&["volume", "rm", "other-name_home"]).to_string()
    );
}

// --- finding the .sandbox folder ---------------------------------------------------

/// Without SANDBOX_DIR (as when run without the launchers) the app searches upwards from the current directory.
#[test]
fn finds_the_sandbox_folder_from_a_subfolder() {
    let f = Fixture::new();
    let sub = f.project_dir.join("src").join("deep");
    fs::create_dir_all(&sub).unwrap();
    let r = f.run_with(&["info"], RunOpts::default().cwd(sub).sandbox_dir(""));
    assert_exit(&r, 0);
    assert_output(&r, &f.work_dir());
    let r = f.run_with(
        &["allowlist"],
        RunOpts::default().cwd(f.dir.clone()).sandbox_dir(""),
    );
    assert!(r.lines().contains(&"api.anthropic.com".to_string()), "{r}");
}

#[test]
fn no_sandbox_folder_is_an_error() {
    let f = Fixture::new();
    let r = f.run_with(
        &["info"],
        RunOpts::default().cwd(f.root.clone()).sandbox_dir(""),
    );
    assert_exit(&r, 1);
    assert_calls(&r, &[]);
    assert_output(
        &r,
        "Could not find the .sandbox folder (with sandbox.toml).",
    );
}

#[test]
fn sandbox_dir_must_be_a_sandbox_folder() {
    let f = Fixture::new();
    let r = f.run_with(
        &["status"],
        RunOpts::default().sandbox_dir(f.project_dir.as_str()),
    );
    assert_exit(&r, 1);
    assert_calls(&r, &[]);
    assert_output(
        &r,
        &format!(
            "SANDBOX_DIR is {}, which is not a .sandbox folder",
            f.project_dir
        ),
    );
}
