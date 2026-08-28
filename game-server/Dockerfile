# Single-stage: build + run in rust:1.93-slim so we avoid a second apt-get
# in a debian:bookworm-slim runtime (its apt GPG keyring is broken on the
# Dokploy build host). Image is larger but the builder apt is cached/working.
# Needs libzmq (omq-zeromq -> libzmq-sys) at build AND runtime.
#
# Runtime extras for execution judges (arena_core::sandbox): `bubblewrap`
# provides the `bwrap` sandbox that isolates untrusted player code, and
# `nodejs`/`python3` are interpreters golf solutions declare via `run:`
# (node/python/sh) — without them the sandbox would falsely fail a correct
# node/python solution with "command not found".
FROM rust:1.93-slim
WORKDIR /app
RUN rm -rf /var/lib/apt/lists/* && for i in 1 2 3; do \
        apt-get update && apt-get install -y --no-install-recommends \
            build-essential pkg-config libsqlite3-dev libzmq3-dev cmake ca-certificates curl git \
            bubblewrap nodejs npm python3 ffmpeg \
        && break || sleep 5; \
    done && rm -rf /var/lib/apt/lists/*

# jscpd powers both the `analysis` probes (code-quality evidence) and the
# cross-session copy/paste validation at session finish. Without it both
# degrade to `unavailable` — silently, so install it explicitly.
RUN npm install -g jscpd@4 && jscpd --version

COPY Cargo.toml Cargo.lock ./
COPY arena-core/ arena-core/
COPY agent-tokens/ agent-tokens/
COPY email-templates/ email-templates/
COPY server/ server/
COPY game-server/ game-server/
COPY ololo/ ololo/

RUN cargo build --release -p game-server

RUN mkdir -p /data
ENV GAME_SERVER_PORT=8081
EXPOSE 8081
VOLUME ["/data"]

HEALTHCHECK --interval=30s --timeout=5s --start-period=20s --retries=3 \
    CMD curl -f http://localhost:8081/health || exit 1

ENTRYPOINT ["/app/target/release/game-server"]