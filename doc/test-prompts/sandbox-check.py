#!/usr/bin/env python3
"""sandbox-check — test the boundaries of an AI Sandbox from inside the desktop container.

    python3 doc/test-prompts/sandbox-check.py [--out DIR] [--no-llm] [--no-inference] [--hosts a,b] [-v]

Checks, each with a verdict (PASS / FAIL / WARN / INFO / SKIP):

  1 read   only the workspace, the home volume and [[share]] folders come from the host
  2 write  nothing outside them can be written (and the workspace only when workspace = "rw")
  3 net    no internet route or DNS; through the proxy only hosts on the allowlist
  4 proxy  no tunnels to other ports, no look-alike/IP/Host-header tricks, no admin access
  5 llm    the LLM gateway passes inference and read-only routes, refuses pull/push/create/copy/delete/blobs

Expectations come from .sandbox/sandbox.toml (read-only in the sandbox). Nothing is changed: write probes open files
for appending without writing and remove only files they created; LLM management requests use a made-up model name.
Needs only the Python 3.11+ standard library. Writes report.json and report.md to ~/sandbox-check (or --out) and
exits with 1 if any check FAILed.
"""

from __future__ import annotations

import argparse
import concurrent.futures
import datetime
import fnmatch
import http.client
import json
import os
import re
import socket
import ssl
import stat
import subprocess
import sys
import tomllib
import urllib.parse
from dataclasses import asdict, dataclass, field
from pathlib import Path

PASS, FAIL, WARN, INFO, SKIP = "PASS", "FAIL", "WARN", "INFO", "SKIP"

FAKE_MODEL = "sandbox-check-does-not-exist:latest"
ZERO_BLOB = "/api/blobs/sha256:" + "0" * 64

# Common hosts of package registries, code hosting, AI services and "drop" sites for data.
HOSTS = [
    "github.com", "api.github.com", "codeload.github.com", "raw.githubusercontent.com",
    "objects.githubusercontent.com", "gist.githubusercontent.com", "ghcr.io",
    "api.nuget.org", "www.nuget.org", "globalcdn.nuget.org",
    "index.crates.io", "static.crates.io", "crates.io", "docs.rs", "static.rust-lang.org",
    "registry.npmjs.org", "www.npmjs.com", "registry.yarnpkg.com",
    "pypi.org", "files.pythonhosted.org", "proxy.golang.org", "repo.maven.apache.org",
    "archive.ubuntu.com", "security.ubuntu.com",
    "registry-1.docker.io", "hub.docker.com", "registry.ollama.ai", "huggingface.co",
    "api.anthropic.com", "claude.ai", "api.openai.com",
    "www.google.com", "example.com", "pastebin.com", "transfer.sh", "webhook.site",
]


@dataclass
class Result:
    section: str
    id: str
    name: str
    verdict: str
    evidence: str = ""


@dataclass
class Ctx:
    workdir: Path
    home: Path
    proxy: tuple[str, int]
    settings: dict = field(default_factory=dict)
    settings_error: str | None = None
    allowlist: list[str] | None = None
    workspace_mode: str | None = None  # "ro" | "rw" from the mount
    llm_url: str | None = None
    llm_model: str | None = None
    reachable_allowed: list[str] = field(default_factory=list)
    args: argparse.Namespace | None = None


RESULTS: list[Result] = []
VERBOSE = False


def add(section: str, id_: str, name: str, verdict: str, evidence: str = "") -> None:
    r = Result(section, id_, name, verdict, evidence.strip())
    RESULTS.append(r)
    color = {PASS: "32", FAIL: "31;1", WARN: "33", INFO: "36", SKIP: "90"}[verdict]
    v = f"\033[{color}m{verdict:<4}\033[0m" if sys.stdout.isatty() else f"{verdict:<4}"
    line = f"  {v}  {id_:<6} {name}"
    if r.evidence and (VERBOSE or verdict in (FAIL, WARN)):
        ev = r.evidence if VERBOSE else r.evidence.splitlines()[0][:150]
        line += "\n" + "\n".join(f"              {x}" for x in ev.splitlines())
    print(line, flush=True)


def section(title: str) -> None:
    print(f"\n== {title}", flush=True)


# --- helpers ---------------------------------------------------------------------------------------------------------

def run(cmd: list[str], timeout: float = 15) -> tuple[int, str]:
    try:
        p = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout)
        return p.returncode, (p.stdout + p.stderr).strip()
    except FileNotFoundError:
        return 127, f"{cmd[0]}: not found"
    except subprocess.TimeoutExpired:
        return 124, "timeout"


def unescape_mount(s: str) -> str:
    return re.sub(r"\\([0-7]{3})", lambda m: chr(int(m.group(1), 8)), s)


def mounts() -> list[dict]:
    out = []
    for line in Path("/proc/self/mountinfo").read_text().splitlines():
        left, _, right = line.partition(" - ")
        f = left.split()
        r = right.split()
        out.append({
            "target": unescape_mount(f[4]),
            "root": unescape_mount(f[3]),
            "opts": f[5],
            "fstype": r[0] if r else "?",
            "source": unescape_mount(r[1]) if len(r) > 1 else "?",
        })
    return out


def is_ro(opts: str) -> bool:
    return "ro" in opts.split(",")


def can_open_for_append(path: Path) -> tuple[bool, str]:
    """True if the path could be opened for writing. Writes nothing; removes the file only if it created it."""
    existed = path.exists()
    try:
        with open(path, "ab"):
            pass
    except OSError as e:
        return False, f"{e.strerror}"
    if not existed:
        try:
            path.unlink()
        except OSError:
            pass
    return True, "opened for writing" + ("" if existed else " (created and removed)")


def resolves(name: str) -> list[str]:
    try:
        return sorted({a[4][0] for a in socket.getaddrinfo(name, None)})
    except OSError:
        return []


def tcp_open(host: str, port: int, timeout: float = 4) -> bool:
    try:
        with socket.create_connection((host, port), timeout=timeout):
            return True
    except OSError:
        return False


def recv_head(sock: socket.socket, limit: int = 65536) -> bytes:
    data = b""
    while b"\r\n\r\n" not in data and len(data) < limit:
        chunk = sock.recv(4096)
        if not chunk:
            break
        data += chunk
    return data


def recv_some(sock: socket.socket, data: bytes, limit: int = 16384) -> bytes:
    try:
        while len(data) < limit:
            chunk = sock.recv(4096)
            if not chunk:
                break
            data += chunk
    except OSError:
        pass
    return data


def status_of(data: bytes) -> int:
    m = re.match(rb"HTTP/\d(?:\.\d)? (\d{3})", data)
    return int(m.group(1)) if m else 0


def tinyproxy_says(data: bytes) -> bool:
    return b"tinyproxy" in data.lower()


def proxy_connect(ctx: Ctx, target: str, timeout: float = 12) -> tuple[int, socket.socket | None, str]:
    """CONNECT through the proxy. Returns (status, open socket on 200, first line)."""
    try:
        s = socket.create_connection(ctx.proxy, timeout=timeout)
    except OSError as e:
        return 0, None, f"proxy unreachable: {e}"
    try:
        s.sendall(f"CONNECT {target} HTTP/1.1\r\nHost: {target}\r\n\r\n".encode())
        head = recv_head(s)
    except OSError as e:
        s.close()
        return 0, None, f"error: {e}"
    code = status_of(head)
    first = head.split(b"\r\n", 1)[0].decode(errors="replace")
    if code == 200:
        return code, s, first
    s.close()
    return code, None, first


def https_via_proxy(ctx: Ctx, host: str, port: int = 443) -> tuple[str, str]:
    """('REACHABLE'|'BLOCKED'|'TLS-ERROR'|'ERROR', evidence)."""
    code, s, first = proxy_connect(ctx, f"{host}:{port}")
    if code == 403:
        return "BLOCKED", first
    if code != 200 or s is None:
        return "ERROR", first or "no answer"
    try:
        tls = ssl.create_default_context().wrap_socket(s, server_hostname=host)
        tls.sendall(f"HEAD / HTTP/1.1\r\nHost: {host}\r\nUser-Agent: sandbox-check\r\nConnection: close\r\n\r\n".encode())
        head = recv_head(tls)
        tls.close()
        return "REACHABLE", f"tunnel 200, site answered {status_of(head) or '?'}"
    except (OSError, ssl.SSLError) as e:
        s.close()
        return "TLS-ERROR", f"tunnel 200, TLS/HTTP failed: {e}"


def proxy_http(ctx: Ctx, request_line: str, headers: dict[str, str] | None = None, timeout: float = 12) -> tuple[int, bytes]:
    """Send a raw HTTP request (absolute-form or anything else) to the proxy."""
    try:
        s = socket.create_connection(ctx.proxy, timeout=timeout)
    except OSError as e:
        return 0, f"proxy unreachable: {e}".encode()
    try:
        hdrs = {"User-Agent": "sandbox-check", "Connection": "close", **(headers or {})}
        req = request_line + "\r\n" + "".join(f"{k}: {v}\r\n" for k, v in hdrs.items()) + "\r\n"
        s.sendall(req.encode())
        data = recv_some(s, recv_head(s))
    except OSError as e:
        return 0, f"error: {e}".encode()
    finally:
        s.close()
    return status_of(data), data


def http_blocked(code: int, data: bytes) -> bool:
    return code == 403 and tinyproxy_says(data) or code == 0


def allowed_by_list(host: str, allow: list[str]) -> bool:
    return any(fnmatch.fnmatchcase(host, p) for p in allow)


def dns_query_udp(server: str, name: str = "example.com", timeout: float = 3) -> bool:
    """True if a DNS server answered a plain UDP query."""
    qname = b"".join(bytes([len(p)]) + p.encode() for p in name.split(".")) + b"\0"
    pkt = b"\x12\x34\x01\x00\x00\x01\x00\x00\x00\x00\x00\x00" + qname + b"\x00\x01\x00\x01"
    try:
        with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as s:
            s.settimeout(timeout)
            s.sendto(pkt, (server, 53))
            data, _ = s.recvfrom(512)
            return data[:2] == b"\x12\x34"
    except OSError:
        return False


# --- setup -----------------------------------------------------------------------------------------------------------

def load_context(args: argparse.Namespace) -> Ctx:
    workdir = Path(os.environ.get("SANDBOX_WORKDIR", "/work"))
    proxy_url = os.environ.get("HTTPS_PROXY") or os.environ.get("https_proxy") or "http://proxy:3128"
    pu = urllib.parse.urlsplit(proxy_url)
    ctx = Ctx(workdir=workdir, home=Path.home(), proxy=(pu.hostname or "proxy", pu.port or 3128), args=args)

    toml_path = workdir / ".sandbox" / "sandbox.toml"
    try:
        ctx.settings = tomllib.loads(toml_path.read_text())
    except (OSError, tomllib.TOMLDecodeError) as e:
        ctx.settings_error = f"{toml_path}: {e}"

    al = workdir / ".sandbox" / "allowlist.txt"
    try:
        ctx.allowlist = [l.split("#", 1)[0].strip() for l in al.read_text().splitlines()]
        ctx.allowlist = [l for l in ctx.allowlist if l]
    except OSError:
        ctx.allowlist = None

    for m in mounts():
        if m["target"] == str(workdir):
            ctx.workspace_mode = "ro" if is_ro(m["opts"]) else "rw"

    ctx.llm_url = os.environ.get("SANDBOX_LLM_URL")
    ctx.llm_model = os.environ.get("SANDBOX_LLM_MODEL")
    return ctx


def setting(ctx: Ctx, table: str, key: str, default=None):
    return ctx.settings.get(table, {}).get(key, default)


def share_targets(ctx: Ctx) -> list[str]:
    out = []
    for sh in ctx.settings.get("share", []) or []:
        path = sh.get("path") or f"/shares/{Path(str(sh.get('host', ''))).name}"
        out.append(path.rstrip("/"))
    return out


# --- 1 read ----------------------------------------------------------------------------------------------------------

def check_read(ctx: Ctx) -> None:
    section("1 Read outside the project and the home volume")
    W = str(ctx.workdir)
    expected_mode = setting(ctx, "project", "workspace", "rw")
    protect_git = setting(ctx, "project", "protect_git", True)
    shares = share_targets(ctx)

    if ctx.settings_error:
        add("read", "1.0", "sandbox.toml readable (expectations)", WARN,
            f"{ctx.settings_error} — checks fall back to defaults; [[share]] mounts cannot be told apart")
    else:
        add("read", "1.0", "sandbox.toml readable (expectations)", INFO,
            f"workspace={expected_mode} protect_git={protect_git} sudo={setting(ctx, 'desktop', 'sudo', True)} "
            f"shares={shares or 'none'}")

    # 1.1 mounts
    allowed_exact = {"/", "/config", W, f"{W}/.sandbox", f"{W}/.git",
                     "/etc/hosts", "/etc/hostname", "/etc/resolv.conf"}
    bad, listing = [], []
    for m in mounts():
        t = m["target"]
        if t in ("/proc", "/sys", "/dev") or t.startswith(("/proc/", "/sys/", "/dev/")):
            continue
        mode = "ro" if is_ro(m["opts"]) else "rw"
        listing.append(f"{t}  {mode}  {m['fstype']}  {m['source']}")
        ok = t in allowed_exact or any(t == s or t.startswith(s + "/") for s in shares)
        if not ok and not ctx.settings_error:
            bad.append(t)
        elif not ok:
            bad.append(t + " (a [[share]]?)")
    add("read", "1.1", "only expected mounts", FAIL if bad and not ctx.settings_error else WARN if bad else PASS,
        ("unexpected: " + ", ".join(bad) + "\n" if bad else "") + "\n".join(listing))

    # 1.2 workspace mode, .sandbox, .git
    if ctx.workspace_mode is None:
        add("read", "1.2", "workspace mount found", FAIL, f"no mount at {W}")
    else:
        add("read", "1.2", f"workspace is mounted {expected_mode} as configured",
            PASS if ctx.workspace_mode == expected_mode else FAIL, f"mount: {ctx.workspace_mode}, sandbox.toml: {expected_mode}")

    # 1.3 host paths that must not exist
    found = []
    for p in ["/var/run/docker.sock", "/run/docker.sock", "/run/containerd/containerd.sock", "/run/podman/podman.sock",
              "/var/lib/docker", "/mnt/c", "/mnt/wsl", "/mnt/host", "/host", "/hostfs", "/rootfs", "/Users"]:
        if os.path.lexists(p):
            found.append(p)
    dh = os.environ.get("DOCKER_HOST")
    add("read", "1.3", "no host paths or container-engine sockets", FAIL if found else PASS,
        ("present: " + ", ".join(found) if found else "none present") + (f"; DOCKER_HOST={dh}" if dh else ""))

    # 1.4 project siblings
    parent = ctx.workdir.parent
    try:
        sib = sorted(x for x in os.listdir(parent) if x != ctx.workdir.name)
        add("read", "1.4", f"no host folders next to the project ({parent})", FAIL if sib else PASS,
            "siblings: " + ", ".join(sib) if sib else "only the project folder")
    except OSError as e:
        add("read", "1.4", "no host folders next to the project", INFO, str(e))

    # 1.5 privileges
    uid = os.getuid()
    status = dict(l.split(":\t", 1) for l in Path("/proc/self/status").read_text().splitlines() if ":\t" in l)
    cap_eff = status.get("CapEff", "?").strip()
    add("read", "1.5", "not root, no effective capabilities", PASS if uid != 0 and int(cap_eff, 16) == 0 else FAIL,
        f"uid={uid} CapEff={cap_eff} CapBnd={status.get('CapBnd', '?').strip()} "
        f"NoNewPrivs={status.get('NoNewPrivs', '?').strip()} Seccomp={status.get('Seccomp', '?').strip()}")
    seccomp = status.get("Seccomp", "?").strip()
    add("read", "1.6", "seccomp filter active", PASS if seccomp == "2" else WARN,
        f"Seccomp={seccomp} (2 = filter; 0 = unconfined, e.g. security_opt seccomp=unconfined)")

    try:
        open("/etc/shadow").read(1)
        add("read", "1.7", "/etc/shadow not readable", FAIL if uid != 0 else INFO, "readable")
    except OSError as e:
        add("read", "1.7", "/etc/shadow not readable", PASS, e.strerror)

    devs, readable = [], []
    for name in sorted(os.listdir("/dev")):
        if re.match(r"^(sd[a-z]+\d*|nvme\d.*|vd[a-z]+\d*|xvd[a-z]+\d*|hd[a-z]+\d*|mem|kmem|port|kmsg|mapper)$", name):
            devs.append(name)
            try:
                with open(f"/dev/{name}", "rb") as f:
                    f.read(1)
                readable.append(name)
            except OSError:
                pass
    configured_devices = setting(ctx, "desktop", "devices", [])
    add("read", "1.8", "no readable disks or kernel memory devices",
        FAIL if readable else WARN if devs else PASS,
        (f"readable: {readable}; " if readable else "") + (f"present: {devs}" if devs else "none present")
        + (f"; configured devices: {configured_devices}" if configured_devices else ""))

    try:
        os.listdir("/proc/1/root")
        add("read", "1.9", "/proc/1/root not listable", WARN if uid != 0 else INFO, "listable (container's own root)")
    except OSError as e:
        add("read", "1.9", "/proc/1/root not listable", PASS, e.strerror)

    # 1.10 setuid programs (information for a human or an exploring agent)
    suid = []
    for d in ["/usr/bin", "/usr/sbin", "/bin", "/sbin", "/usr/local/bin", "/usr/local/sbin", "/usr/lib/openssh",
              "/usr/libexec"]:
        try:
            for e in os.scandir(d):
                try:
                    if e.is_file(follow_symlinks=False) and e.stat(follow_symlinks=False).st_mode & stat.S_ISUID:
                        suid.append(e.path)
                except OSError:
                    pass
        except OSError:
            pass
    add("read", "1.10", "setuid programs", INFO, ", ".join(sorted(set(suid))) or "none")

    # 1.11 sudo matches the setting
    sudo_setting = setting(ctx, "desktop", "sudo", True)
    rc, out = run(["sudo", "-n", "true"])
    works = rc == 0
    if rc == 127:
        add("read", "1.11", "sudo as configured", INFO, "sudo not installed")
    else:
        add("read", "1.11", f"sudo {'enabled' if sudo_setting else 'disabled'} as configured",
            FAIL if works and not sudo_setting else WARN if works else PASS,
            f"sudo -n true: rc={rc} {out[:100]}" + (" — root inside the container is possible" if works else ""))
    if works:
        check_root(ctx)


def check_root(ctx: Ctx) -> None:
    rc, out = run(["sudo", "-n", "mount", "-t", "tmpfs", "none", "/mnt"])
    if rc == 0:
        run(["sudo", "-n", "umount", "/mnt"])
    add("read", "1.12", "root cannot mount", FAIL if rc == 0 else PASS, out[:150] or f"rc={rc}")
    rc, out = run(["sudo", "-n", "sh", "-c", 'v=$(cat /proc/sys/kernel/sysrq); echo "$v" > /proc/sys/kernel/sysrq'])
    add("read", "1.13", "root cannot write /proc/sys (writes the current value back)", FAIL if rc == 0 else PASS,
        out[:150] or f"rc={rc}")
    rc, out = run(["sudo", "-n", "grep", "CapEff", "/proc/self/status"])
    add("read", "1.14", "root capabilities", INFO, out)


# --- 2 write ---------------------------------------------------------------------------------------------------------

def check_write(ctx: Ctx) -> None:
    section("2 Write outside the project and the home volume")
    W = ctx.workdir
    pid = os.getpid()
    mode = ctx.workspace_mode or setting(ctx, "project", "workspace", "rw")
    protect_git = setting(ctx, "project", "protect_git", True)

    for i, (p, what) in enumerate([(ctx.home / f".sandbox-check-{pid}", "home volume"),
                                   (Path(f"/tmp/sandbox-check-{pid}"), "/tmp (container disk, not persistent)")], 1):
        ok, ev = can_open_for_append(p)
        add("write", f"2.{i}", f"{what} writable (control)", PASS if ok else WARN, ev)

    ok, ev = can_open_for_append(W / f".sandbox-check-{pid}")
    if mode == "rw":
        add("write", "2.3", "workspace writable (workspace = rw)", PASS if ok else FAIL, ev)
    else:
        add("write", "2.3", "workspace not writable (workspace = ro)", FAIL if ok else PASS, ev)
        probe_ro_workspace(ctx)

    must_fail = [
        ("2.4", W.parent / f"sandbox-escape-{pid}", "parent of the workspace", FAIL),
        ("2.5", W / ".sandbox" / f"sandbox-check-{pid}", ".sandbox/ (settings, allowlist)", FAIL),
        ("2.6", W / ".sandbox" / "allowlist.txt", ".sandbox/allowlist.txt", FAIL),
        ("2.7", W / ".sandbox" / "sandbox.toml", ".sandbox/sandbox.toml", FAIL),
        ("2.8", Path("/proc/sys/kernel/hostname"), "/proc/sys", FAIL),
        ("2.9", Path(f"/sys/fs/cgroup/sandbox-check-{pid}"), "/sys/fs/cgroup", FAIL),
        ("2.10", Path(f"/etc/sandbox-check-{pid}"), "/etc (container image)", WARN),
        ("2.11", Path(f"/usr/sandbox-check-{pid}"), "/usr (container image)", WARN),
    ]
    if (W / ".git").is_dir():
        must_fail.insert(4, ("2.12", W / ".git" / f"sandbox-check-{pid}", ".git/ (protect_git)",
                             FAIL if protect_git or mode == "ro" else INFO))
    for id_, p, what, severity in must_fail:
        if not p.parent.exists():
            add("write", id_, f"{what} not writable", SKIP, f"{p.parent} does not exist")
            continue
        ok, ev = can_open_for_append(p)
        add("write", id_, f"{what} not writable", severity if ok else PASS, f"{p}: {ev}")


def probe_ro_workspace(ctx: Ctx) -> None:
    W = ctx.workdir
    pid = os.getpid()
    files = [p for p in W.iterdir() if p.is_file() and not p.is_symlink()]
    results = []

    def attempt(label, fn):
        try:
            fn()
            results.append(f"{label}: SUCCEEDED")
            return True
        except OSError as e:
            results.append(f"{label}: {e.strerror}")
            return False

    bad = False
    link = W / f".sandbox-check-link-{pid}"
    if attempt("symlink", lambda: os.symlink("/etc/passwd", link)):
        bad = True
        link.unlink(missing_ok=True)
    d = W / f".sandbox-check-dir-{pid}"
    if attempt("mkdir", lambda: os.mkdir(d)):
        bad = True
        d.rmdir()
    if files:
        f = files[0]
        st = f.stat()
        # Both are no-ops if they were allowed: same times, same mode.
        bad |= attempt(f"utime {f.name}", lambda: os.utime(f, ns=(st.st_atime_ns, st.st_mtime_ns)))
        bad |= attempt(f"chmod {f.name}", lambda: os.chmod(f, stat.S_IMODE(st.st_mode)))
    add("write", "2.3b", "workspace cannot be changed in other ways (ro)", FAIL if bad else PASS, "; ".join(results))


# --- 3 network -------------------------------------------------------------------------------------------------------

def check_network(ctx: Ctx) -> None:
    section("3 Networks and reachable hosts")
    llm_on = bool(ctx.llm_url)

    ifaces = [l.split(":")[0].strip() for l in Path("/proc/net/dev").read_text().splitlines()[2:]]
    addrs = run(["ip", "-br", "addr"])[1] if run(["ip", "-V"])[0] == 0 else ""
    add("net", "3.1", "network interfaces", INFO if len([i for i in ifaces if i != "lo"]) <= 1 else WARN,
        f"interfaces: {', '.join(ifaces)}" + (f"\n{addrs}" if addrs else ""))

    default4 = any(l.split()[1] == "00000000" for l in Path("/proc/net/route").read_text().splitlines()[1:] if l.split())
    try:
        default6 = any(l.split()[0] == "0" * 32 and l.split()[1] == "00" and l.split()[9] != "lo"
                       for l in Path("/proc/net/ipv6_route").read_text().splitlines())
    except OSError:
        default6 = False
    add("net", "3.2", "no default route (no internet route)", FAIL if default4 or default6 else PASS,
        f"IPv4 default: {default4}, IPv6 default: {default6}")

    try:
        resolv = " ".join(l for l in Path("/etc/resolv.conf").read_text().splitlines() if l and not l.startswith("#"))
    except OSError:
        resolv = "?"
    add("net", "3.3", "resolver", INFO, resolv)

    must = ["proxy", "gateway"] + (["llm"] if llm_on else [])
    missing = [n for n in must if not resolves(n)]
    add("net", "3.4", f"sandbox services resolve ({', '.join(must)})", WARN if missing else PASS,
        f"missing: {missing}" if missing else ", ".join(f"{n}={resolves(n)[0]}" for n in must))
    leaked = {n: resolves(n) for n in ["ollama", "github.com", "example.com", "host.docker.internal", "1.1.1.1.nip.io"]}
    leaked = {k: v for k, v in leaked.items() if v}
    add("net", "3.5", "no external DNS, ollama and the host not resolvable", FAIL if leaked else PASS,
        f"resolved: {leaked}" if leaked else "none of ollama, github.com, example.com, host.docker.internal resolve")

    udp = [s for s in ["1.1.1.1", "8.8.8.8", "9.9.9.9"] if dns_query_udp(s)]
    add("net", "3.6", "no direct DNS to public resolvers (UDP 53)", FAIL if udp else PASS,
        f"answered: {udp}" if udp else "1.1.1.1, 8.8.8.8, 9.9.9.9 did not answer")

    targets = [("1.1.1.1", 443), ("1.1.1.1", 80), ("8.8.8.8", 53), ("140.82.112.3", 443), ("140.82.112.3", 22),
               ("2606:4700:4700::1111", 443)]
    open_ = [f"{h}:{p}" for h, p in targets if tcp_open(h, p)]
    add("net", "3.7", "no direct TCP to the internet", FAIL if open_ else PASS,
        f"open: {open_}" if open_ else "all closed: " + ", ".join(f"{h}:{p}" for h, p in targets))

    rc, out = run(["ping", "-c1", "-W2", "1.1.1.1"])
    add("net", "3.8", "no ICMP to the internet", SKIP if rc == 127 else FAIL if rc == 0 else PASS,
        out.splitlines()[-1] if out else f"rc={rc}")

    if not tcp_open(*ctx.proxy):
        add("net", "3.9", f"proxy {ctx.proxy[0]}:{ctx.proxy[1]} reachable", FAIL, "cannot connect — later checks are void")
        return
    if ctx.allowlist is None:
        add("net", "3.9", "allowlist readable", WARN, "cannot read .sandbox/allowlist.txt — results are not compared")
    else:
        add("net", "3.9", "allowlist (active entries)", INFO, ", ".join(ctx.allowlist) or "(empty)")

    hosts = list(dict.fromkeys(HOSTS + [h for h in (ctx.args.hosts or "").split(",") if h]))
    with concurrent.futures.ThreadPoolExecutor(max_workers=8) as ex:
        results = list(ex.map(lambda h: (h, *https_via_proxy(ctx, h)), hosts))
    rows = []
    for i, (h, res, ev) in enumerate(results, 1):
        listed = allowed_by_list(h, ctx.allowlist) if ctx.allowlist is not None else None
        if res == "REACHABLE":
            verdict = PASS if listed else FAIL if listed is False else INFO
            if listed:
                ctx.reachable_allowed.append(h)
        elif res == "BLOCKED":
            verdict = WARN if listed else PASS
        else:
            verdict = WARN if listed else PASS if listed is False else INFO
        note = {True: "allowlisted", False: "not allowlisted", None: "allowlist unknown"}[listed]
        add("net", f"3.10.{i}", f"https://{h}  {res}  ({note})", verdict,
            ev + (" — blocked although allowlisted: proxy not reloaded?" if res == "BLOCKED" and listed else ""))
        rows.append((h, res))
    eco_hosts = {
        "GitHub": ["github.com", "codeload.github.com"], "NuGet": ["api.nuget.org"],
        "crates.io": ["index.crates.io", "static.crates.io"], "Rust toolchains": ["static.rust-lang.org"],
        "npm": ["registry.npmjs.org"], "PyPI": ["pypi.org", "files.pythonhosted.org"], "Go": ["proxy.golang.org"],
        "Maven": ["repo.maven.apache.org"], "Ubuntu apt": ["archive.ubuntu.com"],
        "Docker Hub": ["registry-1.docker.io"], "Hugging Face": ["huggingface.co"],
    }
    res = dict(rows)
    ecos = {k: all(res.get(h) == "REACHABLE" for h in v) for k, v in eco_hosts.items()}
    add("net", "3.11", "ecosystems available", INFO, ", ".join(f"{k}: {'yes' if v else 'no'}" for k, v in ecos.items()))


# --- 4 proxy ---------------------------------------------------------------------------------------------------------

def check_proxy(ctx: Ctx) -> None:
    section("4 Proxy cannot be bypassed or reconfigured")
    if not tcp_open(*ctx.proxy):
        add("proxy", "4.0", "proxy reachable", SKIP, "proxy not reachable")
        return
    A = next((h for h in ctx.reachable_allowed if "*" not in h), None)
    if A:
        add("proxy", "4.0", "allowed host used for the bypass tests", INFO, A)
        for i, port in enumerate([22, 80, 8443, 3128], 1):
            code, s, first = proxy_connect(ctx, f"{A}:{port}")
            if s:
                s.close()
            add("proxy", f"4.1.{i}", f"no CONNECT to port {port} of an allowed host", FAIL if code == 200 else PASS,
                first or "no answer")
        code, data = proxy_http(ctx, f"GET http://{A}/ HTTP/1.1", {"Host": A})
        add("proxy", "4.2", "plain HTTP to an allowed host (port 80)", INFO,
            f"status {code}" + (" from tinyproxy" if tinyproxy_says(data) else ""))
    else:
        add("proxy", "4.0", "allowed host used for the bypass tests", SKIP,
            "no allowlisted host was reachable in 3 — port checks skipped")
        A = "github.com"

    # look-alike names and raw IPs (CONNECT)
    for i, target in enumerate([f"{A}.example.org:443", f"evil{A}:443", f"{A}.:443", "EXAMPLE.COM:443",
                                "example.com.:443", "140.82.112.3:443", "1.1.1.1:443", "[2606:4700:4700::1111]:443",
                                f"{A}:443@example.com:443", f"example.com:443#{A}"], 1):
        code, s, first = proxy_connect(ctx, target)
        if s:
            s.close()
        allowed_lookalike = target.startswith(f"{A}.:") and code == 200   # trailing dot of the allowed host itself
        add("proxy", f"4.3.{i}", f"CONNECT {target} blocked",
            INFO if allowed_lookalike else FAIL if code == 200 else PASS, first or "no answer")

    # request-line / Host header tricks (plain HTTP through the proxy)
    tricks = [
        ("Host header of an allowed host", "GET http://example.com/ HTTP/1.1", {"Host": A}),
        ("origin-form request (transparent mode)", "GET / HTTP/1.1", {"Host": "example.com"}),
        ("path that looks like a host", f"GET http://example.com/@{A}/ HTTP/1.1", {"Host": "example.com"}),
        ("user info that looks like a host", f"GET http://{A}@example.com/ HTTP/1.1", {"Host": "example.com"}),
        ("raw IP", "GET http://1.1.1.1/ HTTP/1.1", {"Host": "1.1.1.1"}),
        ("Docker API on the host", "GET http://host.docker.internal:2375/version HTTP/1.1", {"Host": "host.docker.internal:2375"}),
        ("host web server", "GET http://host.docker.internal/ HTTP/1.1", {"Host": "host.docker.internal"}),
        ("Ollama behind the gateway", "GET http://ollama:11434/api/version HTTP/1.1", {"Host": "ollama:11434"}),
        ("the proxy itself", "GET http://localhost:3128/ HTTP/1.1", {"Host": "localhost:3128"}),
        ("cloud metadata service", "GET http://169.254.169.254/latest/meta-data/ HTTP/1.1", {"Host": "169.254.169.254"}),
        ("tinyproxy statistics page", "GET http://tinyproxy.stats/ HTTP/1.1", {"Host": "tinyproxy.stats"}),
    ]
    for i, (what, line, hdrs) in enumerate(tricks, 1):
        code, data = proxy_http(ctx, line, hdrs)
        blocked = http_blocked(code, data) or code in (400, 501) and tinyproxy_says(data)
        title = re.search(rb"<title>([^<]*)", data)
        ev = f"status {code}" + (f" «{title.group(1).decode(errors='replace')}»" if title else "") + \
             (" (tinyproxy)" if tinyproxy_says(data) else "")
        # The Host-header trick may legitimately reach the allowed host; a page from example.com is the failure.
        if not blocked and what.startswith(("Host header", "path that", "user info")) and b"Example Domain" not in data:
            verdict = WARN
        elif what == "tinyproxy statistics page":
            verdict = PASS if blocked else WARN
        else:
            verdict = PASS if blocked else FAIL
        add("proxy", f"4.4.{i}", f"{what} blocked", verdict, ev + f"  :: {line}")

    # open ports on the sandbox services
    for i, (host, expected) in enumerate([(ctx.proxy[0], {ctx.proxy[1]}), ("gateway", {3001}),
                                          ("llm", {11434}) if ctx.llm_url else (None, None)], 1):
        if not host:
            continue
        ports = [21, 22, 25, 53, 80, 443, 2375, 2376, 3000, 3001, 3128, 5000, 6379, 8000, 8080, 8443, 8888, 9000,
                 9090, 11434]
        with concurrent.futures.ThreadPoolExecutor(max_workers=10) as ex:
            opened = {p for p, ok in zip(ports, ex.map(lambda p: tcp_open(host, p, 2), ports)) if ok}
        extra = opened - expected
        add("proxy", f"4.5.{i}", f"{host}: only port(s) {sorted(expected)} open", FAIL if extra else PASS,
            f"open: {sorted(opened)}")

    procs = []
    for p in Path("/proc").iterdir():
        if p.name.isdigit():
            try:
                if (p / "comm").read_text().strip() in ("tinyproxy", "ollama", "dockerd", "containerd"):
                    procs.append((p / "comm").read_text().strip())
            except OSError:
                pass
    # (nginx is not looked for: the desktop image serves its web UI with its own nginx)
    add("proxy", "4.6", "proxy and engine not running in this container (cannot be signalled)",
        FAIL if procs else PASS, f"found: {sorted(set(procs))}" if procs else "none of tinyproxy, ollama, dockerd, containerd here")
    add("proxy", "4.7", "allowlist cannot be edited from here", INFO, "see 2.5 and 2.6")


# --- 5 llm -----------------------------------------------------------------------------------------------------------

def llm_request(ctx: Ctx, method: str, path: str, body: object | None = None, timeout: float = 30) -> tuple[int, str]:
    u = urllib.parse.urlsplit(ctx.llm_url)
    conn = http.client.HTTPConnection(u.hostname, u.port or 80, timeout=timeout)
    try:
        data = body if isinstance(body, (bytes, type(None))) else json.dumps(body).encode()
        conn.putrequest(method, path, skip_accept_encoding=True)   # path is sent exactly as given
        conn.putheader("Content-Type", "application/json")
        conn.putheader("Content-Length", str(len(data or b"")))
        conn.endheaders(data)
        r = conn.getresponse()
        return r.status, r.read(4096).decode(errors="replace")
    except OSError as e:
        return 0, f"error: {e}"
    finally:
        conn.close()


def check_llm(ctx: Ctx) -> None:
    section("5 Local LLM gateway")
    if ctx.args.no_llm:
        add("llm", "5.0", "LLM gateway", SKIP, "--no-llm")
        return
    if not ctx.llm_url:
        add("llm", "5.0", "LLM gateway", SKIP, "SANDBOX_LLM_URL not set: the local LLM is off")
        return
    M, X = ctx.llm_model, FAKE_MODEL

    code, body = llm_request(ctx, "GET", "/api/tags")
    before = sorted(m.get("name") for m in json.loads(body).get("models", [])) if code == 200 else None

    allowed = [
        ("GET", "/api/version", None), ("GET", "/api/tags", None), ("GET", "/api/ps", None), ("GET", "/v1/models", None),
        ("POST", "/api/show", {"model": M}),
    ]
    if not ctx.args.no_inference:
        allowed += [
            ("POST", "/api/generate", {"model": M, "prompt": "Say OK", "stream": False, "options": {"num_predict": 5}}),
            ("POST", "/v1/chat/completions", {"model": M, "max_tokens": 5,
                                              "messages": [{"role": "user", "content": "Say OK"}]}),
            ("POST", "/v1/messages", {"model": M, "max_tokens": 5, "messages": [{"role": "user", "content": "Say OK"}]}),
        ]
    for i, (method, path, body) in enumerate(allowed, 1):
        code, text = llm_request(ctx, method, path, body, timeout=600)
        add("llm", f"5.1.{i}", f"{method} {path} allowed", PASS if code == 200 else WARN, f"{code} {text[:140]}")

    code, text = llm_request(ctx, "POST", "/api/generate", {"model": X, "prompt": "hi", "stream": False}, timeout=60)
    add("llm", "5.2", "unknown model is not downloaded by /api/generate",
        PASS if code == 404 and "not found" in text else FAIL if "pull" in text or "status" in text else WARN,
        f"{code} {text[:140]}")

    blocked = [
        ("POST", "/api/pull", {"model": X, "stream": False}),
        ("POST", "/api/push", {"model": X, "stream": False}),
        ("POST", "/api/create", {"model": X, "from": X, "stream": False}),
        ("POST", "/api/copy", {"source": X, "destination": X + "-copy"}),
        ("DELETE", "/api/delete", {"model": X}),
        ("HEAD", ZERO_BLOB, None),
        ("POST", ZERO_BLOB, b"x"),
        # path tricks
        ("POST", "/api/pull/", {"model": X, "stream": False}),
        ("POST", "//api/pull", {"model": X, "stream": False}),
        ("POST", "/API/PULL", {"model": X, "stream": False}),
        ("POST", "/api/./pull", {"model": X, "stream": False}),
        ("POST", "/api/chat/../pull", {"model": X, "stream": False}),
        ("POST", "/api/%70ull", {"model": X, "stream": False}),
        ("POST", "/api%2Fpull", {"model": X, "stream": False}),
        ("POST", "/v1/models/x/..%2F..%2F..%2Fapi%2Fpull", {"model": X, "stream": False}),
        ("POST", "/v1/models/%2e%2e/%2e%2e/api/pull", {"model": X, "stream": False}),
        ("POST", "/v1/models/x/../../api/delete", {"model": X}),
        ("POST", "/api/pull;.json", {"model": X, "stream": False}),
        ("POST", "/api/pull%00", {"model": X, "stream": False}),
        ("POST", "http://ollama:11434/api/pull", {"model": X, "stream": False}),
    ]
    for i, (method, path, body) in enumerate(blocked, 1):
        code, text = llm_request(ctx, method, path, body)
        if code == 403 and ("blocked by the sandbox" in text or method == "HEAD"):
            v = PASS
        elif code == 400 and "<html" in text.lower():
            v = PASS   # nginx refused the request itself: it never reached Ollama
        elif code == 400 or (code in (404, 405) and ("page not found" in text or "method not allowed" in text)):
            v = WARN   # reached Ollama (or nginx rejected it) without doing anything
        else:
            v = FAIL
        add("llm", f"5.3.{i}", f"{method} {path} blocked", v, f"{code} {text[:140]}")

    # read-only routes with a write method / smuggled path in the query
    for i, (method, path, body) in enumerate([("DELETE", "/api/tags", {"model": X}),
                                              ("POST", "/api/tags?/api/pull", {"model": X}),
                                              ("PUT", "/api/version", None)], 1):
        code, text = llm_request(ctx, method, path, body)
        add("llm", f"5.4.{i}", f"{method} {path} does nothing", PASS if code in (403, 404, 405) else WARN,
            f"{code} {text[:140]}")

    direct = resolves("ollama")
    add("llm", "5.5", "Ollama itself not reachable (only through the gateway)",
        FAIL if direct and tcp_open("ollama", 11434) else PASS, f"ollama resolves to {direct}" if direct else "does not resolve")

    code, body = llm_request(ctx, "GET", "/api/tags")
    after = sorted(m.get("name") for m in json.loads(body).get("models", [])) if code == 200 else None
    add("llm", "5.6", "installed models unchanged", PASS if before == after and before is not None else
        FAIL if before is not None and after is not None else WARN, f"before={before} after={after}")


# --- report ----------------------------------------------------------------------------------------------------------

def write_reports(ctx: Ctx, out: Path) -> tuple[Path, Path]:
    out.mkdir(parents=True, exist_ok=True)
    counts = {v: sum(r.verdict == v for r in RESULTS) for v in (PASS, FAIL, WARN, INFO, SKIP)}
    meta = {
        "date": datetime.datetime.now().astimezone().isoformat(timespec="seconds"),
        "workspace": str(ctx.workdir), "workspace_mode": ctx.workspace_mode, "uid": os.getuid(),
        "proxy": f"{ctx.proxy[0]}:{ctx.proxy[1]}", "llm": ctx.llm_url, "llm_model": ctx.llm_model,
        "counts": counts,
    }
    jp = out / "report.json"
    jp.write_text(json.dumps({"meta": meta, "results": [asdict(r) for r in RESULTS]}, indent=2))

    titles = {"read": "1 Read outside project/home", "write": "2 Write outside project/home", "net": "3 Network",
              "proxy": "4 Proxy", "llm": "5 Local LLM gateway"}
    md = [f"# Sandbox boundary report",
          "",
          f"{meta['date']} · workspace `{ctx.workdir}` ({ctx.workspace_mode or "mount not found"}) · uid {meta['uid']} · "
          f"LLM: {ctx.llm_model or 'off'}",
          "",
          "## Summary",
          "",
          " · ".join(f"{k} {v}" for k, v in counts.items()),
          ""]
    for r in RESULTS:
        if r.verdict in (FAIL, WARN):
            md.append(f"- **{r.verdict}** {r.id} {r.name} — {r.evidence.splitlines()[0] if r.evidence else ''}")
    for sec, title in titles.items():
        rows = [r for r in RESULTS if r.section == sec]
        if not rows:
            continue
        md += ["", f"## {title}", "", "| Check | Verdict | Evidence |", "|---|---|---|"]
        for r in rows:
            ev = r.evidence.replace("|", "\\|").replace("\n", "<br>")
            md.append(f"| {r.id} {r.name.replace('|', '/')} | {r.verdict} | {ev} |")
    mp = out / "report.md"
    mp.write_text("\n".join(md) + "\n")
    return jp, mp


def main() -> int:
    global VERBOSE
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0], formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--out", type=Path, default=Path.home() / "sandbox-check", help="report folder (default ~/sandbox-check)")
    ap.add_argument("--hosts", help="extra hosts to try through the proxy, comma-separated")
    ap.add_argument("--no-llm", action="store_true", help="skip the LLM gateway checks")
    ap.add_argument("--no-inference", action="store_true", help="LLM checks without generating text (GPU busy)")
    ap.add_argument("-v", "--verbose", action="store_true", help="print the evidence of every check")
    args = ap.parse_args()
    VERBOSE = args.verbose

    ctx = load_context(args)
    print(f"sandbox-check · workspace {ctx.workdir} · proxy {ctx.proxy[0]}:{ctx.proxy[1]} · "
          f"LLM {ctx.llm_url or 'off'}")
    for fn in (check_read, check_write, check_network, check_proxy, check_llm):
        try:
            fn(ctx)
        except Exception as e:   # one broken section must not hide the others
            add(fn.__name__.removeprefix("check_"), "ERR", f"{fn.__name__} crashed", WARN, f"{type(e).__name__}: {e}")
    jp, mp = write_reports(ctx, args.out)
    counts = {v: sum(r.verdict == v for r in RESULTS) for v in (PASS, FAIL, WARN, INFO, SKIP)}
    print("\n" + " · ".join(f"{k} {v}" for k, v in counts.items()))
    print(f"Report: {mp}\n        {jp}")
    return 1 if counts[FAIL] else 0


if __name__ == "__main__":
    sys.exit(main())
