# Tiny egress image: tinyproxy (allowlist proxy) + socat (UI port gateway).
# Used by the "proxy" and "gateway" services.
FROM alpine:3.22

RUN apk add --no-cache tinyproxy socat

# Run unprivileged; nothing here needs root.
USER nobody

EXPOSE 3128
# The proxy config is part of the image (engine-owned, static). Only allowlist.txt comes from the read-only
# /sandbox mount (= .sandbox/), so "sandbox reload" can pick up changes without a rebuild.
COPY --chmod=0644 tinyproxy.conf /etc/tinyproxy/tinyproxy.conf
CMD ["tinyproxy", "-d", "-c", "/etc/tinyproxy/tinyproxy.conf"]
