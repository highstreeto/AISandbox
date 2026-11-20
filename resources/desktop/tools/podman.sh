#!/usr/bin/env bash
# ABOUT: Rootless Podman + docker-compose: containers and nested sandboxes (host: rootless Podman only). No options
# HOSTS: registry-1.docker.io auth.docker.io production.cloudfront.docker.com ghcr.io lscr.io pkg-containers.githubusercontent.com (images)
source "$(dirname "$0")/_lib.sh"

# docker-compose-v2 is what "podman compose" hands the compose file to (through the API socket below);
# podman-compose lacks flags the sandbox app uses.
# nftables: netavark sets up the networks of compose projects with nft (only a Recommends, which apt_install skips).
apt_install podman docker-compose-v2 crun passt netavark aardvark-dns uidmap fuse-overlayfs catatonit libcap2-bin nftables

# newuidmap/newgidmap with file capabilities instead of setuid root: the setuid build stays euid 0, and writing the
# uid_map of a namespace owned by the user then needs CAP_SYS_ADMIN, which the desktop doesn't have.
chmod 0755 /usr/bin/newuidmap /usr/bin/newgidmap
setcap cap_setuid=ep /usr/bin/newuidmap
setcap cap_setgid=ep /usr/bin/newgidmap

# Defaults for Podman inside the desktop container.
install -d /etc/containers/containers.conf.d
cat > /etc/containers/containers.conf.d/50-sandbox.conf <<'CONF'
# Podman inside the sandbox desktop (written by sandbox-install podman).
[containers]
# net.ipv4.ping_group_range can't be set in a nested network namespace.
default_sysctls = []
# The outer seccomp profile allows sethostname only with CAP_SYS_ADMIN: share the desktop's UTS namespace.
utsns = "host"

[engine]
# No systemd in the desktop (s6): cgroupfs, file events; healthchecks run by svc-podman-health.
cgroup_manager = "cgroupfs"
events_logger = "file"
compose_providers = ["/usr/libexec/docker/cli-plugins/docker-compose"]
compose_warning_logs = false
CONF

# s6 services (as abc): the API socket for docker-compose, and the healthcheck runner Podman would leave to systemd.
svc() {
  local name=$1 run=$2 d=/etc/s6-overlay/s6-rc.d/$1
  install -d "$d/dependencies.d"
  echo longrun > "$d/type"
  : > "$d/dependencies.d/init-services"
  printf '%s\n' "$run" > "$d/run"
  chmod 0755 "$d/run"
  : > "/etc/s6-overlay/s6-rc.d/user/contents.d/$name"
}
svc svc-podman-api '#!/usr/bin/with-contenv bash
# Podman API socket for docker-compose ("podman compose" finds it in $XDG_RUNTIME_DIR/podman).
install -d -m 0700 -o abc -g abc "$XDG_RUNTIME_DIR/podman"
exec s6-setuidgid abc podman system service --time=0 "unix://$XDG_RUNTIME_DIR/podman/podman.sock"'
svc svc-podman-health '#!/usr/bin/with-contenv bash
exec s6-setuidgid abc /usr/local/bin/sandbox-podman-health'

cat > /usr/local/bin/sandbox-podman-health <<'RUNNER'
#!/usr/bin/env bash
# Runs the healthchecks of the desktop user's containers: Podman schedules them with systemd timers, and the
# desktop has no systemd (without this, "compose up --wait" never sees a healthy container).
while true; do
  podman ps --format '{{.ID}} {{.Status}}' 2>/dev/null | awk '/\((healthy|unhealthy|starting)\)/ {print $1}' |
    while read -r id; do podman healthcheck run "$id" </dev/null >/dev/null 2>&1; done   # </dev/null: it reads stdin
  sleep 3
done
RUNNER
chmod 0755 /usr/local/bin/sandbox-podman-health

podman --version; docker compose version 2>/dev/null || /usr/libexec/docker/cli-plugins/docker-compose version
getcap /usr/bin/newuidmap /usr/bin/newgidmap
