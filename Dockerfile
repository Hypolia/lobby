# syntax=docker/dockerfile:1

# Build contexts:
#   .            this repository
#   sdk-src      a checkout of hypolia/sdk at sdk.ref
#   rspigot-src  a checkout of ferrislabs/rspigot at rspigot.ref

ARG RUST_VERSION=1.90
ARG DEBIAN_RUNTIME_IMAGE=debian:trixie-slim
ARG DISTROLESS_IMAGE=gcr.io/distroless/cc-debian13:nonroot

FROM rust:${RUST_VERSION}-trixie AS chef

ENV RUSTUP_TOOLCHAIN=stable \
    CARGO_TERM_COLOR=always

RUN apt-get update && apt-get install -y --no-install-recommends \
        protobuf-compiler \
        pkg-config \
        libssl-dev \
    && rm -rf /var/lib/apt/lists/*

RUN --mount=type=cache,id=hypolia-cargo-registry,target=/usr/local/cargo/registry \
    --mount=type=cache,id=hypolia-cargo-git,target=/usr/local/cargo/git \
    cargo install cargo-chef --version 0.1.77 --locked

RUN mkdir -p /distroless-app && chown 65532:65532 /distroless-app

# ==============================================================================
# rspigot server binary
# ==============================================================================
FROM chef AS rspigot-planner

WORKDIR /src/rspigot
COPY --exclude=target --exclude=.git --exclude=rust-toolchain.toml --from=rspigot-src . .
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS rspigot-builder

ARG CARGO_PROFILE_RELEASE_LTO=false
ENV CARGO_PROFILE_RELEASE_CODEGEN_UNITS=16 \
    CARGO_PROFILE_RELEASE_LTO=${CARGO_PROFILE_RELEASE_LTO}
WORKDIR /src/rspigot
COPY --from=rspigot-planner /src/rspigot/recipe.json recipe.json
RUN --mount=type=cache,id=hypolia-cargo-registry-rspigot,sharing=locked,target=/usr/local/cargo/registry \
    --mount=type=cache,id=hypolia-cargo-git-rspigot,sharing=locked,target=/usr/local/cargo/git \
    --mount=type=cache,id=hypolia-cargo-target-rspigot,sharing=locked,target=/src/rspigot/target \
    cargo chef cook --release -p rspigot --recipe-path recipe.json

COPY --exclude=target --exclude=.git --exclude=rust-toolchain.toml --from=rspigot-src . .
RUN --mount=type=cache,id=hypolia-cargo-registry-rspigot,sharing=locked,target=/usr/local/cargo/registry \
    --mount=type=cache,id=hypolia-cargo-git-rspigot,sharing=locked,target=/usr/local/cargo/git \
    --mount=type=cache,id=hypolia-cargo-target-rspigot,sharing=locked,target=/src/rspigot/target \
    cargo build --release -p rspigot \
    && cp /src/rspigot/target/release/rspigot /usr/local/bin/rspigot

# ==============================================================================
# lobby plugin
# ==============================================================================
FROM chef AS plugin-src

COPY --exclude=target --exclude=.git --exclude=rust-toolchain.toml --from=rspigot-src . /opt/minecraft/dev/rspigot
COPY --exclude=target --exclude=.git --from=sdk-src . /src/sdk
WORKDIR /src/lobby
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates

FROM plugin-src AS plugin-planner

RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS plugin-builder

ARG CARGO_PROFILE_RELEASE_LTO=false
ENV CARGO_PROFILE_RELEASE_CODEGEN_UNITS=16 \
    CARGO_PROFILE_RELEASE_LTO=${CARGO_PROFILE_RELEASE_LTO}
COPY --exclude=target --exclude=.git --exclude=rust-toolchain.toml --from=rspigot-src . /opt/minecraft/dev/rspigot
COPY --exclude=target --exclude=.git --from=sdk-src . /src/sdk
WORKDIR /src/lobby
COPY --from=plugin-planner /src/lobby/recipe.json recipe.json
RUN --mount=type=cache,id=hypolia-cargo-registry-lobby,sharing=locked,target=/usr/local/cargo/registry \
    --mount=type=cache,id=hypolia-cargo-git-lobby,sharing=locked,target=/usr/local/cargo/git \
    --mount=type=cache,id=hypolia-cargo-target-lobby,sharing=locked,target=/src/lobby/target \
    cargo chef cook --release --bin hypolia-lobby --recipe-path recipe.json

COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
RUN --mount=type=cache,id=hypolia-cargo-registry-lobby,sharing=locked,target=/usr/local/cargo/registry \
    --mount=type=cache,id=hypolia-cargo-git-lobby,sharing=locked,target=/usr/local/cargo/git \
    --mount=type=cache,id=hypolia-cargo-target-lobby,sharing=locked,target=/src/lobby/target \
    cargo build --release --bin hypolia-lobby --bin hypolia-lobby-migrate \
    && mkdir -p /out \
    && cp target/release/hypolia-lobby target/release/hypolia-lobby-migrate /out/

FROM ${DISTROLESS_IMAGE} AS lobby

WORKDIR /app
COPY --from=chef --chown=65532:65532 /distroless-app /app
COPY --from=plugin-builder /out/ /usr/local/bin/
COPY --chown=65532:65532 lobby.json /app/lobby.json
ARG HYPOLIA_VERSION=dev
ARG RSPIGOT_REF=local
ENV HYPOLIA_VERSION=${HYPOLIA_VERSION} \
    RSPIGOT_REF=${RSPIGOT_REF}
ENTRYPOINT ["/usr/local/bin/hypolia-lobby"]

# ==============================================================================
# lobby server: rspigot + this game's server template
# ==============================================================================
FROM ${DEBIAN_RUNTIME_IMAGE} AS lobby-server

RUN apt-get update && apt-get install -y --no-install-recommends \
        ca-certificates \
        curl \
        python3 \
        unzip \
    && rm -rf /var/lib/apt/lists/*

COPY --from=rspigot-builder /usr/local/bin/rspigot /usr/local/bin/rspigot
COPY server/bukkit.yml server/permissions.yml server/server.properties server/spigot.yml.example server/world.zip /opt/hypolia/template/
COPY --chmod=0755 deploy/docker/rspigot-entrypoint.sh /usr/local/bin/rspigot-entrypoint.sh

EXPOSE 25565 25580 25581 25582
ARG HYPOLIA_VERSION=dev
ARG RSPIGOT_REF=local
ENV HYPOLIA_VERSION=${HYPOLIA_VERSION} \
    RSPIGOT_REF=${RSPIGOT_REF}
ENTRYPOINT ["/usr/local/bin/rspigot-entrypoint.sh"]
