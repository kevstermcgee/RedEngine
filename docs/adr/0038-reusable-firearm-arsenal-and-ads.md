# 0038. A reusable firearm arsenal and aim-down-sights presentation
Status: accepted

## Context

RedDM exposed a shooter-project bottleneck: the authoritative combat path, wire format and viewmodel
renderer were reusable, but `Weapon` was hard-coded to one firearm and aiming had no standard input or
camera transition. Building a game-local parallel combat stack would have split offline, server and replay
behaviour and made every future shooter repeat the same work.

## Decision

- `weapons::Weapon` provides a stable engine arsenal: eleven firearms plus the legacy bat. Each firearm
  has pure `FirearmSpec` tuning for damage, tick-quantised cadence, range, impulse and visual recoil.
- `sim::interact` uses that same tuning for authoritative hitscan, damage and prop impulse. Weapon ids still
  use the bounded one-byte protocol field and unknown ids retain the legacy bat fallback.
- `firearms.rs` generates lightweight, original procedural models from engine primitives; no binary asset
  or imported-model dependency is added. The bespoke revolver remains compatible.
- The graphical client reserves right mouse for smooth ADS: one blend drives FOV and viewmodel alignment.
  ADS is presentation only; it does not weaken server authority or add a protocol input.
- The legacy scene `weapons.bat` / `weapons.revolver` fields remain valid. General per-game loadouts,
  per-firearm magazines, automatic fire and spread are intentionally follow-up work rather than hidden
  game-specific exceptions.

## Consequences

Shooter prototypes can start with a real, network-authoritative library instead of cloning revolver code,
and existing content keeps its behaviour and schema. All firearms temporarily share the existing ammo
state and fire on presses; a future generalized loadout schema can replace that representation without
changing the wire id, hit test or model APIs introduced here.
