FROM rust:1.96-bookworm AS builder
WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY migrations ./migrations
RUN cargo build --release --bin open-sentry

FROM debian:bookworm-slim AS runtime
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates curl && rm -rf /var/lib/apt/lists/*
RUN useradd --system --uid 10001 --create-home sentry
COPY --from=builder /app/target/release/open-sentry /usr/local/bin/open-sentry
USER sentry
EXPOSE 8080
ENTRYPOINT ["/usr/local/bin/open-sentry"]
