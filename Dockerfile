FROM node:26-bookworm-slim AS frontend-build
WORKDIR /src/frontend
COPY frontend/package.json frontend/package-lock.json ./
RUN npm ci
COPY frontend/ ./
RUN npm run build

FROM rust:bookworm AS backend-build
WORKDIR /src
RUN apt-get update \
    && apt-get install --yes --no-install-recommends pkg-config libssl-dev \
    && rm -rf /var/lib/apt/lists/*
COPY README.md LICENSE ./
COPY vendor/ ./vendor/
COPY rust-backend/ ./rust-backend/
RUN cargo build --locked --release --manifest-path rust-backend/Cargo.toml

FROM debian:bookworm-slim AS runtime
RUN apt-get update \
    && apt-get install --yes --no-install-recommends ca-certificates curl libssl3 \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --create-home --uid 10001 workstation \
    && mkdir --parents /app/frontend/dist /data /state \
    && chown --recursive workstation:workstation /app /data /state
COPY --from=backend-build /src/rust-backend/target/release/option-workstation /usr/local/bin/option-workstation
COPY --from=frontend-build /src/frontend/dist/ /app/frontend/dist/

USER workstation
WORKDIR /app
ENV OPTION_WORKSTATION_HOST=0.0.0.0 \
    OPTION_WORKSTATION_PORT=7311 \
    OPTION_WORKSTATION_DATA_ROOT=/data \
    OPTION_WORKSTATION_FRONTEND_DIST=/app/frontend/dist \
    OPTION_WORKSTATION_AUDIT_PATH=/state/audit.jsonl \
    RUST_LOG=option_workstation=info,tower_http=info
EXPOSE 7311
HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
    CMD curl --fail --silent http://127.0.0.1:7311/api/health >/dev/null || exit 1
ENTRYPOINT ["/usr/local/bin/option-workstation"]
