#!/usr/bin/env bash
# Measures the engine's own 3D player in a real headless Chromium: build size, startup, shaders and pipelines, frame time, memory, asset loading, input, and parity with the native
# engine (the same scripted inputs through the same simulation). EXPERIMENTAL and not a CI gate: it needs the wasm32 target, wasm-bindgen-cli and a Playwright Chromium, and it
# runs on whatever GPU the machine has (headless CI: a software adapter, so the numbers describe that adapter and the report says so).
#
# usage: scripts/web3d_measure.sh [scene.json ...]       default: recipes/gated_garden.json examples/marcel/marcel.json
# output: out/web3d/measure-<scene>.json, out/web3d/<scene>.png, and a table on stdout
set -euo pipefail
cd "$(dirname "$0")/.."
for f in "$HOME/.local/toolchain/env.sh" "$HOME/.cargo/env"; do [ -f "$f" ] && . "$f"; done
case ":$PATH:" in *":$HOME/.cargo/bin:"*) ;; *) PATH="$HOME/.cargo/bin:$PATH" ;; esac
export PATH
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$PWD/target/web3d}"
rustup target list --installed 2>/dev/null | grep -qx wasm32-unknown-unknown || { echo "SKIPPED: rustup target add wasm32-unknown-unknown"; exit 0; }
PY="${RED2D_BROWSER_PYTHON:-$HOME/.cache/red_engine2/browser/bin/python}"
[ -x "$PY" ] || PY=python3
"$PY" -c 'import playwright' 2>/dev/null || { echo "SKIPPED: no Playwright (red_engine2 web setup-browser)"; exit 0; }
WB="${WASM_BINDGEN:-/tmp/wb/bin/wasm-bindgen}"
[ -x "$WB" ] || cargo install wasm-bindgen-cli --version 0.2.128 --locked --root /tmp/wb   # the version in Cargo.lock

echo "== build (wasm32, release)"
t0=$(date +%s); cargo build -q -p web3d --target wasm32-unknown-unknown --release; echo "built in $(( $(date +%s) - t0 )) s"
dist=out/web3d/dist; rm -rf "$dist"; mkdir -p "$dist"
"$WB" --target web --out-dir "$dist" "$CARGO_TARGET_DIR/wasm32-unknown-unknown/release/web3d.wasm"
cp crates/web3d/web/index.html "$dist/"

scenes=("$@"); [ ${#scenes[@]} -gt 0 ] || scenes=(recipes/gated_garden.json examples/marcel/marcel.json)
# the same scripted inputs, per scene: the garden's walks to the key, through the gate and onto the bench; anything else walks forward, runs, strafes
script_for() { case "$(basename "$1")" in gated_garden.json) echo "KeyW:56;KeyA:58;KeyD:56;KeyW:260;:20" ;; *) echo "KeyW:90;KeyW+ShiftLeft:60;KeyD:45;:20" ;; esac; }
status=0
for scene in "${scenes[@]}"; do
  name=$(basename "$scene" .json); script=$(script_for "$scene")
  echo "== $name (parity script: $script)"
  cargo run -q --no-default-features --example web3d_parity -- "$scene" "$script" > "out/web3d/expected-$name.json"
  "$PY" crates/web3d/measure.py --dist "$dist" --scene "$scene" --name "$name" --parity-script "$script" --expected "out/web3d/expected-$name.json" --out out/web3d > "out/web3d/measure-$name.json" || status=1
  "$PY" - "out/web3d/measure-$name.json" <<'PYEND'
import json, sys
r = json.load(open(sys.argv[1]))
if r.get("error"): print("  FAILED:", r["error"]); sys.exit(0)
s = r["size"]; st = r["startup"]; f = r["frame"]; m = r["memory"]["after_frames"]; p = r.get("parity", {})
print("  adapter        ", r["adapter"], "(software)" if r["software_adapter"] else "(hardware)")
print("  build size     wasm %.2f MB (gzip %.2f MB), glue %d KB, scene %d bytes" % (s["web3d_bg.wasm"]["bytes"] / 1e6, s["web3d_bg.wasm"]["gzip_bytes"] / 1e6, s["web3d.js"]["bytes"] // 1024, s[next(k for k in s if k.startswith("scene-"))]["bytes"]))
print("  startup        module %d ms, scene fetch %d ms, GPU + pipelines %d ms, navigation to ready %d ms" % (st["module_ms"], st["scene_fetch_ms"], st["gpu_and_pipelines_ms"], st["navigation_to_ready_ms"]))
print("  render         %d colours, pipelines built: %s, shader/pipeline messages: %d, first readback %d ms" % (r["render"]["distinct_colours"], r["render"]["pipelines_built"], len(r["render"]["shader_or_pipeline_messages"]), r["render"]["snapshot_ms"]))
print("  frame          at %dx%d, host load %s: CPU side median %.1f ms (p95 %.1f); a whole frame with the GPU %.0f ms; presented %.2f fps" % (r["canvas"][0], r["canvas"][1], r["host"]["loadavg_1m"], f["frame_call_ms"]["median"], f["frame_call_ms"]["p95"], f["gpu_frame_ms"].get("median", float("nan")), f["presented_fps"]))
print("  memory         js heap %s MB, wasm memory %.1f MB" % (("%.1f" % (m["js_heap"] / 1e6)) if m["js_heap"] else "?", m["wasm_memory"] / 1e6))
print("  input          keyboard %s, mouse look %s, touch %s, gamepad %s" % tuple({True: "ok", False: "FAIL", None: "not wired"}[r["input"][k]["ok"]] for k in ("keyboard", "mouse_look", "touch", "gamepad")))
if "same_checksum" in p: print("  parity         wasm in Chromium vs native: checksum equal %s, position difference %.6f m, rules state equal %s, the play changed the picture %s (browser %s, native %s)" % (p["same_checksum"], p["position_difference_m"], p["same_rules_state"], p["picture_changed_by_play"], p["browser"]["checksum"], p["native"]["checksum"]))
print("  webgl2         %s" % ("works" if r["webgl2"]["works"] else "does not start: " + r["webgl2"]["state"][:110]))
PYEND
done
exit $status
