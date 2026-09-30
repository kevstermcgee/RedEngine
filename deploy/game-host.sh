#!/usr/bin/env bash
# Host this game project on this Linux box as a per-user systemd service (no root needed): builds the headless server from the engine game.json names, makes the
# server's QUIC identity and a join key once, writes the unit and starts it. Safe to run again (an update): identity and key are kept.
#   deploy/install.sh            install / update and (re)start
#   deploy/install.sh --info     print how friends connect (address, fingerprint, join key) without changing anything
#   deploy/install.sh --uninstall  stop and remove the service (identity, key and data are kept)
# Reads game.json: "name", server.map, server.port, server.spawn_group. Env: HOST_PORT overrides the port, HOST_ADDR is the name or address friends use
# (default: this machine's first LAN address). `red_engine2 new-game` writes this file as deploy/install.sh in every project.
set -eu
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
for f in "$HOME/.local/toolchain/env.sh" "$HOME/.cargo/env"; do [ -f "$f" ] && . "$f"; done
json_str() { sed -n "s/.*\"$1\"[[:space:]]*:[[:space:]]*\"\([^\"]*\)\".*/\1/p" "$ROOT/game.json" | head -1; }
NAME="$(json_str name)"
MAP_REL="$(json_str map)"; MAP_REL="${MAP_REL:-maps/main.json}"
GROUP="$(json_str spawn_group)"
PORT="${HOST_PORT:-$(sed -n 's/.*"port"[[:space:]]*:[[:space:]]*\([0-9][0-9]*\).*/\1/p' "$ROOT/game.json" | head -1)}"; PORT="${PORT:-27015}"
[ -n "$NAME" ] || { echo "game.json has no \"name\"" >&2; exit 1; }
DATA="$HOME/.local/share/$NAME"
CONF="$HOME/.config/$NAME"
UNIT_DIR="$HOME/.config/systemd/user"

engine_dir() {
  local p; p="$(json_str path)"
  [ -n "$p" ] || { echo "game.json pins the engine by git, not by path: build it with scripts/red (RED_HEADLESS=1) and set RED_ENGINE to the checkout" >&2; exit 1; }
  (cd "$ROOT/$p" && pwd)
}
[ -z "${RED_ENGINE:-}" ] || engine_dir() { echo "$RED_ENGINE"; }

info() {
  local host="${HOST_ADDR:-$(hostname -I | awk '{print $1}')}"
  echo "connect:      re2 --connect $host:$PORT --server-fingerprint $(cat "$CONF/fingerprint") --key $(sed -n 's/^RED_KEY=//p' "$CONF/server.env") $MAP_REL"
  echo "fingerprint:  $(cat "$CONF/fingerprint")   (public: it names this server)"
  echo "join key:     $(sed -n 's/^RED_KEY=//p' "$CONF/server.env")   (private: send it to friends yourself, never commit it)"
  echo "port:         UDP $PORT (forward it on the router for friends outside this network, or use Tailscale)"
}

case "${1:-}" in
  --info) info; exit 0 ;;
  --uninstall)
    systemctl --user disable --now "$NAME.service" 2>/dev/null || true
    rm -f "$UNIT_DIR/$NAME.service"; systemctl --user daemon-reload
    echo "removed the service; $CONF and $DATA are kept"; exit 0 ;;
esac

ENGINE="$(engine_dir)"
echo "building the headless server from $ENGINE (profile fast) ..."
(cd "$ENGINE" && cargo build --profile fast --no-default-features --bin red_server --bin red_engine2)
mkdir -p "$DATA/bin" "$DATA/maps" "$CONF" "$UNIT_DIR"
install -m 755 "$ENGINE/target/fast/red_server" "$DATA/bin/red_server.new" && mv -f "$DATA/bin/red_server.new" "$DATA/bin/red_server"
install -m 755 "$ENGINE/target/fast/red_engine2" "$DATA/bin/red_engine2"
install -m 644 "$ROOT/$MAP_REL" "$DATA/maps/$(basename "$MAP_REL")"

if [ ! -f "$CONF/identity/cert.pem" ]; then
  "$DATA/bin/red_engine2" net-identity --out "$CONF/identity" | tee "$CONF/identity.txt"
  chmod 600 "$CONF/identity/key.pem"
fi
sed -n 's/.*\(sha256:[0-9a-fA-F:]\{20,\}\).*/\1/p' "$CONF/identity.txt" | head -1 > "$CONF/fingerprint"
[ -s "$CONF/fingerprint" ] || { echo "could not read the fingerprint from $CONF/identity.txt" >&2; exit 1; }

if [ ! -f "$CONF/server.env" ]; then
  ( umask 077; echo "RED_KEY=$(head -c 16 /dev/urandom | od -An -tx1 | tr -d ' \n')" > "$CONF/server.env" )
fi

cat > "$UNIT_DIR/$NAME.service" <<UNIT
[Unit]
Description=$NAME game server (Red Engine, QUIC)
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
WorkingDirectory=$DATA
Environment=RED_MAP=$DATA/maps/$(basename "$MAP_REL") RED_PORT=$PORT RED_BIND=0.0.0.0 RED_STATS_SECS=60${GROUP:+ RED_SPAWN_GROUP=$GROUP}
Environment=RED_TLS_CERT=$CONF/identity/cert.pem RED_TLS_KEY=$CONF/identity/key.pem
EnvironmentFile=$CONF/server.env
ExecStart=$DATA/bin/red_server
Restart=on-failure
RestartSec=3
KillSignal=SIGTERM
TimeoutStopSec=10
NoNewPrivileges=true
PrivateTmp=true
MemoryMax=512M
TasksMax=64

[Install]
WantedBy=default.target
UNIT
systemctl --user daemon-reload
systemctl --user enable --now "$NAME.service"
systemctl --user restart "$NAME.service"
sleep 2
systemctl --user --no-pager status "$NAME.service" | head -6
echo
info
