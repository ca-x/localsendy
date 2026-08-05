# syntax=docker/dockerfile:1.7

FROM node:24-bookworm-slim AS web-builder
WORKDIR /source/web
COPY web/package.json web/package-lock.json ./
RUN --mount=type=cache,target=/root/.npm,sharing=locked npm ci --ignore-scripts
COPY web/ ./
RUN npm run build

FROM rust:1.97-bookworm AS rust-builder
WORKDIR /source
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY crates ./crates
COPY third_party ./third_party
COPY src ./src
COPY web ./web
COPY --from=web-builder /source/web/dist ./web/dist
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/usr/local/cargo/git,sharing=locked \
    --mount=type=cache,target=/source/target,sharing=locked \
    cargo build --release --locked && \
    cp target/release/localsendy /tmp/localsendy

FROM debian:bookworm-slim AS runtime

ARG VERSION=dev
ARG BUILD_TIME=unknown
ARG GIT_COMMIT=unknown

LABEL org.opencontainers.image.title="Localsendy" \
      org.opencontainers.image.description="A Docker-first LocalSend web client powered by Rust" \
      org.opencontainers.image.source="https://github.com/ca-x/localsendy" \
      org.opencontainers.image.licenses="MIT" \
      org.opencontainers.image.version="${VERSION}" \
      org.opencontainers.image.created="${BUILD_TIME}" \
      org.opencontainers.image.revision="${GIT_COMMIT}"

RUN apt-get update && \
    apt-get install --yes --no-install-recommends ca-certificates curl && \
    rm -rf /var/lib/apt/lists/* && \
    groupadd --gid 10001 localsendy && \
    useradd --uid 10001 --gid 10001 --no-create-home \
      --home-dir /data --shell /usr/sbin/nologin localsendy && \
    install -d -o 10001 -g 10001 -m 0750 /data /data/downloads /data/tmp

COPY --from=rust-builder --chown=10001:10001 /tmp/localsendy /usr/local/bin/localsendy

USER 10001:10001
VOLUME ["/data"]
EXPOSE 52222/tcp 53317/tcp 53317/udp
STOPSIGNAL SIGTERM
ENV LOCALSENDY_BIND=0.0.0.0:52222 \
    LOCALSENDY_ALIAS_LOCALE=auto \
    LOCALSENDY_DEVICE_TYPE=server \
    LOCALSENDY_PORT=53317 \
    LOCALSENDY_DATA_DIR=/data \
    LOCALSENDY_DOWNLOAD_DIR=/data/downloads \
    LOCALSENDY_TEMP_DIR=/data/tmp \
    LOCALSENDY_AUTO_ACCEPT=false \
    LOCALSENDY_NETWORK_INTERFACES=all \
    RUST_LOG=localsendy=info,tower_http=info
HEALTHCHECK --interval=30s --timeout=5s --start-period=8s --retries=3 \
  CMD ["curl", "--fail", "--silent", "--show-error", "http://127.0.0.1:52222/api/v1/health"]
ENTRYPOINT ["/usr/local/bin/localsendy"]
