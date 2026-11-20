# Tiny egress image: tinyproxy (allowlist proxy) + socat (UI port gateway).
# Used by the "proxy" and "gateway" services.
FROM alpine:3.22

RUN apk add --no-cache tinyproxy socat

EXPOSE 3128
# The proxy config is part of the image (engine-owned, static). Only allowlist.txt comes from the read-only
# /sandbox mount (= .sandbox/), so "sandbox reload" can pick up changes without a rebuild.
COPY --chmod=0644 tinyproxy.conf /etc/tinyproxy/tinyproxy.conf
# A nested sandbox (started inside a sandbox) has no internet route: its proxy forwards to the outer proxy, which
# the compose file maps to the name sandbox-upstream. Both allowlists apply.
RUN { cat /etc/tinyproxy/tinyproxy.conf; echo; echo 'Upstream http sandbox-upstream:3128'; } \
      > /etc/tinyproxy/tinyproxy-nested.conf \
 && chmod 0644 /etc/tinyproxy/tinyproxy-nested.conf

# Run unprivileged; nothing here needs root.
USER nobody
CMD ["tinyproxy", "-d", "-c", "/etc/tinyproxy/tinyproxy.conf"]
