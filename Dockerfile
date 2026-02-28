# syntax=docker/dockerfile:1
# Build stage
FROM --platform=$BUILDPLATFORM rust:1.93.1-slim AS build
WORKDIR /source
COPY . .
ARG VITE_APP_VERSION
ARG TARGETPLATFORM
ARG BUILDPLATFORM

RUN apt-get update && apt-get install -y --no-install-recommends \
    wget xz-utils nodejs libfontconfig1-dev libssl-dev openssl \
    build-essential cmake gcc-aarch64-linux-gnu g++-aarch64-linux-gnu \
    && rm -rf /var/lib/apt/lists/*

SHELL ["/bin/sh", "-o", "pipefail", "-c"]
RUN wget -qO- https://get.pnpm.io/install.sh | ENV="$HOME/.shrc" SHELL="$(which sh)" sh -

RUN chmod +x ./build-web.sh && ./build-web.sh
RUN export PNPM_HOME="/root/.local/share/pnpm" && export PATH="$PNPM_HOME:$PATH" && pnpm i && export VITE_API_PLATFORM=web && pnpm build

# Runtime stage
FROM debian:stable-slim AS runtime
WORKDIR /app

RUN groupadd -r appuser && useradd -r -g appuser appuser

COPY --from=build --chown=appuser:appuser /source/web/target/release/web .
COPY --from=build --chown=appuser:appuser /source/build ./www

RUN apt-get update && apt-get install -y --no-install-recommends wget \
    && rm -rf /var/lib/apt/lists/*

USER appuser

HEALTHCHECK --interval=30s --timeout=3s --start-period=5s --retries=3 \
    CMD wget -q --spider "http://127.0.0.1:${SERVER_PORT:-3000}/" || exit 1

ENTRYPOINT ["/app/web"]