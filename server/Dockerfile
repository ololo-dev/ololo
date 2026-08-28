# Single-stage: build + run in rust:1.93-slim so we avoid a second apt-get
# in a debian:bookworm-slim runtime (its apt GPG keyring is broken on the
# Dokploy build host). Image is larger but the builder apt is cached/working.
FROM rust:1.93-slim
WORKDIR /app
RUN rm -rf /var/lib/apt/lists/* && for i in 1 2 3; do \
        apt-get update && apt-get install -y --no-install-recommends \
            build-essential pkg-config libsqlite3-dev ca-certificates curl git \
        && break || sleep 5; \
    done && rm -rf /var/lib/apt/lists/*

COPY Cargo.toml Cargo.lock ./
COPY arena-core/ arena-core/
COPY agent-tokens/ agent-tokens/
COPY email-templates/ email-templates/
COPY server/ server/
COPY game-server/ game-server/
COPY ololo/ ololo/

RUN cargo build --release -p server

# Seed content scanned at boot (ARENA_PROJECTS_DIR / ARENA_JUDGES_DIR).
# Kept in the image, not /data: an existing named volume mounted at /data
# would hide anything baked there.
COPY projects/ projects/
COPY judges/ judges/

RUN mkdir -p /data
ENV SERVER_PORT=8080
EXPOSE 8080
VOLUME ["/data"]

HEALTHCHECK --interval=30s --timeout=5s --start-period=20s --retries=3 \
    CMD curl -f http://localhost:8080/health || exit 1

ENTRYPOINT ["/app/target/release/server"]