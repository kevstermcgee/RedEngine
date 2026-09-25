# 0027. Runs anywhere: `doctor`, environment config, a container image, software GPU fallback, LF everywhere
Status: accepted

## Context
Red has to run on a Windows dev PC, a Linux CI runner with no GPU, a container and a small VPS. The pieces existed (the `gfx`
feature, ADR 0017) but nothing told a newcomer what a given machine could do, the server was configurable only by flags,
`docker stop` (SIGTERM) killed it without the graceful shutdown, a GPU-less machine failed `frame`/`tour` even when a software
Vulkan adapter was installed, and files checked out on Windows had CRLF endings that broke text-comparing tests and could make
golden/trace hashes differ.

## Decision
- `red_engine2 doctor` probes for real (GPU adapter hardware then software, audio, ffmpeg, UDP loopback and the default port, a
  writable output directory, git) and ends with "what works here". Only a broken UDP or output directory is a failure: nearly
  everything else works without a GPU (ADR 0017).
- `Gpu::new` falls back to a software adapter (`force_fallback_adapter`) when no hardware one exists (Mesa lavapipe, Windows WARP).
- `red_server` reads `RED_MAP`, `RED_PORT`, `RED_BIND`, `RED_SPAWN_GROUP`, `RED_SNAPSHOT_EVERY`, `RED_TIMEOUT_MS`, `RED_STATS_SECS`,
  `RED_RUN_FOR` (a flag wins; a malformed value exits 2 with the variable's name), and handles SIGTERM as well as Ctrl-C.
- `Dockerfile` (multi-stage, headless, non-root, UDP 27015), `docker-compose.yml`, `deploy/red-server.service` (systemd) and
  `docs/HOSTING.md`; a CI job builds the image and checks it listens and stops cleanly.
- `.gitattributes` forces LF (`* text=auto eol=lf`) and text-reading tests normalise `\r\n`; scripts are bash plus PowerShell twins.

## Consequences
Deployment is one command on any Docker host. Not done or not proven: the Linux CI jobs and the container image have not been run
from the Windows development machine that wrote this (only `cargo check --target x86_64-unknown-linux-gnu` and a `--no-default-features` build), macOS is
untested, and the wire protocol has no encryption or authentication (`docs/HOSTING.md` recommends a private network until that
lands; see `docs/analysis/2026-09-24-cheddar-feedback.md`).
