# Discord Community Scout - local/self-host container image.
# No paid services. SQLite is stored in /app/data (mount a volume to persist it).

FROM rust:1.100-bookworm AS builder
WORKDIR /app

# Cache dependency compilation on a stub binary.
COPY Cargo.toml Cargo.lock ./
RUN mkdir -p src && echo "fn main() {}" > src/main.rs
RUN cargo build --release --locked || true

COPY migrations ./migrations
COPY static ./static
COPY src ./src
# Remove stub artifacts so the real binary is built.
RUN find target/release -name 'discord-community-scout*' -type f -delete 2>/dev/null || true \
    && cargo build --release --locked

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
