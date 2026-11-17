# Discord Community Scout - self-host container image.
# No paid services. SQLite is stored in /app/data (mount a volume to persist it).

FROM rust:1.100-bookworm AS builder
WORKDIR /app

# Copy everything the build needs before compiling. This is deliberately
# simple: an earlier "stub crate" dependency-cache trick shipped an empty
# binary because Cargo's fingerprint is mtime-based and Docker COPY preserves
# source mtimes, so the real build was skipped. BuildKit caches this layer,
# so unchanged sources are still not rebuilt.
COPY Cargo.toml Cargo.lock ./
COPY migrations ./migrations
COPY static ./static
COPY src ./src

RUN cargo build --release --locked \
    && test -x target/release/discord-community-scout

FROM debian:bookworm-slim AS runtime
WORKDIR /app

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/discord-community-scout /usr/local/bin/discord-community-scout
COPY --from=builder /app/static ./static
COPY --from=builder /app/migrations ./migrations

RUN mkdir -p /app/data

ENV PORT=8080 \
    DATABASE_URL=sqlite:///app/data/scout.db?mode=rwc \
    RUST_LOG=info \
    SCOUT_CACHE_TTL_SECS=3600

EXPOSE 8080
VOLUME ["/app/data"]

HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
  CMD curl -fsS http://127.0.0.1:${PORT}/api/health || exit 1

CMD ["discord-community-scout"]
