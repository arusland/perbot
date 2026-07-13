# syntax=docker/dockerfile:1

# ---- Stage 1: build a fully static musl binary ----
FROM rust:1-alpine AS builder

# musl-dev/gcc for the bundled SQLite C sources, static OpenSSL for native-tls
RUN apk add --no-cache musl-dev pkgconf openssl-dev openssl-libs-static

WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY src ./src

# Link OpenSSL statically so the runtime image needs no libssl
ENV OPENSSL_STATIC=1

RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/app/target \
    cargo build --release --locked --bin perbot \
    && cp target/release/perbot /usr/local/bin/perbot

# ---- Stage 2: minimal runtime ----
FROM alpine:3.22

# CA bundle for the Telegram API TLS, tzdata so TZ resolves for process-local time
RUN apk add --no-cache ca-certificates tzdata \
    && addgroup -S perbot \
    && adduser -S -G perbot perbot

COPY --from=builder /usr/local/bin/perbot /usr/local/bin/perbot

# The bot opens data/perbot.db relative to the working directory,
# so WORKDIR / keeps the db at /data/perbot.db (the mounted volume)
WORKDIR /
RUN mkdir -p /data && chown perbot:perbot /data
VOLUME /data

USER perbot

# Scheduling runs in UTC + the per-chat timezone setting; TZ only affects
# process-local time (e.g. log timestamps). Default it to Berlin.
ENV TZ=Europe/Berlin

# Required at runtime: TG_BOT_TOKEN, TG_ADMIN_ID. Optional: RUST_LOG, TZ.
ENTRYPOINT ["perbot"]
