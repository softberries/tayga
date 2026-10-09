# The web app, built on its own so the Rust stages never need Node.
# node:24-bookworm-slim, pinned by digest on 2026-10-06.
FROM node:24-bookworm-slim@sha256:d6aa754f16b3197301076f047b5def2f02ea1dbbc2ca920407d46d7ec7f87b20 AS ui
WORKDIR /ui
COPY ui/package.json ui/package-lock.json ./
RUN npm ci
COPY ui/ ./
RUN npm run build

# lukemathwalker/cargo-chef:latest-rust-1.98-trixie, pinned by digest on 2026-10-06.
FROM lukemathwalker/cargo-chef:latest-rust-1.99-trixie@sha256:c11fb4b1e7a7caf6ba0dfed8f059011bb2024465b8be5c4dfa330672698cca1a AS chef
RUN apt-get update && apt-get install -y --no-install-recommends cmake && rm -rf /var/lib/apt/lists/*
WORKDIR /app

FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
COPY --from=planner /app/recipe.json recipe.json
RUN cargo chef cook --release --recipe-path recipe.json
COPY . .
# tayga-api embeds ui/dist at compile time (rust-embed, feature embed-ui).
COPY --from=ui /ui/dist /app/ui/dist
RUN cargo build --release --workspace --bins

# debian:trixie-slim, pinned by digest on 2026-10-06.
FROM debian:trixie-slim@sha256:a29215f6a35e51e22adffa17f89e9d2ef06214e64a2bad10d765c46aea49f11f
# The services write nothing to disk and listen above 1024, so they run unprivileged.
# tayga-devtools ships for `hash-password` and `remine` (docker run / compose exec / kubectl exec).
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --no-create-home --shell /usr/sbin/nologin tayga
COPY --from=builder /app/target/release/tayga-ingest /app/target/release/tayga-writer /app/target/release/tayga-assembler /app/target/release/tayga-api /app/target/release/tayga-logminer /app/target/release/tayga-notifier /app/target/release/tayga-devtools /usr/local/bin/
ENV RUST_LOG=info
USER tayga
