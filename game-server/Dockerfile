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
#
# jscpd 5 is a Rust rewrite published as prebuilt platform binaries (the npm
# `jscpd` package is only a launcher around them). Installing the platform
# tarball directly pins the exact bytes by digest, skips npm's optional-
# dependency resolution, and leaves a plain executable on the sandbox PATH
# (`/usr/local/bin` is ro-bound into bwrap). Bump the version and both
# digests together; `TARGETARCH` comes from BuildKit (amd64 when unset).
ARG TARGETARCH
ARG JSCPD_VERSION=5.2.1
ARG JSCPD_SHA256_AMD64=4c2449bb881e04183da00312917a129f4e0c931d2daa0bebcf22cb8ebd6308f1
ARG JSCPD_SHA256_ARM64=3f41a3a08f1bab8bebbfdd91272d28609c903b408b2f10093ef2bee260e14f97
RUN set -eu; \
    case "${TARGETARCH:-amd64}" in \
        amd64) pkg=jscpd-linux-x64-gnu;   sha="$JSCPD_SHA256_AMD64" ;; \
        arm64) pkg=jscpd-linux-arm64-gnu; sha="$JSCPD_SHA256_ARM64" ;; \
        *) echo "unsupported TARGETARCH: ${TARGETARCH}" >&2; exit 1 ;; \
    esac; \
    curl -fsSL -o /tmp/jscpd.tgz "https://registry.npmjs.org/${pkg}/-/${pkg}-${JSCPD_VERSION}.tgz"; \
    echo "${sha}  /tmp/jscpd.tgz" | sha256sum -c -; \
    tar -xzf /tmp/jscpd.tgz -C /tmp package/bin/jscpd; \
    install -m 0755 /tmp/package/bin/jscpd /usr/local/bin/jscpd; \
    rm -rf /tmp/jscpd.tgz /tmp/package; \
    jscpd --version | grep -F "${JSCPD_VERSION}"

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