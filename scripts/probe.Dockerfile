# probe.Dockerfile — prebuilt image for the GEV motion / layer probes.
#
# Why this image exists (2026-10-23, "36 GB incident"):
#   The probe wrappers used to run
#       npx --yes playwright install chromium --with-deps
#   inside a throwaway `node:22-trixie` container on EVERY invocation.
#   The container was removed after each run, so the apt packages +
#   chromium + headless-shell + ffmpeg (~0.6-1.2 GB) were re-downloaded
#   every single time. One sp8 run with SP8_MOTION_PROBE=1 (1 motion probe
#   + 3 layer probes) pulled ~4 GB off the WAN, and the same wrappers
#   mounted the whole repo read-write at /ws — so the container (which
#   executes third-party npm code) could read and rewrite core/secrets.env.
#
#   This image bakes the browser + OS deps + npm packages ONCE. Probe runs
#   afterwards are network-free and only ever see the single .mjs file,
#   mounted read-only.
#
# Build:  bash scripts/build-probe-image.sh    (idempotent, prints the tag)
#
# The playwright version MUST match the browsers baked into the image.
# build-probe-image.sh derives it from console/package-lock.json, so the
# npm packages and the browser build can never drift apart.
FROM node:22-trixie

ARG PLAYWRIGHT_VERSION=1.63.0
ARG PNGJS_VERSION=7.0.0

# Browsers are baked into /ms-playwright (read-only at run time).
ENV PLAYWRIGHT_BROWSERS_PATH=/ms-playwright
ENV npm_config_update_notifier=false
ENV npm_config_fund=false
ENV npm_config_audit=false

# 1) OS libs (--with-deps) + chromium — the expensive part, done once.
RUN npx --yes playwright@${PLAYWRIGHT_VERSION} install --with-deps chromium \
 && npm cache clean --force \
 && rm -rf /var/lib/apt/lists/* /root/.npm/_npx

# 2) The npm packages the probe scripts import, pinned to the same
#    playwright version as the browsers above. The browser download is
#    skipped here — step 1 already placed them in /ms-playwright.
RUN mkdir -p /opt/probe && cd /opt/probe \
 && npm init -y >/dev/null \
 && PLAYWRIGHT_SKIP_BROWSER_DOWNLOAD=1 npm install --no-audit --no-fund \
      playwright@${PLAYWRIGHT_VERSION} pngjs@${PNGJS_VERSION} \
 && npm cache clean --force

WORKDIR /opt/probe
