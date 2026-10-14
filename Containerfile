FROM node:22-alpine AS web-build
WORKDIR /source/web
COPY web/package.json web/package-lock.json ./
RUN npm ci
COPY web/ ./
RUN npm run build

FROM rust:1.94-bookworm AS rust-build
WORKDIR /source
COPY Cargo.toml Cargo.lock ./
COPY crates/ crates/
RUN find crates -type f -exec touch {} +
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/source/target,sharing=locked \
    cargo build --locked --release -p hedwig-server -p hedwigctl \
    && mkdir -p /out \
    && cp /source/target/release/hedwig-server /source/target/release/hedwigctl /out/

FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --home-dir /var/lib/hedwig hedwig \
    && mkdir -p /var/lib/hedwig /var/backups/hedwig /opt/hedwig/web \
    && chown -R hedwig:hedwig /var/lib/hedwig /var/backups/hedwig
COPY --from=rust-build /out/hedwig-server /usr/local/bin/hedwig-server
COPY --from=rust-build /out/hedwigctl /usr/local/bin/hedwigctl
COPY --from=web-build /source/web/dist/ /opt/hedwig/web/
USER hedwig
EXPOSE 8080
ENV HEDWIG_DATA_DIR=/var/lib/hedwig \
    HEDWIG_WEB_DIR=/opt/hedwig/web \
    HEDWIG_LISTEN=0.0.0.0:8080
ENTRYPOINT ["hedwig-server"]
