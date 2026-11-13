//! Docker missing or failing, invalid configuration: the right message, exit code 1, and no further docker calls.

use crate::support::*;
use rstest::rstest;
use std::fs;

fn up_wait(f: &Fixture) -> Call {
    f.compose(&["up", "-d", "--wait", "--wait-timeout", "300"])
}

#[test]
fn docker_not_installed() {
    let f = Fixture::new();
    let r = f.run_with(&["status"], RunOpts::default().path(f.empty_bin.as_str()));
    assert_exit(&r, 1);
    assert_calls(&r, &[]);
    assert_output(&r, "Docker is not installed.");
}

#[rstest]
#[case(&["up"])]
#[case(&["status"])]
#[case(&["shell"])]
#[case(&["claude"])]
#[case(&["allow", "example.com"])]
fn docker_not_running(#[case] args: &[&str]) {
    let f = Fixture::new();
    let allowlist = fs::read_to_string(f.allowlist_file()).unwrap();
    let r = f.run_with(
        args,
        RunOpts::default().rules(vec![
            rule("info")
                .stderr("Cannot connect to the Docker daemon")
                .exit(1),
        ]),
    );
    assert_exit(&r, 1);
    assert_calls(&r, &[docker_info()]);
    assert_output(&r, "Docker is not running.");
    assert_eq!(allowlist, fs::read_to_string(f.allowlist_file()).unwrap());
}

#[test]
fn up_refuses_a_container_group_of_another_project() {
    let f = Fixture::new();
    let r = f.run_with(
        &["up", "--no-open"],
        RunOpts::default().rules(vec![rule("ps -a --filter").stdout("ffffffff\n")]),
    );
    assert_exit(&r, 1);
    assert_calls(&r, &[docker_info(), f.collision_check(PROJECT_NAME)]);
    assert_output(
        &r,
        "Another project already uses the container group 'demo-project'.",
    );
}

#[test]
fn up_stops_when_the_build_fails() {
    let f = Fixture::new();
    let r = f.run_with(
        &["up", "--no-open"],
        RunOpts::default().rules(vec![rule(" build").stderr("failed to solve").exit(1)]),
    );
    assert_exit(&r, 1);
    assert_calls(
        &r,
        &[
            docker_info(),
            f.collision_check(PROJECT_NAME),
            f.compose_interactive(&["build"]),
        ],
    );
    assert_output(&r, "Building the sandbox images failed");
}

#[test]
fn up_tolerates_an_unhealthy_but_running_desktop() {
    let f = Fixture::new();
    let r = f.run_with(
        &["up", "--no-open"],
        RunOpts::default().rules(vec![
            rule("up -d --wait")
                .stderr("container desktop is unhealthy")
                .exit(1),
        ]),
    );
    assert_exit(&r, 0);
    assert_calls(
        &r,
        &[
            docker_info(),
            f.collision_check(PROJECT_NAME),
            f.compose_interactive(&["build"]),
            up_wait(&f),
            f.compose(&["ps", "-a", "--format", "json"]),
            f.compose(&["port", "gateway", "3001"]),
        ],
    );
    assert_output(&r, "The desktop isn't reporting ready yet");
    assert_output(&r, URL);
}

#[test]
fn up_fails_when_the_desktop_is_not_running() {
    let f = Fixture::new();
    let ps = "{\"Service\":\"desktop\",\"State\":\"exited\",\"Health\":\"\",\"Status\":\"Exited (1)\"}\n";
    let r = f.run_with(
        &["up", "--no-open"],
        RunOpts::default().rules(vec![
            rule("up -d --wait").stderr("boom").exit(1),
            rule("ps -a --format json").stdout(ps),
        ]),
    );
    assert_exit(&r, 1);
    assert_calls(
        &r,
        &[
            docker_info(),
            f.collision_check(PROJECT_NAME),
            f.compose_interactive(&["build"]),
            up_wait(&f),
            f.compose(&["ps", "-a", "--format", "json"]),
        ],
    );
    assert_output(&r, "docker compose up (exit 1)");
    assert_output(&r, "boom");
    assert_output(&r, "Starting the sandbox failed");
}

#[test]
fn rebuild_does_not_tolerate_an_unhealthy_desktop() {
    let f = Fixture::new();
    let r = f.run_with(
        &["rebuild"],
        RunOpts::default().rules(vec![rule("up -d --wait").stderr("unhealthy").exit(1)]),
    );
    assert_exit(&r, 1);
    assert_calls(
        &r,
        &[
            docker_info(),
            f.compose_interactive(&["build"]),
            up_wait(&f),
        ],
    );
    assert_output(&r, "Starting the sandbox failed");
}

#[test]
fn up_fails_without_a_ui_port() {
    let f = Fixture::new();
    let r = f.run_with(
        &["up", "--no-open"],
        RunOpts::default().rules(vec![rule("port gateway 3001").exit(1)]),
    );
    assert_exit(&r, 1);
    assert_output(&r, "its UI port could not be determined");
}

#[rstest]
#[case("stop", "Stopping")]
#[case("down", "Removing containers")]
#[case("restart", "Restarting")]
fn compose_failure_shows_the_output(#[case] cmd: &str, #[case] what: &str) {
    let f = Fixture::new();
    let r = f.run_with(
        &[cmd],
        RunOpts::default().rules(vec![
            rule(&format!("never {cmd}"))
                .stderr("no such service")
                .exit(3),
        ]),
    );
    assert_exit(&r, 1);
    assert_calls(&r, &[docker_info(), f.compose(&[cmd])]);
    assert_output(&r, &format!("docker compose {cmd} (exit 3)"));
    assert_output(&r, "no such service");
    assert_output(&r, &format!("{what} failed."));
}

#[rstest]
#[case("1")] // exit code
#[case("0")] // port 0 = not published
fn open_when_not_running(#[case] mode: &str) {
    let f = Fixture::new();
    let rule = if mode == "1" {
        rule("port gateway 3001").exit(1)
    } else {
        rule("port gateway 3001").stdout(":0\n")
    };
    let r = f.run_with(&["open"], RunOpts::default().rules(vec![rule]));
    assert_exit(&r, 1);
    assert_calls(
        &r,
        &[docker_info(), f.compose(&["port", "gateway", "3001"])],
    );
    assert_output(&r, "The sandbox is not running");
}

#[test]
fn reset_home_needs_confirmation_when_not_interactive() {
    let f = Fixture::new();
    let r = f.run(&["reset-home"]);
    assert_exit(&r, 1);
    assert_calls(&r, &[docker_info()]);
    assert_output(
        &r,
        "Refusing to delete the home volume without confirmation (use --yes).",
    );
}

#[test]
fn reset_home_reports_a_failed_volume_removal() {
    let f = Fixture::new();
    let r = f.run_with(
        &["reset-home", "--yes"],
        RunOpts::default().rules(vec![rule("volume rm").stderr("volume is in use").exit(1)]),
    );
    assert_exit(&r, 1);
    assert_calls(
        &r,
        &[
            docker_info(),
            f.compose(&["down"]),
            docker(&["volume", "rm", "demo-project_home"]),
        ],
    );
    assert_output(&r, "Could not remove volume demo-project_home.");
}

#[test]
fn reload_failure() {
    let f = Fixture::new();
    let r = f.run_with(
        &["reload"],
        RunOpts::default().rules(vec![
            rule("restart proxy").exit(1).stderr("proxy not found"),
        ]),
    );
    assert_exit(&r, 1);
    assert_calls(&r, &[docker_info(), f.compose(&["restart", "proxy"])]);
    assert_output(&r, "Could not restart the proxy.");
}

#[test]
fn allow_rejects_an_invalid_host() {
    let f = Fixture::new();
    let before = fs::read_to_string(f.allowlist_file()).unwrap();
    let r = f.run(&["allow", "ok.example.com", "bad host!"]);
    assert_exit(&r, 1);
    assert_calls(&r, &[docker_info()]);
    assert_output(&r, "Not a valid host name: 'bad host!'.");
    assert_eq!(before, fs::read_to_string(f.allowlist_file()).unwrap());
}

#[rstest]
#[case(
    "[project]\nworkspace = \"rx\"\n",
    "sandbox.toml: project.workspace must be \"rw\" or \"ro\" (got \"rx\")."
)]
#[case(
    "[project]\nprotect_git = \"maybe\"\n",
    "sandbox.toml: project.protect_git must be true or false (got \"maybe\")."
)]
#[case(
    "[project]\nui_port = \"abc\"\n",
    "sandbox.toml: project.ui_port must be a whole number (got \"abc\")."
)]
#[case(
    "[project]\nui_port = 70000\n",
    "sandbox.toml: project.ui_port must be a port number from 1 to 65535 (got 70000)."
)]
#[case(
    "[project]\nworkspace_mode = \"ro\"\n",
    "sandbox.toml: unknown key 'project.workspace_mode' (known: name, workspace, protect_git, ui_port)."
)]
#[case(
    "[tool]\nnode = 24\n",
    "sandbox.toml: unknown setting 'tool' (known: project, tools, desktop, env, share)."
)]
#[case("project = 1\n", "sandbox.toml: project must be a table (got 1).")]
#[case(
    "[tools]\nnodejs = 24\n",
    "sandbox.toml: unknown tool tools.nodejs (known: apt, claude, claude_code, dotnet, node, bun, vscode, jetbrains)."
)]
#[case(
    "[tools]\ndotnet = 10.0\n",
    "sandbox.toml: tools.dotnet = 10.0: write versions as text, e.g. \"10.0\""
)]
#[case(
    "[tools]\napt = [true]\n",
    "sandbox.toml: tools.apt must be a list of names or versions."
)]
#[case(
    "[desktop]\nmemory = \"lots\"\n",
    "sandbox.toml: desktop.memory must be a size like \"8g\" or \"512m\" (got \"lots\")."
)]
#[case(
    "[desktop]\ncpus = 0\n",
    "sandbox.toml: desktop.cpus must be greater than 0 (got 0.0)."
)]
#[case(
    "[desktop]\ndevices = \"/dev/dri\"\n",
    "sandbox.toml: desktop.devices must be a list of texts, e.g. [\"a\", \"b\"] (got \"/dev/dri\")."
)]
#[case(
    "[desktop]\nauth = { user = \"me\" }\n",
    "sandbox.toml: desktop.auth needs both user and password."
)]
#[case(
    "[desktop.auth]\nuser = \"me\"\npassword = \"x\"\nemail = \"m\"\n",
    "sandbox.toml: unknown key 'desktop.auth.email' (known: user, password)."
)]
#[case(
    "[env]\nHTTP_PROXY = \"http://evil\"\n",
    "sandbox.toml: env.HTTP_PROXY is set by the sandbox and can't be changed."
)]
#[case(
    "[env]\nno_proxy = \"*\"\n",
    "sandbox.toml: env.no_proxy is set by the sandbox and can't be changed."
)]
#[case(
    "[env]\nTZ = \"UTC\"\n",
    "sandbox.toml: env.TZ is set by the sandbox; use desktop.timezone instead."
)]
#[case(
    "[env]\n\"MY-VAR\" = \"x\"\n",
    "sandbox.toml: env.MY-VAR is not a valid environment variable name."
)]
#[case(
    "[env]\nX = [1]\n",
    "sandbox.toml: env.X must be a text, number or true/false (got a list)."
)]
#[case(
    "[[share]]\npath = \"/x\"\n",
    "sandbox.toml: share #1 needs host = \"<folder on this machine>\"."
)]
#[case(
    "[[share]]\nhost = \"../missing\"\n",
    "sandbox.toml: share #1: ../missing does not exist"
)]
#[case(
    "[[share]]\nhost = \".git\"\npath = \"rel\"\n",
    "sandbox.toml: share #1: path must be absolute (got \"rel\")."
)]
#[case(
    "[[share]]\nhost = \".git\"\npath = \"/work/x\"\n",
    "sandbox.toml: share #1: path /work/x is reserved"
)]
#[case(
    "[[share]]\nhost = \".git\"\npath = \"/config\"\n",
    "sandbox.toml: share #1: path /config is reserved"
)]
#[case(
    "[[share]]\nhost = \".git\"\n[[share]]\nhost = \".git/hooks\"\npath = \"/shares/.git\"\n",
    "sandbox.toml: share #2: path /shares/.git is used twice."
)]
#[case(
    "[[share]]\nhost = \".git\"\nro = true\n",
    "sandbox.toml: unknown key 'share.ro' (known: host, path, readonly)."
)]
#[case(
    "share = { host = \".git\" }\n",
    "sandbox.toml: share must be written as [[share]] blocks (got a table)."
)]
#[case(
    "[project]\nworkspace = \n",
    "sandbox.toml is not valid TOML (line 2, column 13):"
)]
fn invalid_sandbox_toml(#[case] toml: &str, #[case] message: &str) {
    let f = Fixture::new();
    f.write_conf(toml);
    for cmd in ["status", "info"] {
        let r = f.run(&[cmd]);
        assert_exit(&r, 1);
        assert_calls(&r, &[]);
        assert_output(&r, message);
    }
    assert!(!f.generated_file().exists());
}

#[test]
fn old_sandbox_yml_is_reported_with_the_key_mapping() {
    let f = Fixture::new();
    fs::remove_file(f.conf_file()).unwrap();
    fs::write(f.old_conf_file(), "x-sandbox:\n  workspace_mode: rw\n").unwrap();
    let r = f.run(&["up", "--no-open"]);
    assert_exit(&r, 1);
    assert_calls(&r, &[]);
    assert_output(
        &r,
        "sandbox.yml is no longer read. Move its settings to sandbox.toml",
    );
    assert_output(&r, "→ [tools] <tool>");
}

#[test]
fn leftover_yml_files_are_ignored_with_a_warning() {
    let f = Fixture::new();
    fs::write(f.old_conf_file(), "x-sandbox:\n  workspace_mode: ro\n").unwrap();
    fs::write(
        f.old_local_conf_file(),
        "x-sandbox:\n  workspace_mode: ro\n",
    )
    .unwrap();
    let r = f.run(&["status"]);
    assert_exit(&r, 0);
    assert_output(&r, "sandbox.yml is ignored");
    assert_output(&r, "sandbox.local.yml is no longer read");
    assert!(
        fs::read_to_string(f.generated_file())
            .unwrap()
            .contains("read_only: false}")
    );
}

#[test]
fn missing_sandbox_toml() {
    let f = Fixture::new();
    fs::remove_file(f.conf_file()).unwrap();
    let r = f.run(&["status"]);
    assert_exit(&r, 1);
    assert_calls(&r, &[]);
    assert_output(&r, "which is not a .sandbox folder (with sandbox.toml).");

    // Without the launcher's SANDBOX_DIR, the search upwards finds nothing either.
    let r = f.run_with(
        &["status"],
        RunOpts::default()
            .cwd(f.project_dir.clone())
            .sandbox_dir(""),
    );
    assert_exit(&r, 1);
    assert_calls(&r, &[]);
    assert_output(
        &r,
        "Could not find the .sandbox folder (with sandbox.toml).",
    );
}
