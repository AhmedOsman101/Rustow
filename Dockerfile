# Dockerfile — Integration test environment for rustow
#
# Build:   mask e2e
# Run:     podman run --rm -v "$PWD:/app" -w /app rustow-e2e bash tests/e2e.sh
#
# Void Linux base = small image, same family as other tooling on this machine.

FROM ghcr.io/void-linux/void-glibc-full

# Sync repodata and update xbps first — Void images sometimes ship a stale xbps
RUN xbps-install -Syu xbps && xbps-install -Syu

# Stable base tooling
RUN xbps-install -Sy bash git rust cargo pkg-config shadow && \
    xbps-install -Sy libgit2-devel curl

# Test user (mirrors the pass-gui pattern)
ARG USER_UID=1000
ARG USER_GID=1000
RUN groupadd -g "${USER_GID}" testuser && \
    useradd -u "${USER_UID}" -g testuser -m -d /home/testuser -s /bin/bash testuser && \
    mkdir -p /app && \
    chown -R testuser:testuser /home/testuser /app

USER testuser
ENV CARGO_HOME=/home/testuser/.cargo
WORKDIR /app

CMD ["bash"]
