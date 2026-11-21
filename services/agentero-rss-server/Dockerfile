# Build from the repository root:
#   docker build -f services/agentero-rss-server/Dockerfile -t agentero-rss-server .
FROM rust:1-bookworm AS build
RUN apt-get update && apt-get install -y --no-install-recommends cmake clang pkg-config && rm -rf /var/lib/apt/lists/*
WORKDIR /src
COPY . .
RUN cargo build --release --locked -p agentero-rss-server \
    && install -Dm755 target/release/agentero-rss-server /out/agentero-rss-server \
    && install -Dm755 target/release/agentero-rss-admin /out/agentero-rss-admin

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --home /data rss && mkdir -p /data && chown rss /data
COPY --from=build /out/ /usr/local/bin/
USER rss
ENV RSS_BIND=0.0.0.0:8787 RSS_DB_PATH=/data/rss-archive.sqlite
VOLUME /data
EXPOSE 8787
CMD ["agentero-rss-server"]
