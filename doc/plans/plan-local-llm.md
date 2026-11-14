# Local LLM support (Ollama + filtering gateway, per project)

> **Status:** implemented; not yet tried on a real host (no Docker daemon or GPU where it was written). See *As built*.

## Context

Agents in the sandbox should be able to use a local model (e.g. on an RTX 4090) without installing anything on the
host except Docker. Today the only option is a commented-out `host.docker.internal` allowlist entry, which needs
Ollama/LM Studio installed on the host. Goal:

- The LLM runs in its own container, part of the project's compose stack. It starts with `up` when the project
  enables it.
- `sandbox.toml` says which model the project uses. `sandbox.local.toml` can turn it off on this machine, change the
  model or pick the GPU type.
- Models live in one machine-wide Docker volume, so they survive engine updates, `down`, `reset-home` and are shared
  by all projects (one download per model).
- Works with any tool (OpenCode, Copilot BYOK, Continue, Claude Code, ...) through standard APIs. Usage (requests,
  tokens, speed) is recorded and shown by the CLI.
- Works on Linux and on Windows (Docker Desktop with the WSL2 backend and an NVIDIA GPU).

Decisions made with the user: one LLM stack **per project**; **Ollama** behind a small **nginx + njs gateway**
(no LiteLLM); usage as **JSON log + `sandbox llm usage`**; **local models only** for now (cloud models stay in each
tool's own login; tools like OpenCode can use cloud and local side by side).

## Architecture

```
desktop ──(sandbox net, NO_PROXY)──► llm  (nginx-unprivileged + njs: path allowlist, usage log)
                                       │  llm net (internal)
                                       ▼
                                     ollama (GPU, volume sandbox-llm-models) ──(llm-egress)──► model registry
```

- The desktop never reaches Ollama directly. The gateway passes only inference and read-only endpoints:
  `/`, `/api/version|tags|show|ps|chat|generate|embed|embeddings`, `/v1/chat/completions|completions|embeddings|models`,
  `/v1/messages` (+ `/v1/messages/count_tokens`). Everything else (`/api/pull|push|create|delete|copy|blobs`) gets
  403 with a JSON hint ("pull models on the host: sandbox llm pull <model>"). This keeps the agent from using Ollama's
  pull to reach arbitrary hosts and bypass the allowlist.
- Ollama has its own egress network only for downloads, which the CLI starts via `docker compose exec ollama ollama
  pull`. Desktop allowlist is unchanged.
- Ollama speaks OpenAI (`/v1`), Anthropic (`/v1/messages`, since v0.14) and its own API, so all tools work without a
  translation layer.

## Configuration

`sandbox.toml` (committed):

```toml
[llm]
model = "qwen3.6:27b"     # Ollama tag; setting a model (or enabled = true) turns the local LLM on
# enabled = true          # on with the built-in default model
# models = ["gpt-oss:20b"]  # extra models pulled too
# context = 65536         # context length (OLLAMA_CONTEXT_LENGTH)
# keep_alive = "30m"      # how long a model stays in VRAM after the last request
```

`sandbox.local.toml` (not committed): allow `[llm]` besides `[host]`, with keys `enabled` (false = off on this
machine even if the project turns it on), `model` (replaces the project's main model), `context`, `gpu`
(`"nvidia"` default | `"amd"` (Linux, ROCm image + `/dev/kfd`, `/dev/dri`) | `"none"` (CPU)).

Defaults: off unless `[llm]` turns it on; built-in default model `qwen3.6:27b`; `context = 65536` with
`OLLAMA_FLASH_ATTENTION=1` and `OLLAMA_KV_CACHE_TYPE=q8_0` (Claude Code's system prompt alone is >20K tokens).
Same strict validation as everything else (unknown keys, wrong types, model tag pattern, `context` 2048..1M,
`keep_alive` duration).

## Changes

### Config — `src/config/mod.rs`
- New `LlmSettings { model, extra_models, context, keep_alive, gpu, sources }` in `Settings` (`Option`, `None` = off),
  with `BrowserSource`-like source tracking (generalise `BrowserSource` → `SettingSource` and reuse it).
- `local_host()` → `read_local()` returns a `LocalSettings { browser, llm }`; the "only `[host]`" check becomes
  "only `[host]` and `[llm]`" (update message + tests `the_local_file_may_only_have_host`).
- Add `SANDBOX_LLM_URL`, `SANDBOX_LLM_MODEL`, `OLLAMA_HOST` to `RESERVED_ENV` (with hint `llm.model`).

### Compose — `resources/compose.yml.j2`, `src/compose.rs`
- `Model` gets `llm: Option<Llm { model, context, keep_alive, gpu }>`; all new blocks are `{% if llm %}`.
- Networks `llm: {internal: true}` and `llm-egress: {}`.
- Service `ollama`: image `ollama/ollama:<pinned ≥0.14>` (`:<ver>-rocm` for amd), volume
  `models:/root/.ollama`, env `OLLAMA_HOST=0.0.0.0:11434`, context, keep-alive, flash attention, KV cache type;
  `no-new-privileges`; healthcheck `ollama list`; GPU via
  `deploy.resources.reservations.devices: [{driver: nvidia, count: all, capabilities: [gpu]}]` (works on Docker
  Desktop/WSL2; `device_ids` does not, so don't use it), AMD via `devices`.
- Service `llm` (gateway): `<<: *hardened`-style (read-only, cap_drop ALL, tmpfs `/tmp`), build `./.state/llm`,
  networks `[sandbox, llm]`, volume `llm-usage:/var/log/llm`, `depends_on: ollama (healthy)`.
- Desktop: `SANDBOX_LLM_URL=http://llm:11434`, `SANDBOX_LLM_MODEL`, `OLLAMA_HOST=http://llm:11434`, add `llm` to
  `NO_PROXY`/`no_proxy`. Do **not** set `OPENAI_BASE_URL`/`ANTHROPIC_BASE_URL` globally (would hijack cloud logins).
- `volumes: models: {external: true, name: sandbox-llm-models}` and per-project `llm-usage:`. External so
  `down -v` can never delete the models.

### Engine — `resources/llm/`, `src/engine/mod.rs`
- Add `"llm"` to `FOLDERS` (synced to `.state/llm/`, hashed, stale files removed like desktop/proxy).
- `resources/llm/Dockerfile`: `FROM nginxinc/nginx-unprivileged:<pinned>-alpine` (ships njs), `/var/log/llm`
  owned by uid 101 so the named volume inherits it.
- `resources/llm/nginx.conf`: `listen 11434`, Docker DNS resolver, the location allowlist above, `proxy_buffering off`,
  long `proxy_read_timeout`, `js_body_filter usage.filter` + `js_set` vars on inference locations, JSON
  `log_format` (`ts, path, status, ms, model, in, out, stream`) to `/var/log/llm/usage.jsonl`; normal access log to
  stdout.
- `resources/llm/usage.js` (~80 lines): reads model and token counts from the response, line by line for
  NDJSON/SSE (keeps a partial-line buffer across chunks), whole body (capped) otherwise. Fields: Ollama
  `prompt_eval_count`/`eval_count`; OpenAI `usage.prompt_tokens`/`completion_tokens` (streams only when the client
  sends `stream_options.include_usage`; logged as unknown otherwise); Anthropic `message_start.usage.input_tokens`
  + `message_delta.usage.output_tokens`.

### Desktop helper — `resources/desktop/bin/sandbox-llm`
- `sandbox-llm` (no args): endpoint, model, reachable?, and ready-to-paste config for OpenCode (OpenAI-compatible
  provider, `baseURL http://llm:11434/v1`), VS Code Copilot BYOK (Ollama endpoint `http://llm:11434`; confirm the
  current setting name when implementing), Continue.
- `sandbox-llm claude [args]`: runs Claude Code with `ANTHROPIC_BASE_URL=$SANDBOX_LLM_URL`,
  `ANTHROPIC_AUTH_TOKEN=local`, `ANTHROPIC_API_KEY=`, `--model $SANDBOX_LLM_MODEL` and the default
  haiku/sonnet/opus/subagent model variables pointed at the local model (background calls would fail otherwise).
  Plain `claude` stays on the cloud login, so both can run side by side in two terminals.
- `sandbox-llm env`: prints `export` lines for scripts.

### CLI — `src/cli.rs`, new `src/commands/llm.rs`, `src/commands/lifecycle.rs`, `maintenance.rs`
- `up` / `rebuild` (when LLM is on): `docker volume create sandbox-llm-models` (idempotent) before compose up; after
  start, `compose exec -T ollama ollama list`, then pull missing models interactively (`compose_interactive`, shows
  Ollama's progress) with a note about size. `up --no-pull` skips it.
- GPU failure hint: if compose up fails with "could not select device driver"/nvidia errors, say what's needed
  (Linux: NVIDIA Container Toolkit; Windows: Docker Desktop WSL2 backend + current Windows NVIDIA driver) and that
  `gpu = "none"` under `[llm]` in `sandbox.local.toml` runs on CPU.
- `sandbox llm status`: model(s), source file, GPU mode, endpoint, `ollama list` + `ollama ps` (loaded, VRAM/CPU split).
- `sandbox llm pull [model]` (default: the project's models), `sandbox llm rm <model>` (asks first: the volume is
  shared by all projects; `--yes`).
- `sandbox llm usage [--days N] [--json] [--clear]`: reads the usage log via `compose exec -T llm cat`, table per day
  and model: requests, errors, input/output tokens, avg output tok/s. Uses `ui.new_table`.
- `info`: an "LLM" row (model, from which file, GPU, endpoint or "off").
- `update`: also pulls the Ollama/nginx base images (already covered if it uses `build --pull` + `pull`; check).
- `down` / `reset-home` never touch `sandbox-llm-models`.

### Templates and docs
- `resources/init/sandbox.toml`: commented `[llm]` section with the model table below.
- `resources/init/allowlist.txt`: in the "Local LLM on the host" group, point to `[llm]` as the built-in way.
- `README.md`: "Local LLM" section (config, endpoints per tool, Windows requirements, usage, models, disk space:
  on Docker Desktop the volume lives in the WSL2 VM disk, which does not shrink by itself).
- `doc/plans/plan-local-llm.md`: copy of this plan, like the other plans.

## Model recommendations (24 GB, RTX 4090)

Verify each tag with `ollama show <tag>` (tools capability) during implementation.

| Model (Ollama tag) | Size (Q4) | Notes |
|---|---|---|
| `qwen3.6:27b` (default) | ~17 GB | Dense, best tool-call reliability in 2026 reviews, ~30–40 tok/s, room for 64K context. One May 2026 report said Ollama couldn't load it (vision projector); the registry lists it now; fall back to the MoE if it fails |
| `qwen3.6:35b-a3b` | ~22 GB | MoE, ~120–150 tok/s, little VRAM left for context on 24 GB |
| `gpt-oss:20b` | ~14 GB | Clean tool calls, adjustable reasoning, lots of room for context; also fits 16 GB cards |
| `devstral-small-2` (24B) | ~15 GB | Mistral's agentic coding model |
| `qwen3-coder:30b` (30B-A3B) | ~19 GB | Older but widely supported coding model |

## Tests (`tests/e2e`, unit tests)

- `compose_file.rs`: exact output without `[llm]` unchanged; with `[llm]`: services, networks, volumes, env,
  `NO_PROXY`; `gpu = "none"` (no `deploy`), `"amd"` (rocm image + devices); local override of model/enabled.
- `errors.rs`: every new validation message; `sandbox.local.toml` with `[llm]` allowed, other sections still rejected.
- `commands.rs`: `up` with LLM → `volume create`, compose up, `exec ollama ollama list`, `pull` for missing models
  (fake-docker rules); `--no-pull`; `llm status|pull|rm|usage` docker calls; usage aggregation from a canned JSONL.
- `engine.rs`: `.state/llm/` written with headers, manifest, hand-edit restore.
- Unit tests in `config/mod.rs` for the reader and in `commands/llm.rs` for usage aggregation.

## Verification

1. `cargo test` and `cargo clippy` (on the host if crates.io is blocked in this sandbox; try `--offline` first).
2. On the host with the RTX 4090 (Linux and Windows/Docker Desktop): a test project with `[llm] model = "qwen3.6:27b"`,
   `sandbox up` → model pulled once; second project → no download; `sandbox llm status` shows the model on GPU.
3. In the desktop: `curl $SANDBOX_LLM_URL/v1/models`; `curl -X POST $SANDBOX_LLM_URL/api/pull` → 403;
   `sandbox-llm claude -p "hello"` works; OpenCode with the printed config works.
4. `sandbox llm usage` shows the requests with token counts (native, OpenAI non-streaming, Anthropic streaming).
5. `sandbox.local.toml` `[llm] enabled = false` → no LLM services in `.state/compose.yml`; `gpu = "none"` starts on CPU.
6. App update (engine hash change) + `up` → models still present.

## As built

Everything in the plan is implemented as described. Differences and details worth knowing:

- **Config.** `BrowserSource` became `SettingSource`. `Settings` has `llm: Option<LlmSettings>` and `llm_off_locally`
  (for `info`). In `sandbox.local.toml`, `enabled = true` or a `model` also turns the LLM on for this machine when the
  project doesn't; `models` and `keep_alive` are read only from `sandbox.toml`, `gpu` only from `sandbox.local.toml`
  (the other file reports them as unknown keys, with the known ones listed). `llm.models` is de-duplicated and never
  repeats the main model. `keep_alive` accepts `30m`, `1h30m`, `90s`, `0`, `-1`.
- **Engine.** `llm/` is always synced to `.state/llm/` (also when `[llm]` is off, like `desktop` and `proxy`); it only
  matters to the engine hash. `llm/nginx.conf` and `llm/Dockerfile` get the usual header, `llm/usage.js` none (not `#`
  comments). `desktop/bin/sandbox-llm` is copied by the desktop Dockerfile next to `sandbox-install`.
- **Gateway.** nginx reads model and token counts from the *response* (njs body filter with `buffer_type=buffer`, so
  the bytes pass through untouched), not from the request. The usage log keys are `ts, path, status, secs, model, in,
  out, stream`; `secs` is nginx's `$request_time` (seconds), not milliseconds as the plan said. Streams are detected by
  the response's content type (`ndjson`, `event-stream`).
- **Compose.** `ollama` has `no-new-privileges` only (no `read_only`/`cap_drop`: Ollama writes outside the volume and
  runs GPU hooks); the gateway uses the `hardened` anchor with its own `volumes`. The `llm` network is internal, the
  desktop is on `sandbox` only and reaches the gateway by the name `llm` (added to `NO_PROXY`). Pinned images:
  `ollama/ollama:0.24.0` (`-rocm` for AMD) and `nginxinc/nginx-unprivileged:1.29.3-alpine`; neither tag could be checked.
- **CLI.** `up` and `rebuild`/`update` create the volume (`docker volume create sandbox-llm-models`) before compose
  starts and pull missing models afterwards (`compose exec [-T] ollama ollama pull`, `-T` only without a terminal).
  Only `up` has `--no-pull`. No separate image pull is done: compose pulls the pinned Ollama image on first start, so
  the first `up` shows "creating containers…" for a while; `update` doesn't need to pull it (the tag is pinned, a new
  app version brings a new tag). A GPU failure shows the hint when the compose output mentions the device driver or
  NVIDIA/AMD devices. `sandbox llm usage` defaults to all days; `--days N` counts today as one day (UTC) and `--clear`
  empties the log without printing it.
- **Not verified:** the model tags other than `qwen3.6:27b`, the image tags, the VS Code setting name, the OpenCode
  provider JSON, the njs script (no nginx/njs available to run it), and everything that needs Docker and a GPU: see
  the README section "Not verified yet".
