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
# Production transport (ADR 0044): QUIC + TLS 1.3 with the deployment's own identity, mounted read-only at /identity (make one with
# `docker run --rm -v "$PWD/identity:/identity" --entrypoint red_engine2 red-server net-identity --out /identity`). Without it the server
# refuses to start: it never serves plaintext UDP on a public address unless RED_INSECURE_PUBLIC_UDP=1 says so.
ENV RED_MAP=/opt/red/test_lab.json RED_PORT=27015 RED_BIND=0.0.0.0 RED_TLS_CERT=/identity/cert.pem RED_TLS_KEY=/identity/key.pem
EXPOSE 27015/udp
ENTRYPOINT ["red_server"]
