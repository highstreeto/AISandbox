# The local LLM gateway: nginx (with njs) in front of Ollama. It lets the desktop reach only inference and read-only
# endpoints, and writes one JSON line per request (model, token counts, duration) to /var/log/llm/usage.jsonl.
# The "alpine" variant (not "alpine-slim") ships the njs module.
FROM nginx/nginx-unprivileged:1.31.8-alpine

# The named volume mounted here inherits this owner (uid 101 = the nginx user of the image).
USER root
RUN mkdir -p /var/log/llm && chown 101:101 /var/log/llm
USER 101

COPY --chmod=0644 nginx.conf /etc/nginx/nginx.conf
COPY --chmod=0644 usage.js /etc/nginx/usage.js

EXPOSE 11434
CMD ["nginx", "-g", "daemon off;"]
