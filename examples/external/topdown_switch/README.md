# Top-down switch (a custom client outside the engine)

A small top-down game built on Red's public client layer, `red_engine2::app` (ADR 0043). It is its own crate: it depends on the engine
by path and copies none of its source. Step on both blue switches to turn their lamps green and open the red gate, then reach the gold
exit.

```bash
cargo run            # play: WASD or click to move, Q / E turn the view, wheel zooms, R restarts, Esc quits
cargo test           # scripted play through the real input path + rendered-pixel checks (writes out/start.png, out/escaped.png)
```

| file | what it is |
|---|---|
| `topdown_switch.json` | the scene; **all gameplay** is its `rules` (switches, gate, exit) and `checks.sim` proves them: `red_engine2 sim topdown_switch.json` |
| `src/lib.rs` | the client: input to `PlayerInput`, a `ViewCamera::top_down` that follows the player, click-to-move, the player marker, the HUD |
| `src/main.rs` | `red_engine2::app::run(game, WindowOptions::default())` |
| `tests/play.rs` | clicks and keys into `TopDown::frame`, rule-state assertions, and `app::Offscreen` presentation checks |

The same scene also plays in first person with the standard client (`re2 examples/external/topdown_switch/topdown_switch.json`), because
the rules run in the shared simulation rather than in either client. `red_engine2 lint` reports the exit as unreachable: static lint
cannot see the rule that opens the gate, which is why the route is proven by `checks.sim` instead.

`Cargo.lock` is a copy of the engine's, so this builds with the dependency versions the engine is tested with (and offline once the
engine has been built).
