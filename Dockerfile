# syntax=docker/dockerfile:1
# Explicit Debian release: the binary links against the build image's glibc, so the
# runtime image must never silently move to a newer or older release than the build.
FROM --platform=$BUILDPLATFORM rust:1.99.0-slim-trixie AS build
ARG TARGETARCH
WORKDIR /source

# The build image ships gcc; the arm64 cross build also needs a cross linker and the arm64 libc headers.
RUN apt-get update && apt-get install -y --no-install-recommends wget \
    && if [ "$TARGETARCH" = arm64 ]; then apt-get install -y --no-install-recommends gcc-aarch64-linux-gnu libc6-dev-arm64-cross; fi \
    && rm -rf /var/lib/apt/lists/*

SHELL ["/bin/sh", "-o", "pipefail", "-c"]
RUN wget -qO- https://get.pnpm.io/install.sh | ENV="$HOME/.shrc" SHELL="$(which sh)" sh -
ENV PNPM_HOME=/root/.local/share/pnpm
ENV PATH=$PNPM_HOME/bin:$PATH

# Debian's nodejs is older than SvelteKit supports, so install the Node pinned in mise.toml.
# Copied on its own so source changes don't re-download Node.
COPY mise.toml .
RUN pnpm runtime set node "$(sed -n 's/^node = "\(.*\)"/\1/p' mise.toml)" -g

COPY . .
RUN bash ./build-web.sh

ARG VITE_APP_VERSION
ENV VITE_API_PLATFORM=web
RUN pnpm install --frozen-lockfile && pnpm build

FROM debian:trixie-slim AS runtime
WORKDIR /app

RUN groupadd -r appuser && useradd -r -g appuser appuser

COPY --from=build --chown=appuser:appuser /out/web .
COPY --from=build --chown=appuser:appuser /source/build ./www

RUN apt-get update && apt-get install -y --no-install-recommends wget \
    && rm -rf /var/lib/apt/lists/*

USER appuser

HEALTHCHECK --interval=30s --timeout=3s --start-period=5s --retries=3 \
    CMD wget -q --spider "http://127.0.0.1:${SERVER_PORT:-3000}/" || exit 1

ENTRYPOINT ["/app/web"]
