//! Nested sandboxes: `[tools] podman` gives the desktop what nested containers need (rootless Podman only), and a
//! sandbox started inside a desktop (`SANDBOX_INSIDE=1`) chains its proxy to the outer one.

use crate::support::*;
use std::fs;

const LLM_CONF: &str = "\n[llm]\nmodel = \"qwen3.6:27b\"\n";

fn add_conf(f: &Fixture, extra: &str) {
    f.write_conf(&format!("{}{extra}", f.read_conf()));
}

/// `podman = true` under the fixture's `[tools]`.
fn with_podman_tool(f: &Fixture) {
    let conf = f.read_conf();
    assert!(conf.contains("[tools]\n"), "{conf}");
    f.write_conf(&conf.replacen("[tools]\n", "[tools]\npodman = true\n", 1));
}

/// Rootless Podman only on PATH.
fn podman(f: &Fixture) -> RunOpts {
    RunOpts::default()
        .path(f.fake_path(true, false))
        .runtime("")
}

/// Inside a sandbox desktop whose proxy is 127.0.0.1:3128.
fn inside(f: &Fixture) -> RunOpts {
    podman(f)
        .env("SANDBOX_INSIDE", "1")
        .env("HTTPS_PROXY", "http://127.0.0.1:3128")
}

/// What every Podman command runs before the compose file is written.
fn runtime_check(f: &Fixture) -> Vec<Call> {
    vec![
        podman_info(),
        podman_compose_version(),
        f.podman_collision_check(),
    ]
}

fn compose_file(f: &Fixture) -> String {
    fs::read_to_string(f.generated_file()).unwrap()
}

fn assert_has(text: &str, part: &str) {
    assert!(text.contains(part), "missing {part:?} in:\n{text}");
}

fn assert_lacks(text: &str, part: &str) {
    assert!(!text.contains(part), "unexpected {part:?} in:\n{text}");
}

#[test]
fn the_podman_tool_gives_the_desktop_what_nested_containers_need() {
    let f = Fixture::new();
    with_podman_tool(&f);
    let r = f.run_with(&["status"], podman(&f));
    assert_exit(&r, 0);
    let text = compose_file(&f);
    assert_has(&text, "        INSTALL_PODMAN: \"true\"\n");
    assert_has(
        &text,
        "    devices:\n      - \"/dev/fuse\"\n      - \"/dev/net/tun\"\n",
    );
    assert_has(
        &text,
        "    security_opt:\n      - \"label=disable\"\n      - \"unmask=ALL\"\n",
    );
    assert_has(&text, "    pids_limit: 32768\n");
    assert_has(&text, "      - containers:/config/.local/share/containers");
    assert_has(&text, "\nvolumes:\n  home:\n  containers:\n");
    assert_has(&text, "      SANDBOX_INSIDE: \"1\"");
    // not nested itself
    assert_has(&text, "    hostname: sandbox\n");
    assert_has(&text, "tinyproxy.conf\"]");
}

#[test]
fn the_nesting_devices_and_options_join_the_configured_ones() {
    let f = Fixture::new();
    let conf = f
        .read_conf()
        .replace("[desktop]\n", "[desktop]\ndevices = [\"/dev/dri:/dev/dri\", \"/dev/fuse\"]\nsecurity_opt = [\"seccomp=unconfined\"]\n");
    f.write_conf(&conf);
    with_podman_tool(&f);
    let r = f.run_with(&["status"], podman(&f));
    assert_exit(&r, 0);
    let text = compose_file(&f);
    assert_has(
        &text,
        "    devices:\n      - \"/dev/dri:/dev/dri\"\n      - \"/dev/fuse\"\n      - \"/dev/net/tun\"\n",
    );
    assert_has(
        &text,
        "    security_opt:\n      - \"seccomp=unconfined\"\n      - \"label=disable\"\n      - \"unmask=ALL\"\n",
    );
}

#[test]
fn without_the_podman_tool_there_are_no_nesting_parts() {
    let f = Fixture::new();
    let r = f.run_with(&["status"], podman(&f));
    assert_exit(&r, 0);
    let text = compose_file(&f);
    for part in [
        "/dev/fuse",
        "unmask=ALL",
        "SANDBOX_INSIDE",
        "containers:",
        "32768",
    ] {
        assert_lacks(&text, part);
    }
    assert_has(&text, "    pids_limit: 4096\n");
}

#[test]
fn the_podman_tool_needs_podman_not_docker() {
    let f = Fixture::new();
    with_podman_tool(&f);
    let r = f.run(&["status"]);
    assert_exit(&r, 1);
    assert_output(
        &r,
        "podman = true under [tools] needs rootless Podman, not Docker. Set runtime = \"podman\" under [host] in sandbox.local.toml, or podman = false under [tools] in sandbox.toml.",
    );
    assert_calls(&r, &[docker_info(), f.collision_check(PROJECT_NAME)]);
}

#[test]
fn the_podman_tool_needs_rootless_podman() {
    let f = Fixture::new();
    with_podman_tool(&f);
    let r = f.run_with(
        &["status"],
        podman(&f).rules(vec![rule("{{.Version.Version}}").stdout("5.4.1 false\n")]),
    );
    assert_exit(&r, 1);
    assert_output(&r, "needs rootless Podman, but this Podman runs as root.");
    // stops after the runtime check, before any compose call
    assert_calls(&r, &runtime_check(&f));
}

#[test]
fn a_nested_sandbox_chains_its_proxy_to_the_outer_one() {
    let f = Fixture::new();
    let r = f.run_with(&["status"], inside(&f));
    assert_exit(&r, 0);
    let text = compose_file(&f);
    assert_has(
        &text,
        "    command: [\"tinyproxy\", \"-d\", \"-c\", \"/etc/tinyproxy/tinyproxy-nested.conf\"]",
    );
    assert_has(&text, "      - \"sandbox-upstream:127.0.0.1\"\n");
    // builds get the outer proxy in the lowercase names too (apt reads only those)
    assert_has(&text, "  http_proxy: \"http://127.0.0.1:3128\"\n");
    assert_has(&text, "  https_proxy: \"http://127.0.0.1:3128\"\n");
    assert_has(&text, "    build: {context: ./.state/proxy, args: *build-proxy}\n");
    assert_has(&text, "      args:\n        <<: *build-proxy\n");
    assert_lacks(&text, "host-gateway");
    assert_lacks(&text, "hostname:");
    // not a nesting desktop itself without the tool
    assert_lacks(&text, "SANDBOX_INSIDE");
}

#[test]
fn a_nested_sandbox_with_the_podman_tool_can_nest_again() {
    let f = Fixture::new();
    with_podman_tool(&f);
    let r = f.run_with(&["status"], inside(&f));
    assert_exit(&r, 0);
    let text = compose_file(&f);
    assert_has(&text, "tinyproxy-nested.conf");
    assert_has(&text, "      - \"/dev/fuse\"\n");
    assert_has(&text, "      SANDBOX_INSIDE: \"1\"");
}

#[test]
fn a_nested_sandbox_ignores_the_local_llm() {
    let f = Fixture::new();
    add_conf(&f, LLM_CONF);
    let r = f.run_with(&["status"], inside(&f));
    assert_exit(&r, 0);
    let text = compose_file(&f);
    assert_lacks(&text, "ollama");
    assert_lacks(&text, "SANDBOX_LLM_URL");

    let r = f.run_with(&["info"], inside(&f));
    assert_exit(&r, 0);
    assert_output(&r, "off (nested sandbox)");

    let r = f.run_with(&["llm", "status"], inside(&f));
    assert_exit(&r, 1);
    assert_output(&r, "The local LLM is off in a nested sandbox");
}

#[test]
fn a_nested_sandbox_needs_the_outer_proxy() {
    let f = Fixture::new();
    let r = f.run_with(&["status"], podman(&f).env("SANDBOX_INSIDE", "1"));
    assert_exit(&r, 1);
    assert_output(
        &r,
        "HTTPS_PROXY is not set, so the outer sandbox's proxy is unknown",
    );
}

#[test]
fn an_unresolvable_outer_proxy_is_an_error() {
    let f = Fixture::new();
    let r = f.run_with(
        &["status"],
        podman(&f)
            .env("SANDBOX_INSIDE", "1")
            .env("HTTPS_PROXY", "http://no-such-proxy.invalid:3128"),
    );
    assert_exit(&r, 1);
    assert_output(
        &r,
        "The outer proxy no-such-proxy.invalid (HTTPS_PROXY) can't be resolved",
    );
}

#[cfg(unix)]
#[test]
fn a_read_only_sandbox_folder_says_to_use_a_writable_copy() {
    use std::os::unix::fs::PermissionsExt;
    if rustix::process::geteuid().is_root() {
        return; // root writes anyway
    }
    let f = Fixture::new();
    f.run_with(&["status"], inside(&f)); // creates .state
    let state = f.state_dir();
    fs::set_permissions(&state, fs::Permissions::from_mode(0o555)).unwrap();
    let r = f.run_with(&["status"], inside(&f));
    fs::set_permissions(&state, fs::Permissions::from_mode(0o755)).unwrap();
    assert_exit(&r, 1);
    assert_output(
        &r,
        "is read-only here. Copy the project to a writable folder (e.g. under /config) and run sandbox there.",
    );
    assert_calls(&r, &runtime_check(&f));
}
