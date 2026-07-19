# syntax=docker/dockerfile:1.7

ARG RUST_IMAGE=rust:1.95.0-bookworm@sha256:6258907abe69656e41cd992e0b705cdcfabcbbe3db374f92ed2d47121282d4a1
ARG RUNTIME_IMAGE=debian:bookworm-slim@sha256:7b140f374b289a7c2befc338f42ebe6441b7ea838a042bbd5acbfca6ec875818

FROM ${RUST_IMAGE} AS chef

WORKDIR /src/backend

ARG CARGO_CHEF_VERSION=0.1.77
RUN cargo install cargo-chef --version "${CARGO_CHEF_VERSION}" --locked

FROM chef AS planner
COPY backend/ ./
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS build

ARG CHILL_VERSION=dev
ARG CHILL_COMMIT=unknown
ARG CHILL_BUILD_TIME=unknown
ARG CARGO_BUILD_JOBS=1
ENV CHILL_VERSION=${CHILL_VERSION} \
    CHILL_COMMIT=${CHILL_COMMIT} \
    CHILL_BUILD_TIME=${CHILL_BUILD_TIME} \
    CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS} \
    CARGO_PROFILE_RELEASE_LTO=false

COPY --from=planner /src/backend/recipe.json recipe.json
RUN cargo chef cook --locked --release --recipe-path recipe.json

# The control-plane crate embeds repository-owned policies and migrations at
# compile time, so preserve the repository layout for the source-only layer.
COPY backend/ ./
COPY policies/ /src/policies/
COPY schemas/behavior/v1/envelope.schema.json /src/schemas/behavior/v1/envelope.schema.json

RUN cargo build --locked --release --bin chilld --bin chillctl && \
    install -D -m 0755 target/release/chilld /out/chilld && \
    install -D -m 0755 target/release/chillctl /out/chillctl

FROM ${RUNTIME_IMAGE}

COPY --from=build /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/ca-certificates.crt
COPY --from=build --chown=65532:65532 /out/chilld /usr/local/bin/chilld
COPY --from=build --chown=65532:65532 /out/chillctl /usr/local/bin/chillctl
COPY --from=build --chown=65532:65532 /src/schemas/behavior/v1/envelope.schema.json /usr/local/share/chill/envelope.schema.json
COPY --chown=65532:65532 LICENSE NOTICE /usr/local/share/licenses/chill/

USER 65532:65532
WORKDIR /var/lib/chill
EXPOSE 4318

CMD ["/usr/local/bin/chilld"]
