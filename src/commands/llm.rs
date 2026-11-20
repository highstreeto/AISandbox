//! The local LLM (`[llm]`): preparing and pulling models for `up`, `sandbox llm status|pull|rm|usage`.
//!
//! Models live in the machine-wide runtime volume [`MODELS_VOLUME`] (shared by all projects, never removed by `down`
//! or `reset-home`). Ollama's downloads are started from here (`compose exec ollama ollama pull`); the desktop can't
//! start them, because the gateway in front of Ollama refuses those endpoints.

use crate::cli::LlmCommand;
use crate::config::{self, LlmSettings};
use crate::process::ProcResult;
use crate::runtime::Runtime;
use crate::ui::Ui;
use anyhow::{Result, bail};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

/// The external volume that holds the models of every project on this machine.
pub const MODELS_VOLUME: &str = "sandbox-llm-models";
/// Where the gateway writes one JSON line per request (in the `llm` container).
const USAGE_LOG: &str = "/var/log/llm/usage.jsonl";
/// The address of the gateway inside the sandbox.
pub const ENDPOINT: &str = "http://llm:11434";

pub fn run(ui: &Ui, rt: &dyn Runtime, command: LlmCommand) -> Result<i32> {
    match command {
        LlmCommand::Status => status(ui, rt),
        LlmCommand::Pull { model } => pull(ui, rt, model.as_deref()),
        LlmCommand::Rm { model, yes } => rm(ui, rt, &model, yes),
        LlmCommand::Usage { days, json, clear } => usage(ui, rt, days, json, clear),
    }
}

/// The project's `[llm]`, or an error that says how to turn it on.
fn need_llm(rt: &dyn Runtime) -> Result<&LlmSettings> {
    match &rt.project().settings.llm {
        Some(llm) => Ok(llm),
        None if rt.project().settings.llm_off_nested => {
            bail!("The local LLM is off in a nested sandbox (one started inside a sandbox).")
        }
        None if rt.project().settings.llm_off_locally => bail!(
            "The local LLM is switched off on this machine (enabled = false under [llm] in {}).",
            config::LOCAL_FILE_NAME
        ),
        None => bail!(
            "The local LLM is off for this project. Turn it on with model = \"{}\" under [llm] in sandbox.toml.",
            config::DEFAULT_LLM_MODEL
        ),
    }
}

/// `ollama` must be running for anything that talks to it.
fn need_running(ui: &Ui, rt: &dyn Runtime) -> Result<()> {
    let running = rt
        .compose_ps(ui)?
        .iter()
        .any(|s| s.service == "ollama" && s.state == "running");
    if !running {
        bail!(
            "The local LLM is not running (start it with: {} up).",
            ui.self_name()
        );
    }
    Ok(())
}

/// `compose exec -T <service> <command…>`, captured.
fn exec(ui: &Ui, rt: &dyn Runtime, service: &str, command: &[&str]) -> ProcResult {
    rt.compose_exec(ui, service, command)
}

/// The models Ollama has, as the names `ollama list` prints (`qwen3.6:27b`).
fn installed(ui: &Ui, rt: &dyn Runtime) -> Result<Vec<String>> {
    let r = exec(ui, rt, "ollama", &["ollama", "list"]);
    if r.exit_code != 0 {
        ui.show_output("ollama list", &r);
        bail!("Could not ask the local LLM which models it has.");
    }
    Ok(parse_list(&r.stdout))
}

/// The first column of `ollama list` without its header line.
fn parse_list(text: &str) -> Vec<String> {
    text.lines()
        .skip(1)
        .filter_map(|l| l.split_whitespace().next())
        .map(str::to_string)
        .collect()
}

/// `name` is `name:latest` for Ollama.
fn full_tag(model: &str) -> String {
    let last = model.rsplit('/').next().unwrap_or(model);
    if last.contains(':') {
        model.to_string()
    } else {
        format!("{model}:latest")
    }
}

/// Downloads one model with Ollama's own progress display.
fn download(ui: &Ui, rt: &dyn Runtime, model: &str) -> Result<()> {
    let tty = ui.interactive;
    if rt.compose_exec_attached(ui, "ollama", &["ollama", "pull", model], tty)? != 0 {
        bail!(
            "Downloading {model} failed (see the output above). Try again with: {} llm pull {model}",
            ui.self_name()
        );
    }
    Ok(())
}

/// What a download costs, for the note before it starts.
fn size_note(model: &str) -> &'static str {
    match model {
        "qwen3.6:27b" => " (about 17 GB)",
        "qwen3.6:35b-a3b" => " (about 22 GB)",
        "gpt-oss:20b" => " (about 14 GB)",
        "devstral-small-2" => " (about 15 GB)",
        "qwen3-coder:30b" => " (about 19 GB)",
        _ => "",
    }
}

/// `up`: pulls the project's models that the volume doesn't have yet.
pub fn pull_missing(ui: &Ui, rt: &dyn Runtime) -> Result<()> {
    let Some(llm) = &rt.project().settings.llm else {
        return Ok(());
    };
    let have: Vec<String> = installed(ui, rt)?.iter().map(|m| full_tag(m)).collect();
    let missing: Vec<String> = llm
        .all_models()
        .into_iter()
        .filter(|m| !have.contains(&full_tag(m)))
        .collect();
    if missing.is_empty() {
        return Ok(());
    }
    ui.info(&format!(
        "Downloading {} into the shared volume {MODELS_VOLUME}; this happens once per machine and can take a while.",
        if missing.len() == 1 {
            "one model".to_string()
        } else {
            format!("{} models", missing.len())
        }
    ));
    for m in &missing {
        ui.rule(&format!("Pulling {m}{}", size_note(m)));
        download(ui, rt, m)?;
        ui.plain_rule();
    }
    Ok(())
}

/// What to do when `up` failed and the log looks like a missing GPU driver for the container.
pub fn gpu_hint(rt: &dyn Runtime, up: &ProcResult) -> Option<String> {
    let llm = rt.project().settings.llm.as_ref()?;
    let text = format!("{}\n{}", up.stdout, up.stderr).to_lowercase();
    let mentions_gpu = [
        "could not select device driver",
        "nvidia",
        "/dev/kfd",
        "/dev/dri",
    ]
    .iter()
    .any(|p| text.contains(p));
    if !mentions_gpu {
        return None;
    }
    Some(format!(
        "The local LLM could not get the {} GPU. {} To run on the CPU instead (slow), set gpu = \"none\" under [llm] in {}.",
        llm.gpu.name(),
        rt.gpu_hint(),
        config::LOCAL_FILE_NAME
    ))
}

fn status(ui: &Ui, rt: &dyn Runtime) -> Result<i32> {
    let llm = need_llm(rt)?;
    let source = |s: config::SettingSource| match s.file_name() {
        Some(f) => format!(" ({f})"),
        None => " (built-in default)".to_string(),
    };
    let mut lines = vec![
        format!(
            "{}   {}{}",
            ui.bold("Model"),
            llm.model,
            source(llm.model_source)
        ),
        format!(
            "{}     {}{}",
            ui.bold("GPU"),
            llm.gpu.name(),
            source_gpu(llm)
        ),
        format!("{} {} tokens", ui.bold("Context"), llm.context),
        format!("{} {ENDPOINT} (inside the sandbox)", ui.bold("Endpoint")),
    ];
    if !llm.extra_models.is_empty() {
        lines.insert(
            1,
            format!("{}  {}", ui.bold("Also"), llm.extra_models.join(", ")),
        );
    }
    ui.print_panel("Local LLM", &lines, None, false);
    need_running(ui, rt)?;
    for (title, command) in [("Models", "list"), ("Loaded now", "ps")] {
        let r = exec(ui, rt, "ollama", &["ollama", command]);
        if r.exit_code != 0 {
            ui.show_output(&format!("ollama {command}"), &r);
            bail!("Could not ask the local LLM (ollama {command}).");
        }
        println!("{}\n{}", ui.bold(title), r.stdout.trim_end());
    }
    Ok(0)
}

fn source_gpu(llm: &LlmSettings) -> String {
    llm.gpu_source
        .file_name()
        .map_or(String::new(), |f| format!(" ({f})"))
}

fn pull(ui: &Ui, rt: &dyn Runtime, model: Option<&str>) -> Result<i32> {
    let llm = need_llm(rt)?;
    let models = match model {
        Some(m) => vec![config::parse_model_tag(m)?],
        None => llm.all_models(),
    };
    need_running(ui, rt)?;
    for m in &models {
        ui.rule(&format!("Pulling {m}{}", size_note(m)));
        download(ui, rt, m)?;
        ui.plain_rule();
    }
    ui.success(&format!("Models ready: {}.", models.join(", ")));
    Ok(0)
}

fn rm(ui: &Ui, rt: &dyn Runtime, model: &str, yes: bool) -> Result<i32> {
    need_llm(rt)?;
    let model = config::parse_model_tag(model)?;
    if !yes {
        if !ui.interactive {
            bail!("Refusing to delete a model without confirmation (use --yes).");
        }
        let question = format!(
            "Delete the model {} from {MODELS_VOLUME}? It is shared by all projects on this machine.",
            ui.bold(&model)
        );
        if !ui.confirm(&question) {
            return Ok(0);
        }
    }
    need_running(ui, rt)?;
    let r = ui.spin(&format!("Deleting {model}…"), || {
        exec(ui, rt, "ollama", &["ollama", "rm", &model])
    });
    if r.exit_code != 0 {
        ui.show_output("ollama rm", &r);
        bail!("Could not delete {model}.");
    }
    ui.success(&format!("Deleted {model}."));
    Ok(0)
}

// --- usage ----------------------------------------------------------------------

/// One day and model of the usage log.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct UsageRow {
    pub day: String,
    pub model: String,
    pub requests: u64,
    /// Answers with a status of 400 or above.
    pub errors: u64,
    pub input: u64,
    pub output: u64,
    /// Time of the requests that reported output tokens, in seconds.
    pub busy_secs: f64,
}

impl UsageRow {
    /// Output tokens per second over the requests that reported them.
    pub fn tokens_per_second(&self) -> Option<f64> {
        (self.output > 0 && self.busy_secs > 0.0).then(|| self.output as f64 / self.busy_secs)
    }
}

/// Groups the lines of the usage log by day and model. `since` is the first day to keep (`YYYY-MM-DD`); lines that
/// aren't JSON are skipped.
pub fn aggregate(log: &str, since: Option<&str>) -> Vec<UsageRow> {
    let mut rows: BTreeMap<(String, String), UsageRow> = BTreeMap::new();
    for line in log.lines() {
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let day: String = v["ts"].as_str().unwrap_or("").chars().take(10).collect();
        if day.is_empty() || since.is_some_and(|s| day.as_str() < s) {
            continue;
        }
        let model = match v["model"].as_str() {
            Some(m) if !m.is_empty() => m.to_string(),
            _ => "(unknown)".to_string(),
        };
        let row = rows
            .entry((day.clone(), model.clone()))
            .or_insert_with(|| UsageRow {
                day,
                model,
                ..UsageRow::default()
            });
        row.requests += 1;
        if v["status"].as_u64().unwrap_or(0) >= 400 {
            row.errors += 1;
        }
        row.input += v["in"].as_u64().unwrap_or(0);
        if let Some(out) = v["out"].as_u64() {
            row.output += out;
            if out > 0 {
                row.busy_secs += v["secs"].as_f64().unwrap_or(0.0);
            }
        }
    }
    rows.into_values().collect()
}

/// `YYYY-MM-DD` of the day `days_ago` days before the given Unix time (UTC).
pub fn day_before(unix_seconds: u64, days_ago: u64) -> String {
    let days =
        i64::try_from(unix_seconds / 86_400).unwrap_or(0) - i64::try_from(days_ago).unwrap_or(0);
    // civil-from-days (Howard Hinnant)
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

fn usage(ui: &Ui, rt: &dyn Runtime, days: Option<u32>, as_json: bool, clear: bool) -> Result<i32> {
    need_llm(rt)?;
    need_running(ui, rt)?;
    if clear {
        let r = exec(ui, rt, "llm", &["sh", "-c", &format!(": > {USAGE_LOG}")]);
        if r.exit_code != 0 {
            ui.show_output("clear the usage log", &r);
            bail!("Could not clear the usage log.");
        }
        ui.success("Usage log cleared.");
        return Ok(0);
    }
    let r = exec(ui, rt, "llm", &["cat", USAGE_LOG]);
    if r.exit_code != 0 {
        ui.show_output("read the usage log", &r);
        bail!("Could not read the usage log.");
    }
    let since = days.map(|n| {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |t| t.as_secs());
        day_before(now, u64::from(n.max(1)) - 1)
    });
    let rows = aggregate(&r.stdout, since.as_deref());
    if as_json {
        let items: Vec<Value> = rows
            .iter()
            .map(|r| {
                json!({
                    "day": r.day,
                    "model": r.model,
                    "requests": r.requests,
                    "errors": r.errors,
                    "input_tokens": r.input,
                    "output_tokens": r.output,
                    "output_tokens_per_second": r.tokens_per_second().map(|t| (t * 10.0).round() / 10.0),
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&items)?);
        return Ok(0);
    }
    if rows.is_empty() {
        println!("No requests recorded yet.");
        return Ok(0);
    }
    let mut t = ui.new_table(&[
        "Day",
        "Model",
        "Requests",
        "Errors",
        "Input",
        "Output",
        "Out tok/s",
    ]);
    crate::ui::table::align_right(&mut t, 2);
    crate::ui::table::align_right(&mut t, 3);
    crate::ui::table::align_right(&mut t, 4);
    crate::ui::table::align_right(&mut t, 5);
    crate::ui::table::align_right(&mut t, 6);
    for r in &rows {
        t.add_row([
            r.day.clone(),
            r.model.clone(),
            r.requests.to_string(),
            r.errors.to_string(),
            r.input.to_string(),
            r.output.to_string(),
            r.tokens_per_second()
                .map_or("-".to_string(), |t| format!("{t:.1}")),
        ]);
    }
    ui.print_table(
        Some(&ui.bold("Local LLM usage")),
        &t,
        Some(&ui.dim("Tokens are counted when the answer reports them (streamed OpenAI answers only with include_usage).")),
    );
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_output_is_read_by_its_first_column() {
        let text = "NAME            ID      SIZE    MODIFIED\nqwen3.6:27b  abc  17 GB  2 days ago\nllama3:latest  def  4 GB  1 week ago\n";
        assert_eq!(parse_list(text), vec!["qwen3.6:27b", "llama3:latest"]);
        assert!(parse_list("NAME ID SIZE MODIFIED\n").is_empty());
        assert!(parse_list("").is_empty());
    }

    #[test]
    fn a_model_without_a_tag_is_latest() {
        assert_eq!(full_tag("llama3"), "llama3:latest");
        assert_eq!(full_tag("qwen3.6:27b"), "qwen3.6:27b");
        assert_eq!(full_tag("hf.co/org/repo"), "hf.co/org/repo:latest");
        assert_eq!(full_tag("hf.co/org/repo:Q4_K_M"), "hf.co/org/repo:Q4_K_M");
    }

    const LOG: &str = concat!(
        "{\"ts\":\"2026-11-13T10:00:00+00:00\",\"path\":\"/api/chat\",\"status\":200,\"secs\":2.000,\"model\":\"qwen3.6:27b\",\"in\":100,\"out\":50,\"stream\":true}\n",
        "{\"ts\":\"2026-11-13T11:00:00+00:00\",\"path\":\"/v1/chat/completions\",\"status\":200,\"secs\":3.000,\"model\":\"qwen3.6:27b\",\"in\":200,\"out\":100,\"stream\":false}\n",
        "{\"ts\":\"2026-11-13T12:00:00+00:00\",\"path\":\"/v1/chat/completions\",\"status\":404,\"secs\":0.010,\"model\":\"\",\"in\":null,\"out\":null,\"stream\":false}\n",
        "not json\n",
        "{\"ts\":\"2026-11-14T09:00:00+00:00\",\"path\":\"/v1/messages\",\"status\":200,\"secs\":1.000,\"model\":\"qwen3.6:27b\",\"in\":null,\"out\":null,\"stream\":true}\n",
    );

    #[test]
    fn usage_is_summed_per_day_and_model() {
        let rows = aggregate(LOG, None);
        assert_eq!(rows.len(), 3);
        let first = &rows[0];
        assert_eq!(
            (first.day.as_str(), first.model.as_str()),
            ("2026-11-13", "(unknown)")
        );
        assert_eq!((first.requests, first.errors), (1, 1));
        let second = &rows[1];
        assert_eq!(second.model, "qwen3.6:27b");
        assert_eq!((second.requests, second.errors), (2, 0));
        assert_eq!((second.input, second.output), (300, 150));
        // 150 tokens in 5 s
        assert_eq!(second.tokens_per_second(), Some(30.0));
        let third = &rows[2];
        assert_eq!(
            (third.day.as_str(), third.requests, third.output),
            ("2026-11-14", 1, 0)
        );
        assert_eq!(third.tokens_per_second(), None);
    }

    #[test]
    fn usage_can_start_at_a_day() {
        let rows = aggregate(LOG, Some("2026-11-14"));
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].day, "2026-11-14");
        assert!(aggregate("", None).is_empty());
    }

    #[test]
    fn days_are_counted_back_in_the_calendar() {
        // 2026-11-14T12:00:00Z
        let now = 1_794_657_600;
        assert_eq!(day_before(now, 0), "2026-11-14");
        assert_eq!(day_before(now, 1), "2026-11-13");
        assert_eq!(day_before(now, 14), "2026-10-31");
        assert_eq!(day_before(now, 318), "2025-12-31");
        assert_eq!(day_before(0, 0), "1970-01-01");
        // leap day
        assert_eq!(day_before(1_709_208_000, 0), "2024-02-29");
    }
}
