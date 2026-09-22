FROM rust:1.86-slim AS builder

WORKDIR /app

# Install build dependencies (curl is used by utoipa-swagger-ui's build script)
RUN apt-get update && apt-get install -y --no-install-recommends libssl-dev pkg-config curl ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# sqlx offline mode: compile-time query checks use the committed .sqlx metadata
ENV SQLX_OFFLINE=true

# Build dependencies first so they are cached independently of source changes
COPY Cargo.toml Cargo.lock ./
RUN mkdir src && echo "fn main() {}" > src/main.rs && touch src/lib.rs \
    && cargo build --release --locked \
    && rm -rf src

# Copy sources, query metadata and migrations (embedded into the binary)
COPY src ./src
COPY .sqlx ./.sqlx
COPY migrations ./migrations

# Touch sources so cargo rebuilds the crate rather than reusing the stub
RUN touch src/main.rs src/lib.rs && cargo build --release --locked

# Create runtime image
FROM debian:stable-slim

WORKDIR /app

RUN apt-get update && apt-get install -y --no-install-recommends libssl3 ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --no-create-home app

COPY --from=builder /app/target/release/clean_axum_demo /usr/local/bin/clean_axum_demo

# Public assets ship with the release; private uploads are runtime state (mount a volume)
COPY assets/public ./assets/public
RUN mkdir -p assets/private && chown -R app:app assets/private

# No config is baked in: supply all settings as environment variables at run time.
ENV RUST_LOG=info \
    LOG_FORMAT=json \
    SERVICE_HOST=0.0.0.0 \
    SERVICE_PORT=8080

USER app

EXPOSE 8080

ENTRYPOINT ["clean_axum_demo"]
CMD ["serve"]
