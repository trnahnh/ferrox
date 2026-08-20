# Builds and runs examples/subscriber.rs as a standalone container.
#
# This is the cloud leg of docs/SYSTEM_DESIGN.md §13: the matching engine
# itself is never containerized (it stays pinned bare-metal, §11) — only the
# decoupled UDP subscriber that consumes ExecutionReport broadcasts.
#
# In Fargate, run with MULTICAST_GROUP unset (unicast mode) since the
# multicast feed is bridged in by examples/relay.rs running on-prem — see
# docs/SYSTEM_DESIGN.md §13.2.

FROM rust:1.93-slim AS build
WORKDIR /build

COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY examples ./examples
COPY benches ./benches

RUN cargo build --release --example subscriber

FROM debian:trixie-slim AS runtime
RUN useradd --system --no-create-home subscriber
COPY --from=build /build/target/release/examples/subscriber /usr/local/bin/subscriber
USER subscriber

ENV BIND_ADDR=0.0.0.0:9001
EXPOSE 9001/udp

ENTRYPOINT ["/usr/local/bin/subscriber"]
