# AI sandbox desktop: LinuxServer.io WebTop (Ubuntu + KDE Plasma).
# Tools are selected per project under [tools] in sandbox.toml (claude_code → INSTALL_CLAUDE_CODE);
# each one maps to tools/<name>.sh via the sandbox-install dispatcher. A new tool needs a
# tools/<name>.sh plus an ARG/RUN pair below; the app reads the ARG INSTALL_* lines as the known tools.
ARG WEBTOP_TAG=ubuntu-kde
FROM lscr.io/linuxserver/webtop:${WEBTOP_TAG}

ENV DEBIAN_FRONTEND=noninteractive \
    DOTNET_ROOT=/usr/share/dotnet \
    DOTNET_CLI_TELEMETRY_OPTOUT=1 \
    DOTNET_NOLOGO=1 \
    RUSTUP_HOME=/usr/local/rustup

# Strip what the base image ships but a sandbox doesn't use: Docker (no daemon here), the apt Rust
# toolchain and the C/C++ compilers (rust = true brings rustup and build-essential; otherwise add
# "build-essential" under [tools] apt). Python stays: Selkies and Plasma depend on it.
# xz-utils and bzip2 would go with the compilers; tarballs still need them.
RUN apt-get purge -y --autoremove \
      docker-ce docker-ce-cli containerd.io docker-buildx-plugin docker-compose-plugin fuse-overlayfs \
      cargo rustc gcc g++ cmake \
 && rm -f /etc/apt/sources.list.d/docker.list /usr/local/bin/dind /usr/local/bin/dockerd-entrypoint.sh \
 && rm -rf /etc/s6-overlay/s6-rc.d/svc-docker /etc/s6-overlay/s6-rc.d/user/contents.d/svc-docker \
 && (groupdel docker 2>/dev/null || true)

RUN apt-get update \
 && apt-get install -y --no-install-recommends \
      ca-certificates curl wget gnupg git ripgrep jq unzip less nano procps \
      iputils-ping dnsutils xz-utils bzip2 \
 && rm -rf /var/lib/apt/lists/*

COPY bin/sandbox-install bin/sandbox-llm /usr/local/bin/
COPY tools/ /usr/local/lib/sandbox/tools/
RUN sed -i 's/\r$//' /usr/local/bin/sandbox-install /usr/local/bin/sandbox-llm /usr/local/lib/sandbox/tools/*.sh \
 && chmod 0755 /usr/local/bin/sandbox-install /usr/local/bin/sandbox-llm /usr/local/lib/sandbox/tools/*.sh

# "sandbox update" bumps this to fetch the newest versions of everything below.
ARG CACHE_BUST=0

# One layer per tool: changing one INSTALL_* only rebuilds from that line on.
ARG INSTALL_APT=""
RUN sandbox-install apt "${INSTALL_APT}"
ARG INSTALL_CLAUDE=true
RUN sandbox-install claude "${INSTALL_CLAUDE}"
ARG INSTALL_CLAUDE_CODE=false
RUN sandbox-install claude-code "${INSTALL_CLAUDE_CODE}"
ARG INSTALL_DOTNET=false
RUN sandbox-install dotnet "${INSTALL_DOTNET}"
ARG INSTALL_NODE=false
RUN sandbox-install node "${INSTALL_NODE}"
ARG INSTALL_BUN=false
RUN sandbox-install bun "${INSTALL_BUN}"
ARG INSTALL_RUST=false
RUN sandbox-install rust "${INSTALL_RUST}"
ARG INSTALL_VSCODE=false
RUN sandbox-install vscode "${INSTALL_VSCODE}"
ARG INSTALL_JETBRAINS=false
RUN sandbox-install jetbrains "${INSTALL_JETBRAINS}"

# Sandbox integration: proxy settings for apt/sudo/KDE, startup script.
COPY root/ /
RUN sed -i 's/\r$//' /custom-cont-init.d/* /etc/sudoers.d/sandbox-proxy \
 && chmod 0440 /etc/sudoers.d/sandbox-proxy \
 && chmod 0755 /custom-cont-init.d/*
