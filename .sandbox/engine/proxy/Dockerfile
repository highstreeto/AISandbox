# Tiny egress image: tinyproxy (allowlist proxy) + socat (UI port gateway).
# Used by the "proxy" and "gateway" services.
FROM alpine:3.22

RUN apk add --no-cache tinyproxy socat

# Run unprivileged; nothing here needs root.
USER nobody

EXPOSE 3128
# Config and allowlist come from the read-only /sandbox mount (= .sandbox/).
CMD ["tinyproxy", "-d", "-c", "/sandbox/engine/proxy/tinyproxy.conf"]
