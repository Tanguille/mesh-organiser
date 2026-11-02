#!/bin/bash
set -e

case "$TARGETARCH" in
    amd64)
        RUST_TARGET=x86_64-unknown-linux-gnu
        ;;
    arm64)
        RUST_TARGET=aarch64-unknown-linux-gnu
        # cc-rs finds aarch64-linux-gnu-gcc itself; cargo has to be told to link with it.
        export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc
        ;;
    *)
        echo "Unsupported architecture: $TARGETARCH"
        exit 1
        ;;
esac

rustup target add "$RUST_TARGET"
cargo build -p web --release --locked --target "$RUST_TARGET"
# `web` is a workspace member, so cargo writes to the root target/, not web/target/.
install -D "target/$RUST_TARGET/release/web" /out/web
