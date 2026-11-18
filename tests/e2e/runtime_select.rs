//! Docker or Podman: which runtime runs (auto, `[host] runtime`, `SANDBOX_RUNTIME`), the Podman calls, the
//! Podman parts of the compose file and the NVIDIA check. PATH holds only the fake programs here, so a real Podman
//! or Docker on this machine doesn't matter.

use crate::support::*;
use std::fs;

/// What `status` does with Podman.
fn podman_status_calls(f: &Fixture) -> Vec<Call> {
    vec![
        podman_info(),
        f.podman_collision_check(),
        f.podman_compose(&["ps", "-a", "--format", "json"]),
        f.podman_compose(&["port", "gateway", "3001"]),
    ]
}

fn docker_status_calls(f: &Fixture) -> Vec<Call> {
    vec![
        docker_info(),
        f.collision_check(PROJECT_NAME),
        f.compose(&["ps", "-a", "--format", "json"]),
        f.compose(&["port", "gateway", "3001"]),
    ]
}

fn auto(f: &Fixture, podman: bool, docker: bool) -> RunOpts {
    RunOpts::default()
        .path(f.fake_path(podman, docker))
        .runtime("")
}

fn write_local(f: &Fixture, text: &str) {
    fs::write(f.dir.join("sandbox.local.toml"), text).unwrap();
}

#[test]
fn auto_prefers_podman_when_both_are_installed() {
    let f = Fixture::new();
    let r = f.run_with(&["status"], auto(&f, true, true));
    assert_exit(&r, 0);
    assert_calls(&r, &podman_status_calls(&f));
    assert_output(&r, "127.0.0.1:49153→3001");
    assert_output(&r, &format!("URL: {URL}"));
}

#[test]
fn auto_uses_docker_without_podman() {
    let f = Fixture::new();
    let r = f.run_with(&["status"], auto(&f, false, true));
    assert_exit(&r, 0);
    assert_calls(&r, &docker_status_calls(&f));
}

#[test]
fn auto_uses_podman_alone() {
    let f = Fixture::new();
    let r = f.run_with(&["status"], auto(&f, true, false));
    assert_exit(&r, 0);
    assert_calls(&r, &podman_status_calls(&f));
}

#[test]
fn the_setting_chooses_docker_although_podman_is_installed() {
    let f = Fixture::new();
    write_local(&f, "[host]\nruntime = \"docker\"\n");
    let r = f.run_with(&["status"], auto(&f, true, true));
    assert_exit(&r, 0);
    assert_calls(&r, &docker_status_calls(&f));
}

#[test]
fn the_environment_variable_beats_the_setting() {
    let f = Fixture::new();
    write_local(&f, "[host]\nruntime = \"docker\"\n");
    let r = f.run_with(
        &["status"],
        RunOpts::default()
            .path(f.fake_path(true, true))
            .runtime("Podman"),
    );
    assert_exit(&r, 0);
    assert_calls(&r, &podman_status_calls(&f));
}

#[test]
fn a_chosen_runtime_that_is_missing_names_the_setting() {
    let f = Fixture::new();
    write_local(&f, "[host]\nruntime = \"podman\"\n");
    let r = f.run_with(&["status"], auto(&f, false, true));
    assert_exit(&r, 1);
    assert_calls(&r, &[]);
    assert_output(
        &r,
        "Podman is not installed (runtime = \"podman\" in sandbox.local.toml).",
    );
}

#[test]
fn an_unknown_environment_value_is_an_error() {
    let f = Fixture::new();
    let r = f.run_with(
        &["status"],
        RunOpts::default()
            .path(f.fake_path(true, true))
            .runtime("lxc"),
    );
    assert_exit(&r, 1);
    assert_calls(&r, &[]);
    assert_output(
        &r,
        "SANDBOX_RUNTIME must be \"auto\", \"docker\" or \"podman\" (got \"lxc\").",
    );
}

#[test]
fn podman_not_working() {
    let f = Fixture::new();
    let r = f.run_with(
        &["status"],
        auto(&f, true, true).rules(vec![rule("info").stderr("no connection").exit(125)]),
    );
    assert_exit(&r, 1);
    assert_calls(&r, &[podman_info()]);
    assert_output(&r, "Podman is not");
}

#[test]
fn podman_up_builds_starts_and_prints_the_url() {
    let f = Fixture::new();
    let r = f.run_with(&["up", "--no-open"], auto(&f, true, false));
    assert_exit(&r, 0);
    assert_calls(
        &r,
        &[
            podman_info(),
            f.podman_collision_check(),
            f.podman_compose(&["build"]),
            f.podman_compose(&["up", "-d", "--wait", "--wait-timeout", "300"]),
            f.podman_compose(&["port", "gateway", "3001"]),
        ],
    );
    assert_output(&r, URL);
}

#[test]
fn podman_reset_home_removes_the_volume_with_podman() {
    let f = Fixture::new();
    let r = f.run_with(&["reset-home", "--yes"], auto(&f, true, false));
    assert_exit(&r, 0);
    assert_calls(
        &r,
        &[
            podman_info(),
            f.podman_collision_check(),
            f.podman_compose(&["down"]),
            podman(&["volume", "rm", &format!("{PROJECT_NAME}_home")]),
        ],
    );
}

#[test]
fn podman_status_reads_podman_compose_json() {
    let f = Fixture::new();
    let podman_ps = r#"[{"Names":["demo_desktop_1"],"State":"running","Status":"Up 2 minutes (healthy)","Labels":{"com.docker.compose.service":"desktop"},"Ports":null},{"State":"running","Status":"Up 2 minutes","Labels":{"com.docker.compose.service":"gateway"},"Ports":[{"host_ip":"127.0.0.1","container_port":3001,"host_port":49153,"range":1,"protocol":"tcp"}]}]"#;
    let r = f.run_with(
        &["status"],
        auto(&f, true, false).rules(vec![rule("ps -a --format json").stdout(podman_ps)]),
    );
    assert_exit(&r, 0);
    for s in ["desktop", "gateway", "healthy", "127.0.0.1:49153→3001"] {
        assert_output(&r, s);
    }
}

#[test]
fn rootless_podman_keeps_the_user_id_and_labels_the_mounts() {
    let f = Fixture::new();
    let r = f.run_with(&["status"], auto(&f, true, false));
    assert_exit(&r, 0);
    let text = fs::read_to_string(f.generated_file()).unwrap();
    assert!(text.contains("    userns_mode: \"keep-id\""), "{text}");
    assert!(text.contains("    user: \"0:0\""), "{text}");
    assert!(text.contains("- .:/sandbox:ro,z"), "{text}");
    assert!(text.contains(", bind: {selinux: z}}"), "{text}");
}

#[test]
fn rootful_podman_doesnt_map_the_user() {
    let f = Fixture::new();
    let r = f.run_with(
        &["status"],
        auto(&f, true, false).rules(vec![rule("{{.Version.Version}}").stdout("5.4.1 false\n")]),
    );
    assert_exit(&r, 0);
    let text = fs::read_to_string(f.generated_file()).unwrap();
    assert!(!text.contains("userns_mode"), "{text}");
    assert!(text.contains("- .:/sandbox:ro,z"), "{text}");
}

#[test]
fn docker_compose_file_has_no_podman_parts() {
    let f = Fixture::new();
    let r = f.run(&["status"]);
    assert_exit(&r, 0);
    let text = fs::read_to_string(f.generated_file()).unwrap();
    for podman_only in ["userns_mode", "selinux", ":ro,z", "nvidia.com/gpu"] {
        assert!(!text.contains(podman_only), "{podman_only}: {text}");
    }
}

const LLM_CONF: &str = "\n[llm]\nmodel = \"qwen3.6:27b\"\n";

fn with_llm(f: &Fixture) {
    f.write_conf(&format!("{}{LLM_CONF}", f.read_conf()));
}

#[test]
fn podman_uses_the_nvidia_gpu_as_a_cdi_device() {
    let f = Fixture::new();
    with_llm(&f);
    let r = f.run_with(&["status"], auto(&f, true, false));
    assert_exit(&r, 0);
    let text = fs::read_to_string(f.generated_file()).unwrap();
    assert!(
        text.contains("    devices:\n      - \"nvidia.com/gpu=all\""),
        "{text}"
    );
    assert!(!text.contains("driver: nvidia"), "{text}");
}

#[cfg(target_os = "linux")]
#[test]
fn podman_up_stops_before_building_without_the_nvidia_cdi_device() {
    let f = Fixture::new();
    with_llm(&f);
    let r = f.run_with(
        &["up", "--no-open"],
        auto(&f, true, false).rules(vec![rule("cdi list").stdout("INFO Found 0 CDI devices\n")]),
    );
    assert_exit(&r, 1);
    assert_calls(
        &r,
        &[
            podman_info(),
            f.podman_collision_check(),
            call("nvidia-ctk", &["cdi", "list"]),
        ],
    );
    assert_output(&r, "The NVIDIA GPU is not available to Podman");
    assert_output(&r, "nvidia-ctk cdi generate --output=/etc/cdi/nvidia.yaml");
    assert_output(&r, "gpu = \"none\" under [llm] in sandbox.local.toml");
}

#[cfg(target_os = "linux")]
#[test]
fn podman_up_checks_the_nvidia_cdi_device_then_builds() {
    let f = Fixture::new();
    with_llm(&f);
    let r = f.run_with(
        &["up", "--no-open"],
        auto(&f, true, false).rules(vec![
            rule("ollama list").stdout("NAME  ID  SIZE  MODIFIED\nqwen3.6:27b  a  17 GB  now\n"),
        ]),
    );
    assert_exit(&r, 0);
    assert_eq!(r.calls[2], call("nvidia-ctk", &["cdi", "list"]), "{r}");
    assert_eq!(r.calls[3], f.podman_compose(&["build"]), "{r}");
    assert_eq!(
        r.calls[4],
        podman(&["volume", "inspect", "sandbox-llm-models"]),
        "{r}"
    );
}

#[test]
fn podman_on_the_cpu_skips_the_nvidia_check() {
    let f = Fixture::new();
    with_llm(&f);
    write_local(&f, "[llm]\ngpu = \"none\"\n");
    let r = f.run_with(
        &["up", "--no-open"],
        auto(&f, true, false).rules(vec![
            rule("ollama list").stdout("NAME  ID  SIZE  MODIFIED\nqwen3.6:27b  a  17 GB  now\n"),
        ]),
    );
    assert_exit(&r, 0);
    assert!(!r.calls.iter().any(|c| c.name == "nvidia-ctk"), "{r}");
}

#[test]
fn info_shows_the_runtime_and_where_it_is_chosen() {
    let f = Fixture::new();
    let r = f.run_with(&["info"], auto(&f, true, true));
    assert_exit(&r, 0);
    assert_calls(&r, &[]);
    assert_output(&r, "Podman (auto)");

    write_local(&f, "[host]\nruntime = \"docker\"\n");
    let r = f.run_with(&["info"], auto(&f, true, true));
    assert_output(&r, "Docker (runtime = \"docker\" in sandbox.local.toml)");

    let r = f.run_with(&["info"], auto(&f, false, false));
    assert_exit(&r, 0);
    assert_output(
        &r,
        "Docker (runtime = \"docker\" in sandbox.local.toml): not installed",
    );
}
