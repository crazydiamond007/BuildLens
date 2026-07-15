# Build stage.
#
# Dependencies are compiled in their own layer, keyed on Cargo.toml/Cargo.lock
# alone. Rust builds are slow enough that without this, touching one line of
# main.rs would recompile every crate in the tree on every image build.
FROM rust:1.96-slim-bookworm AS builder

WORKDIR /build

RUN apt-get update \
    && apt-get install -y --no-install-recommends pkg-config \
    && rm -rf /var/lib/apt/lists/*

COPY Cargo.toml Cargo.lock ./

# A throwaway main.rs, so `cargo build` has something to compile and caches the
# dependency graph. Then we delete the artifacts of the dummy binary itself, so
# the real source cannot be mistaken for already-built.
RUN mkdir src \
    && echo 'fn main() {}' > src/main.rs \
    && cargo build --release \
    && rm -rf src target/release/deps/buildlens_gateway*

COPY src ./src
RUN cargo build --release

# Runtime stage.
#
# Nothing from the build stage comes along except the binary itself: no compiler,
# no source, no cargo registry. rustls means no OpenSSL to install either. The
# TLS stack is statically linked into the binary.
FROM debian:bookworm-slim

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --create-home --uid 10001 buildlens

WORKDIR /app

COPY --from=builder /build/target/release/buildlens-gateway /app/buildlens-gateway

USER buildlens

EXPOSE 8080

CMD ["/app/buildlens-gateway"]
