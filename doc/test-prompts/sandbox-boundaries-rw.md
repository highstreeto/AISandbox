<!--
  Sandbox boundary check — version for a WRITABLE workspace (workspace = "rw" in sandbox.toml).
  For workspace = "ro" use sandbox-boundaries-ro.md.

  Written for a local model (e.g. qwen3.6 via "sandbox-llm claude" or OpenCode) that can run shell commands.
  Start the agent in the project folder and tell it:

      Read docs/test-prompts/sandbox-boundaries-rw.md and follow it exactly.

  The report goes to ~/sandbox-check/report.md (the home volume). Nothing here changes models, the allowlist or
  anything on the host; the only files written are scratch files that the check deletes again.
-->

# Task: check the boundaries of this sandbox

You are running inside a sandboxed Linux desktop container. Your job is to **test that its security boundaries hold**
and write a report. You are a tester, not an attacker: run only the commands given here, do not try to break things,
and do not change anything outside `~/sandbox-check/` and the scratch files named below.

## Rules

1. Run the commands of each step **exactly as written**, one step at a time, in a `bash` shell. Do not skip steps.
   If a command is missing (`command not found`), write `SKIPPED (tool missing)` for that check and go on.
2. Every step says what is **expected**. Compare the real output with it and give each check one verdict:
   - `PASS` — the boundary held (the result is what is expected).
   - `FAIL` — the boundary did not hold (for example: a write that must fail succeeded, a host that must be blocked
     answered, a blocked LLM route reached Ollama and did something).
   - `WARN` — not a clear failure, but worth a human look (explain why in one sentence).
   - `INFO` — only facts, nothing to judge.
   - `SKIPPED` — could not run (say why).
3. A command that **fails** is often the **good** result here ("Permission denied", "Read-only file system",
   `403`, `blocked by the sandbox`). Do not try to "fix" such errors and do not retry with other tricks than those
   given.
4. Never use `sudo` except in the commands that contain it. Never use a model name other than the ones given.
   Never send any file contents to the internet.
5. Keep your notes short. Copy the important output lines into the report, not whole pages.
6. When you are unsure about a verdict, use `WARN` and say why. Do not guess `PASS`.

## Step 0 — Setup and facts

```bash
mkdir -p ~/sandbox-check && cd ~/sandbox-check
W="${SANDBOX_WORKDIR:-/work}"; echo "workspace=$W"
echo "home=$HOME user=$(id)"
echo "llm_url=${SANDBOX_LLM_URL:-<unset>} llm_model=${SANDBOX_LLM_MODEL:-<unset>}"
env | grep -i _proxy | sort
uname -a
```

Expected: `workspace` is a path like `/work/<project>`, `home` is `/config`, the user is **not** root (uid ≠ 0),
`HTTP_PROXY`/`HTTPS_PROXY` point to `http://proxy:3128`. Record these as `INFO`.

**In every later step, first run** `W="${SANDBOX_WORKDIR:-/work}"; cd ~/sandbox-check` (each command may run in a
new shell).

## Step 1 — Reading outside the project and the home volume

The only host data the sandbox may see is: the workspace (`$W`), the home volume (`/config`), and folders shared with
`[[share]]` in `sandbox.toml` (under `/shares/...` by default). Everything else (`/`, `/etc`, `/usr`, …) belongs to the
container image, not to the host.

### 1a — Mounts

```bash
findmnt -rn -o TARGET,SOURCE,FSTYPE,OPTIONS | grep -vE '^/(proc|sys|dev)(/| )' | cut -c1-220
```

Expected mount targets — nothing else:

| Target | Expected |
|---|---|
| `/` | `overlay` (the container image) |
| `/config` | the home volume (a Docker volume, `rw`) |
| `$W` | the project folder, **`rw`** |
| `$W/.sandbox` | **`ro`** (may be missing if the whole workspace is read-only) |
| `$W/.git` | `ro` if `protect_git = true`; absent otherwise |
| `/shares/...` or other `[[share]]` paths | only if configured; note `ro`/`rw` |
| `/etc/hosts`, `/etc/hostname`, `/etc/resolv.conf` | Docker's files, fine |

Verdict: `PASS` if every target is in this table. `FAIL` for any other host folder (for example `/mnt/c`, `/host`,
`/var/run/docker.sock`, a home directory of the host, a whole drive). List each mount with its `ro`/`rw` in the
report.

### 1b — Host paths that must not exist

```bash
for p in /var/run/docker.sock /run/docker.sock /run/containerd/containerd.sock /var/lib/docker \
         /mnt/c /mnt/wsl /mnt/host /host /hostfs /rootfs /Users /media; do
  if [ -e "$p" ]; then echo "EXISTS  $p"; ls -la "$p" 2>&1 | head -5; else echo "absent  $p"; fi
done
docker ps 2>&1 | head -2
```

Expected: every path `absent` (an empty `/media` from the image is fine: `WARN` only if it holds files), and
`docker ps` fails with "Cannot connect to the Docker daemon". `FAIL` if a Docker socket exists or `docker ps` lists
containers.

### 1c — Siblings of the project on the host

```bash
ls -la "$(dirname "$W")"
ls -la "$W/.." | head -20
```

Expected: the parent folder holds **only** the project folder (it is a folder inside the container). `FAIL` if you see
other projects or folders of the host next to it.

### 1d — Privileged files and devices

```bash
cat /etc/shadow 2>&1 | head -1
grep -E '^Cap(Prm|Eff)' /proc/self/status
ls /dev | tr '\n' ' '; echo
for d in /dev/sda /dev/sdb /dev/nvme0n1 /dev/vda /dev/mem /dev/kmsg; do
  [ -e "$d" ] && { echo "EXISTS $d"; head -c 1 "$d" >/dev/null 2>&1 && echo "  READABLE $d" || echo "  not readable"; }
done
ls /proc/1/root/ 2>&1 | head -3
sudo -n true 2>&1; echo "sudo rc=$?"
```

Expected: `/etc/shadow` → "Permission denied"; `CapPrm` and `CapEff` are `0000000000000000`; no disk devices
(`sda`, `nvme…`, `vda`), and nothing marked `READABLE`; `/proc/1/root` → "Permission denied"; `sudo` → a password is
required or "not allowed" (rc ≠ 0). If `sudo rc=0`, sudo is enabled (`sudo = true` in `sandbox.toml`): mark `WARN`
and run 1e; otherwise skip 1e.

### 1e — Only if sudo works (rc=0 in 1d)

```bash
sudo -n mount -t tmpfs none /mnt 2>&1; echo "mount rc=$?"
sudo -n sh -c 'v=$(cat /proc/sys/kernel/sysrq); echo "$v" > /proc/sys/kernel/sysrq' 2>&1; echo "sysctl rc=$?"
sudo -n grep CapEff /proc/self/status
sudo -n ls /proc/1/root/ | head -3
```

Expected: `mount` fails (rc ≠ 0, "permission denied"), the `/proc/sys` write (it writes the current value back, so nothing changes) fails ("Read-only file system").
`FAIL` if `mount` or the sysctl write succeeds. If `mount` succeeded, run `sudo -n umount /mnt` right away. Root
seeing `/proc/1/root` is expected (it is the container's own root): `INFO`.

## Step 2 — Writing outside the project and the home volume

### 2a — Control writes (must work)

```bash
echo ok > "$W/.sandbox-check-$$" && echo "workspace write OK" && rm -f "$W/.sandbox-check-$$"
echo ok > ~/sandbox-check/write-test && echo "home write OK" && rm -f ~/sandbox-check/write-test
echo ok > /tmp/sandbox-check-$$ && echo "tmp write OK" && rm -f /tmp/sandbox-check-$$
```

Expected: all three print `OK`. The workspace is writable in this version (`workspace = "rw"`); if it is not, the
wrong prompt version is in use — write `WARN: workspace is read-only, use sandbox-boundaries-ro.md` and go on.
`/tmp` is the container's own disk (lost when the container is recreated): `INFO`.

### 2b — Writes that must fail

```bash
# opens for append without writing anything; only files this check created are removed again
try() { local new=0; [ -e "$1" ] || new=1
        if ( : >> "$1" ) 2>/dev/null; then echo "WRITABLE   $1"; [ $new = 1 ] && rm -f "$1"; else echo "denied     $1"; fi; }
try "$(dirname "$W")/sandbox-escape-$$"
try "$W/.sandbox/sandbox-check-$$"
try "$W/.sandbox/allowlist.txt.check"
[ -d "$W/.git" ] && try "$W/.git/sandbox-check-$$"
try /etc/sandbox-check-$$
try /usr/sandbox-check-$$
try /sandbox-check-$$
try /proc/sys/kernel/hostname
try /sys/fs/cgroup/sandbox-check-$$
[ -f "$W/.sandbox/allowlist.txt" ] && { cp -p "$W/.sandbox/allowlist.txt" ~/sandbox-check/allowlist.copy; \
  ( echo "# sandbox-check" >> "$W/.sandbox/allowlist.txt" ) 2>&1 | head -1; \
  cmp -s "$W/.sandbox/allowlist.txt" ~/sandbox-check/allowlist.copy && echo "allowlist unchanged" || echo "ALLOWLIST CHANGED"; }
```

Expected: every line says `denied`, the append to `allowlist.txt` fails with "Read-only file system", and
`allowlist unchanged`.

- The parent of the workspace, `$W/.sandbox`, `/proc/sys` and `/sys/fs/cgroup` must be `denied`: `FAIL` otherwise.
- `$W/.git`: `denied` if `protect_git = true` (the default). If it is `WRITABLE`, report `WARN` ("protect_git is off or
  not working").
- `/etc`, `/usr`, `/`: `denied` for a normal user. These are inside the container, not the host, so `WRITABLE` is
  `WARN`, not `FAIL`.
- `ALLOWLIST CHANGED` is a `FAIL`. Then restore it right away:
  `cp ~/sandbox-check/allowlist.copy "$W/.sandbox/allowlist.txt"`.

### 2c — Symlink out of the workspace (information only)

```bash
ln -s /etc/passwd "$W/.sandbox-check-link" && readlink "$W/.sandbox-check-link"; rm -f "$W/.sandbox-check-link"
```

A link in the workspace points into the **container**, not into the host, so this is no escape: `INFO`. (It only
matters if a tool on the host follows links in the project; mention that in one sentence.)

## Step 3 — Networks and which hosts can be reached

The desktop is on an internal Docker network **without** an internet route and **without** external DNS. All traffic
must go through the proxy `proxy:3128`, which only lets through the hosts listed in `$W/.sandbox/allowlist.txt`.

### 3a — Interfaces, routes, names

```bash
ip -br addr 2>/dev/null || cat /proc/net/dev
ip route 2>/dev/null || cat /proc/net/route
cat /etc/resolv.conf | grep -v '^#'
for n in proxy gateway llm ollama github.com example.com host.docker.internal; do
  printf '%-22s ' "$n"; getent hosts "$n" | head -1 || echo "(no address)"; echo
done
```

Expected: only `lo` and one `eth0` on a private range (172.x / 10.x / 192.168.x); **no `default` route**; `proxy` and
`gateway` resolve (and `llm` when the local LLM is on); `ollama`, `github.com`, `example.com` and
`host.docker.internal` do **not** resolve. `FAIL` if there is a default route or an internet name resolves.

### 3b — Direct internet access (without the proxy) must fail

```bash
for u in https://github.com https://1.1.1.1 http://8.8.8.8 https://api.anthropic.com; do
  curl -s --noproxy '*' --max-time 6 -o /dev/null -w "%{http_code}" "$u"; echo "  rc=$?  $u"
done
for hp in 1.1.1.1:53 8.8.8.8:53 1.1.1.1:443 host.docker.internal:2375 host.docker.internal:22; do
  timeout 4 bash -c "</dev/tcp/${hp%:*}/${hp#*:}" 2>/dev/null && echo "OPEN    $hp" || echo "closed  $hp"
done
ping -c1 -W2 1.1.1.1 >/dev/null 2>&1 && echo "PING WORKS" || echo "ping blocked"
```

Expected: every `curl` has `000` and rc ≠ 0, every port `closed`, `ping blocked`. Any answer is a `FAIL`.

### 3c — Hosts through the proxy

First the active allowlist (comment lines and text after `#` do not count):

```bash
grep -vE '^\s*(#|$)' "$W/.sandbox/allowlist.txt" 2>/dev/null | sed 's/\s*#.*//' | sort -u
```

Then try common hosts:

```bash
check() {
  local url=$1 r rc http conn v
  r=$(curl -s -o /tmp/sc-body --max-time 15 -w '%{http_code} %{http_connect}' "$url"); rc=$?
  read -r http conn <<<"${r:-000 000}"
  if [ "$conn" = 403 ]; then v=BLOCKED
  elif grep -q 'Generated by tinyproxy' /tmp/sc-body 2>/dev/null; then
    if [ "$http" = 403 ]; then v=BLOCKED; else v="PROXY-ERROR"; fi
  elif [ "$rc" = 0 ]; then v=REACHABLE
  else v="ERROR(rc=$rc)"; fi
  printf '%-48s %-12s http=%s connect=%s\n' "$url" "$v" "$http" "$conn"; rm -f /tmp/sc-body
}
for h in github.com api.github.com codeload.github.com raw.githubusercontent.com objects.githubusercontent.com ghcr.io \
         api.nuget.org www.nuget.org globalcdn.nuget.org \
         index.crates.io static.crates.io crates.io docs.rs static.rust-lang.org \
         registry.npmjs.org www.npmjs.com registry.yarnpkg.com \
         pypi.org files.pythonhosted.org proxy.golang.org repo.maven.apache.org \
         archive.ubuntu.com security.ubuntu.com \
         registry-1.docker.io hub.docker.com registry.ollama.ai huggingface.co \
         api.anthropic.com claude.ai api.openai.com \
         www.google.com example.com pastebin.com transfer.sh webhook.site; do
  check "https://$h/"
done
```

Expected: a host is `REACHABLE` **only** if it matches an allowlist line (`*.example.com` matches any subdomain);
every other host is `BLOCKED`. `ERROR(rc=…)` or `PROXY-ERROR` for an allowlisted host means the proxy let it through
but the site did not answer: `WARN`.

In the report, make a table `host | result | in allowlist? | verdict`. Verdict `FAIL` for any host that is
`REACHABLE` but not allowlisted; `PASS` otherwise. Also list which of these ecosystems work: GitHub, NuGet, crates.io,
npm, PyPI, Go, Maven, Ubuntu apt, Docker Hub, Hugging Face (`INFO`).

## Step 4 — The proxy cannot be bypassed or reconfigured from inside

Pick one host that was `REACHABLE` in 3c and set it as `A` (for example `A=index.crates.io`). If none was reachable,
use `A=github.com` and note that the "allowed" lines below cannot pass.

```bash
A=index.crates.io      # <- replace with a REACHABLE host from step 3c
p() { curl -s -o /tmp/sc-body --max-time 15 -w '%{http_code} connect=%{http_connect}' "$@"; echo "  rc=$?  :: ${*: -1}";
      grep -o '<title>[^<]*' /tmp/sc-body 2>/dev/null | head -1; rm -f /tmp/sc-body; }
# 4a — tunnels only to port 443
p -p "http://$A:22/"
p -p "http://$A:80/"
p "https://$A:8443/"
# 4b — look-alike names and raw IPs
p "https://$A.example.org/"
p "https://evil$A/"
p "https://140.82.112.3/"
p "http://1.1.1.1/"
# 4c — Host header and absolute-URL tricks
p -H "Host: $A" "http://example.com/"
p "http://example.com/@$A/"
p "http://$A@example.com/"
# 4d — the host and the Docker API through the proxy
p "http://host.docker.internal:2375/version"
p "http://host.docker.internal/"
p "http://ollama:11434/api/version"
p "http://localhost:3128/"
```

Expected: every line is blocked — `connect=403` or `<title>403 Filtered` (or a connect error with rc ≠ 0). `FAIL` if
any of them returns a page from the target (Docker API JSON, `example.com` content, an Ollama version, …).
(`host.docker.internal` is only allowed if the user put it in the allowlist; then `WARN`.)

### 4e — No way to change or reload the proxy

```bash
p "http://tinyproxy.stats/"
for port in 22 80 443 2375 3128 8080 8888 9000; do
  timeout 3 bash -c "</dev/tcp/proxy/$port" 2>/dev/null && echo "proxy:$port OPEN" || echo "proxy:$port closed"
done
pgrep -a tinyproxy || echo "no tinyproxy process in this container"
ls -la "$W/.sandbox/" 2>&1 | head
command -v sandbox && sandbox reload 2>&1 | head -3
```

Expected:
- `tinyproxy.stats` → `403 Filtered` (the stats page is not reachable). `WARN` if a statistics page appears.
- On `proxy` only port `3128` is open. `FAIL` for any other open port.
- No `tinyproxy` process here (it runs in its own container, so it cannot be signalled from here).
- `$W/.sandbox/` is mounted read-only (shown in 1a, write denied in 2b), so the allowlist cannot be edited from here.
- `sandbox reload` is either not found or fails (no Docker daemon in here). `FAIL` if it reports a successful reload.

Verdict for this step: `PASS` only if all of the above hold.

## Step 5 — The local LLM gateway

Skip this step (`SKIPPED: local LLM is off`) if `SANDBOX_LLM_URL` was `<unset>` in step 0.

The gateway (`llm:11434`) must pass **only** inference and read-only routes (version, model list, model info) and
refuse everything that manages models (pull, push, create, copy, delete, blobs) with **HTTP 403** and the message
`blocked by the sandbox`. All tests use the made-up model `sandbox-check-does-not-exist:latest`, so even a broken
gateway cannot download or delete a real model.

```bash
U="${SANDBOX_LLM_URL:-http://llm:11434}"; M="${SANDBOX_LLM_MODEL}"; X="sandbox-check-does-not-exist:latest"
t() { # method path [json]
  rm -f /tmp/sc-llm
  local code; code=$(curl -s --path-as-is --max-time 120 -X "$1" -o /tmp/sc-llm -w '%{http_code}' \
    -H 'Content-Type: application/json' ${3:+-d "$3"} "$U$2")
  printf '%-7s %-46s %s  %s\n' "$1" "$2" "$code" "$(head -c 140 /tmp/sc-llm 2>/dev/null | tr '\n' ' ')"
}
echo "== allowed =="
t GET  /api/version
t GET  /api/tags
t GET  /v1/models
t POST /api/show "{\"model\":\"$M\"}"
t POST /api/generate "{\"model\":\"$M\",\"prompt\":\"Say OK\",\"stream\":false,\"options\":{\"num_predict\":5}}"
t POST /v1/chat/completions "{\"model\":\"$M\",\"messages\":[{\"role\":\"user\",\"content\":\"Say OK\"}],\"max_tokens\":5}"
t POST /api/generate "{\"model\":\"$X\",\"prompt\":\"hi\",\"stream\":false}"
echo "== must be blocked =="
t POST   /api/pull   "{\"model\":\"$X\",\"stream\":false}"
t POST   /api/push   "{\"model\":\"$X\",\"stream\":false}"
t POST   /api/create "{\"model\":\"$X\",\"from\":\"$X\",\"stream\":false}"
t POST   /api/copy   "{\"source\":\"$X\",\"destination\":\"$X-copy\"}"
t DELETE /api/delete "{\"model\":\"$X\"}"
t HEAD   /api/blobs/sha256:0000000000000000000000000000000000000000000000000000000000000000
t POST   /api/blobs/sha256:0000000000000000000000000000000000000000000000000000000000000000 "x"
echo "== path tricks (must be blocked too) =="
t POST /api/pull/                                "{\"model\":\"$X\",\"stream\":false}"
t POST //api/pull                                "{\"model\":\"$X\",\"stream\":false}"
t POST /API/PULL                                 "{\"model\":\"$X\",\"stream\":false}"
t POST /api/chat/../pull                         "{\"model\":\"$X\",\"stream\":false}"
t POST /api/%70ull                               "{\"model\":\"$X\",\"stream\":false}"
t POST '/v1/models/x/..%2F..%2F..%2Fapi%2Fpull'  "{\"model\":\"$X\",\"stream\":false}"
t POST '/v1/models/%2e%2e/%2e%2e/api/pull'       "{\"model\":\"$X\",\"stream\":false}"
t POST '/api/tags?/api/pull'                     "{\"model\":\"$X\",\"stream\":false}"
t DELETE /api/tags                               "{\"model\":\"$X\"}"
echo "== Ollama itself must not be reachable =="
curl -s --noproxy '*' --max-time 5 http://ollama:11434/api/version; echo "  direct rc=$?"
curl -s --max-time 5 -o /dev/null -w '%{http_code}\n' http://ollama:11434/api/version
t GET /api/tags
```

Expected:

| Request | Expected |
|---|---|
| `GET /api/version`, `/api/tags`, `/v1/models` | `200` with JSON |
| `POST /api/show` (real model) | `200` |
| `POST /api/generate`, `/v1/chat/completions` (real model) | `200` with a short answer |
| `POST /api/generate` with the made-up model | `404 … not found` — Ollama does **not** download it |
| every line under "must be blocked" and "path tricks" | `403` with `blocked by the sandbox` (`HEAD` has no body: `403` alone is fine) |
| `DELETE /api/tags`, `POST /api/tags?/api/pull` | `403`, or `405`/`404` from Ollama (read-only route, nothing happens): `PASS` |
| direct `ollama:11434` | fails (rc ≠ 0 or `403` from the proxy) |
| the last `GET /api/tags` | the same model list as at the start |

Verdict per line: `PASS` as in the table. `FAIL` if a blocked request gets `200`, a body with `"status"` (for example
`pulling manifest`, `success`), or an error that shows Ollama **tried** the action (for example
`pull model manifest`, `file does not exist`, `model … not found` from `/api/delete` or `/api/copy`). `FAIL` if the
model list changed. `WARN` if an allowed route fails (the gateway may be too strict, or the model is still loading —
then retry that one line once).

## Step 6 — Clean up and write the report

```bash
rm -f "$W"/.sandbox-check-* /tmp/sandbox-check-* /tmp/sc-body /tmp/sc-llm ~/sandbox-check/allowlist.copy
ls -la "$W" | grep -i sandbox-check || echo "workspace clean"
```

Write `~/sandbox-check/report.md` with this structure (fill in real values; keep it short):

```markdown
# Sandbox boundary report — workspace rw

Date: <date>  ·  Workspace: <$W>  ·  User: <id>  ·  Local LLM: <on/off, model>

## Summary
PASS <n> · FAIL <n> · WARN <n> · SKIPPED <n>
<One line per FAIL and WARN: check id — what happened.>

## 1 Read outside project/home
| Check | Verdict | Evidence (short) |
|---|---|---|
| 1a mounts | | <list of mounts with ro/rw> |
| 1b host paths | | |
| 1c project siblings | | |
| 1d privileged files/devices | | |
| 1e root checks | | |

## 2 Write outside project/home
| Check | Verdict | Evidence |
...

## 3 Network
<interfaces/routes/DNS facts>
| Host | Result | In allowlist? | Verdict |
...
Ecosystems available: GitHub <yes/no>, NuGet …, crates.io …, npm …, PyPI …, Go …, Maven …, apt …, Docker Hub …, Hugging Face …

## 4 Proxy
| Check | Verdict | Evidence |
...

## 5 Local LLM gateway
| Request | Code | Verdict |
...

## Notes
<anything odd, in at most five lines>
```

Finally, print the **Summary** section in your answer and the path `~/sandbox-check/report.md`. Do not claim a
check passed unless you ran it and saw the expected output.
