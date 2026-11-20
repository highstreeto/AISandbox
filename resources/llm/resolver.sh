#!/bin/sh
# Picks the DNS server nginx asks for "ollama" (included by nginx.conf). Run by the image's entrypoint before nginx.
# Engine file: it is part of the app and baked into the llm image.
#
# nginx ignores /etc/resolv.conf and needs the server spelled out. Docker answers on 127.0.0.11; Podman (aardvark-dns)
# on the gateway of each network the container is on, and not every one of them knows ollama. So: the nameservers of
# resolv.conf that resolve ollama, else all of them (ollama may not be up yet after a host restart).
set -eu

out=/tmp/nginx/resolver.conf
mkdir -p /tmp/nginx

found=""
all=""
for ns in $(awk '$1 == "nameserver" { print $2 }' /etc/resolv.conf); do
    case "$ns" in *:*) addr="[$ns]" ;; *) addr="$ns" ;; esac
    all="$all $addr"
    if nslookup ollama "$ns" >/dev/null 2>&1; then
        found="$found $addr"
    fi
done
servers="${found:-${all:- 127.0.0.11}}"

echo "resolver$servers valid=10s ipv6=off;   # ollama may get a new address when it restarts" > "$out"
echo "$0: resolver$servers"
