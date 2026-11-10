#!/usr/bin/env bash
# ABOUT: Visual Studio Code (Microsoft apt repo)
# HOSTS: packages.microsoft.com (install) marketplace.visualstudio.com *.gallerycdn.vsassets.io *.vscode-cdn.net update.code.visualstudio.com (extensions)
source "$(dirname "$0")/_lib.sh"

add_apt_repo vscode https://packages.microsoft.com/keys/microsoft.asc \
  "deb [arch=amd64,arm64 signed-by=KEYRING] https://packages.microsoft.com/repos/code stable main"
apt_install code

# Electron sandbox off (see _lib.sh); VS Code picks up the proxy from the env.
# DONT_PROMPT_WSL_INSTALL: Docker Desktop on Windows runs containers on the WSL2
# kernel, so the code launcher wrongly thinks it's inside WSL and prompts.
cat > /usr/local/bin/code <<'EOF'
#!/bin/sh
export DONT_PROMPT_WSL_INSTALL=1
exec /usr/bin/code --no-sandbox "$@"
EOF
chmod 0755 /usr/local/bin/code
for f in /usr/share/applications/code*.desktop; do
  if [ -f "$f" ]; then sed -i 's#^Exec=/usr/share/code/code #Exec=/usr/share/code/code --no-sandbox #' "$f"; fi
done
shortcut vscode "VS Code" "/usr/local/bin/code @WORKDIR@" vscode
