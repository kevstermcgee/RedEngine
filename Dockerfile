# The dedicated Red server (and the map/analysis CLI) in a small image. No GPU, window or audio libraries: the
# `--no-default-features` build is the headless one (ADR 0017).
#
#   docker build -t red-server .
#   docker run --rm -p 27015:27015/udp red-server                                 # the built-in Test Lab
#   docker run --rm -p 27015:27015/udp -v "$PWD/maps:/maps:ro" -e RED_MAP=/maps/main.json -e RED_SPAWN_GROUP=duel red-server
#   docker run --rm --entrypoint red_engine2 -v "$PWD/maps:/maps:ro" red-server verify /maps/main.json --no-views
#
# Configuration is by environment (RED_MAP, RED_PORT, RED_BIND, RED_SPAWN_GROUP, ...; see src/bin/red_server.rs) or flags
# appended to `docker run`. `docker stop` sends SIGTERM, which the server handles (clients are told, a --record trace is saved).

FROM rust:1-slim-bookworm AS build
WORKDIR /src
COPY . .
RUN cargo build --locked --release --no-default-features --bin red_server --bin red_bot --bin red_engine2

FROM debian:bookworm-slim
RUN useradd --system --uid 10001 --no-create-home red
COPY --from=build /src/target/release/red_server /src/target/release/red_bot /src/target/release/red_engine2 /usr/local/bin/
COPY examples/test_lab.json /opt/red/test_lab.json
USER red
ENV RED_MAP=/opt/red/test_lab.json RED_PORT=27015 RED_BIND=0.0.0.0
EXPOSE 27015/udp
ENTRYPOINT ["red_server"]
