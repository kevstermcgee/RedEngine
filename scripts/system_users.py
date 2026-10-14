#!/usr/bin/env python3
"""Which games use each game-sized system of the engine: the measurement behind the audit of ADR 2026-10-13-the-next-phase, item 5.

    python3 scripts/system_users.py [--games ../RedEngineGames] [--json]

A system is used by a game when one of the game's scenes has the scene key that turns it on (`shooter` the loadout shooter, `race` the karts, `procgen`
the endless meadow, ...), or when a game crate's own Rust names it (`games/<crate>/src`: Killchain's client is written on the shooter's modules). A game
is a folder that holds scenes: `examples/<name>` (a scene file directly under `examples/` is a game of its own), `recipes/<name>`, `games/<crate>`,
`tests/fixtures/<name>`, and in a RedEngineGames checkout `projects/<name>`, `games/<name>` (copies of this repository's; counted once, under their
source) and `prototypes/<name>`. A game published from here and generated here too (`examples/3d/x`, `projects/x`) is one game. The fixture that
exercises every key (`tests/fixtures/content`) is not a game. Lines are counted from the files each system lives in. Prints, per system: its lines,
the games that use it, the engine's own examples and recipes that use it (`examples/`, `recipes/`: it is offered to whoever reads them), and the
`new-game` kind that scaffolds a game on it; or the same as JSON.
"""
import argparse
import json
import re
import sys
from pathlib import Path

# system -> (files and folders it lives in, scene keys that turn it on, Rust paths a game crate may name it by)
SYSTEMS = {
    "loadout shooter": (
        ["crates/red_core/src/arsenal.rs", "crates/red_core/src/killcam.rs", "src/shooter_world.rs", "src/objective_world.rs",
         "crates/red_core/src/sim/kit.rs", "crates/red_core/src/sim/ordnance.rs", "crates/red_core/src/sim/shooter.rs",
         "crates/red_core/src/sim/objective.rs", "crates/red_core/src/sim/objective_run.rs"],
        ["shooter"],
        ["arsenal", "killcam", "shooter_world", "objective_world", "sim::kit", "sim::ordnance", "sim::shooter", "sim::objective"],
    ),
    "weapons and guns": (
        ["crates/red_core/src/weapons.rs", "src/firearms.rs"],
        ["weapons", "combat"],
        ["weapons::", "firearms"],
    ),
    "karts and races": (
        ["crates/red_core/src/sim/kart.rs", "crates/red_core/src/sim/race.rs", "src/kart_camera.rs", "src/kart_sound.rs"],
        ["race"],
        ["sim::kart", "sim::race", "kart_camera", "kart_sound"],
    ),
    "endless meadow (procgen)": (
        ["src/procgen", "crates/red_core/src/procgen", "crates/red_core/src/nature.rs", "crates/red_core/src/terrain.rs", "src/stream_gpu.rs"],
        ["procgen"],
        ["procgen::", "stream_gpu", "nature::", "terrain::"],
    ),
    "day and night (clock)": (["crates/red_core/src/daycycle.rs"], ["clock"], ["daycycle"]),
    "wrapping world (expanse)": (["crates/red_core/src/expanse.rs"], ["world"], ["expanse"]),
    "music and soundscape": (
        ["src/music.rs", "crates/red_core/src/listen.rs", "crates/red_core/src/ambience.rs", "src/soundscape.rs"],
        ["audio", "music"],
        ["ambience::", "listen::", "soundscape", "music::"],
    ),
}


# system -> the `new-game` kind that scaffolds a game on it
KINDS = {"karts and races": "race"}


def lines(root: Path, paths) -> int:
    n = 0
    for p in paths:
        p = root / p
        files = sorted(p.rglob("*.rs")) if p.is_dir() else [p]
        n += sum(len(f.read_text(encoding="utf-8", errors="replace").splitlines()) for f in files if f.is_file())
    return n


def game_of(rel: Path) -> str | None:
    """The game a scene file belongs to, from its path (`examples/marcel/marcel.json` -> `examples/marcel`)."""
    parts = rel.parts
    if parts[0] in ("examples", "recipes", "projects", "prototypes", "games") and len(parts) >= 2:
        if parts[0] == "examples" and parts[1] in ("2d", "3d", "external") and len(parts) >= 3:
            return "/".join(parts[:3]) if len(parts) > 3 else "/".join(parts[:2]) + "/" + Path(parts[2]).stem
        return "/".join(parts[:2]) if len(parts) > 2 else f"{parts[0]}/{Path(parts[1]).stem}"
    return None


def scenes(root: Path, dirs):
    for d in dirs:
        base = root / d
        if not base.is_dir():
            continue
        for f in sorted(base.rglob("*.json")):
            if any(part in ("target", "out", "node_modules", ".red", "golden") or part.startswith(".") for part in f.relative_to(root).parts):
                continue
            try:
                v = json.loads(f.read_text(encoding="utf-8"))
            except (ValueError, UnicodeDecodeError):
                continue
            if isinstance(v, dict) and "objects" in v:
                yield f.relative_to(root), v


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--games", type=Path, help="a RedEngineGames checkout, to count the games published there too")
    ap.add_argument("--json", action="store_true")
    a = ap.parse_args()
    root = Path.cwd()
    users = {name: set() for name in SYSTEMS}
    sources = [(root, ["examples", "recipes", "games"], "")]
    if a.games:
        # RedEngineGames' games/ are copies of this repository's (counted under their source here); its projects and prototypes are its own.
        sources.append((a.games, ["projects", "prototypes"], "RedEngineGames:"))
    for base, dirs, prefix in sources:
        for rel, v in scenes(base, dirs):
            g = game_of(rel)
            if not g:
                continue
            for name, (_, keys, _) in SYSTEMS.items():
                if any(k in v for k in keys):
                    users[name].add(prefix + g)
    for crate in sorted((root / "games").glob("*/Cargo.toml")):
        src = "\n".join(f.read_text(encoding="utf-8") for f in (crate.parent / "src").rglob("*.rs"))
        for name, (_, _, rust) in SYSTEMS.items():
            if any(re.search(r"red_engine2::" + re.escape(r), src) for r in rust):
                users[name].add(f"games/{crate.parent.name} (crate)")
    rows = []
    for n in SYSTEMS:
        # A game is one game wherever its copies are (`examples/3d/x`, `RedEngineGames:projects/x`, `games/x (crate)`).
        names = {}
        for g in sorted(users[n]):
            name = g.split(":")[-1].split(" ")[0].rstrip("/").split("/")[-1]
            names.setdefault(name, []).append(g)
        engine_examples = sorted(g for g in users[n] if g.startswith(("examples/", "recipes/")) and not g.startswith("examples/3d/"))
        rows.append({
            "system": n,
            "lines": lines(root, SYSTEMS[n][0]),
            "keys": SYSTEMS[n][1],
            "games": {k: v for k, v in sorted(names.items())},
            "engine_examples": engine_examples,
            "new_game_kind": KINDS.get(n),
        })
    if a.json:
        print(json.dumps(rows, indent=1))
        return 0
    for r in rows:
        offered = ", ".join(filter(None, [f"new-game --kind {r['new_game_kind']}" if r["new_game_kind"] else "", *r["engine_examples"]])) or "nothing"
        print(f"{r['system']:28} {r['lines']:6} lines  {len(r['games']):3} game(s)  keys {', '.join(r['keys'])}  offered by: {offered}")
        for name, where in r["games"].items():
            print(f"    {name:24} {', '.join(where)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
