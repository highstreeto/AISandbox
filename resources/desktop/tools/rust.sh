#!/usr/bin/env bash
# ABOUT: Rust via rustup + rustfmt, clippy, rust-src, rust-analyzer, build-essential. Option: toolchain(s), e.g. stable nightly 1.88 (default stable; first = default)
# HOSTS: sh.rustup.rs static.rust-lang.org (install, rustup) index.crates.io static.crates.io (crates)
source "$(dirname "$0")/_lib.sh"
[ $# -gt 0 ] || set -- stable

export RUSTUP_HOME=/usr/local/rustup
export CARGO_HOME=/usr/local/cargo
export PATH="$CARGO_HOME/bin:$PATH"   # rustup itself, before the /usr/local/bin symlinks exist
# An apt rustc/cargo may already be on PATH; the rustup proxies in /usr/local/bin win anyway.
export RUSTUP_INIT_SKIP_PATH_CHECK=yes
# Runs as root, but HOME is the desktop user's /config (image ENV, or kept by sudo -E); rustup warns about that.
export HOME=/root
COMPONENTS=(rustfmt clippy rust-src rust-analyzer)

# The linker (cc) and the usual native dependencies.
apt_install build-essential pkg-config libssl-dev gdb

# Re-runnable: when rustup is already there (runtime `sandbox-install rust nightly`),
# only add toolchains and keep the existing default.
first=$1
if [ ! -x "$CARGO_HOME/bin/rustup" ]; then
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
    | sh -s -- -y --no-modify-path --profile minimal --default-toolchain "$first"
  shift
fi
for tc in "$@"; do
  rustup toolchain install "$tc" --profile minimal
done

# Every requested toolchain gets the components.
for tc in "$first" "$@"; do
  rustup component add "${COMPONENTS[@]}" --toolchain "$tc"
done

# rustup proxies dispatch on argv[0], so symlinks in /usr/local/bin work (no PATH change needed).
for f in "$CARGO_HOME"/bin/*; do
  ln -sf "$f" "/usr/local/bin/$(basename "$f")"
done

# The toolchains are shared; the desktop user may add components/toolchains without sudo.
chmod -R a+rwX "$RUSTUP_HOME"
rm -rf "$CARGO_HOME/registry" "$CARGO_HOME/git"

# ~/.cargo (= CARGO_HOME at runtime, in the persistent home volume) for `cargo install`ed tools.
cat > /etc/profile.d/sandbox-rust.sh <<'PROFILE'
case ":$PATH:" in
  *":$HOME/.cargo/bin:"*) ;;
  *) export PATH="$HOME/.cargo/bin:$PATH" ;;
esac
PROFILE
# Konsole starts non-login shells: source it from bash.bashrc too.
grep -qF sandbox-rust.sh /etc/bash.bashrc 2>/dev/null \
  || echo '[ -r /etc/profile.d/sandbox-rust.sh ] && . /etc/profile.d/sandbox-rust.sh' >> /etc/bash.bashrc

rustc --version; cargo --version; cargo fmt --version; cargo clippy --version
