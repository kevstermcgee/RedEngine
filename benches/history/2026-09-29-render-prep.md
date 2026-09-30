# Render preparation and settled-world scaling (2026-09-29)

Base commit 0b654bd (one after the reviewed 966f63f). Intel N97, 4 cores, `bench` profile (thin LTO), rustc 1.98.1, llvmpipe only
(no GPU access: /dev/dri permission denied), so **CPU preparation only**; GPU time and presentation were not measured.
Median (p50) us per frame, 400-600 samples each (200 for synthetic-8000); p95/p99 are printed by the bench.

| scene (leaf meshes) | old | cached, camera only | cached, 1% moving | cached, all moving | old upload B/frame -> camera-only |
|---|---|---|---|---|---|
| test_lab (350) | 44.4 | 21.9 | 22.0 | 37.0 | 89,600 -> 0 |
| house (1787) | 221 | 127 | 129 | 197 | 457,472 -> 0 |
| school (4532) | 661 | 394 | 397 | 712 | 1,160,192 -> 0 |
| synthetic-2000 (8500) | 933 | 448 | 476 | 1067 | 2,176,000 -> 0 |
| synthetic-8000 (34000) | 5127 | 2006 | 2160 | 3842 | 8,704,000 -> 0 |

Allocations per frame: old = one per leaf mesh (376 / 1843 / 4577 / 8518 / 34020); cached = 0. "All moving" is the worst case (every
object changes): about the old cost, within noise of it on the larger scenes.
What remains in the camera-only number is sampling the object tree (tracks, TRS, parent multiply) and the per-slot frustum tests.

Settled world (`settled_world`, N loose props all promoted then asleep, N=1000): `PropWorld::step` 7.6 us vs 0.2 us untouched;
`awake_count` 3.4 us; per-client `props_to_send` 1.1 us (nothing unconfirmed) / 3.6 us (nothing known). N=4000: 4.4 us / 13.9 us per client.
Linear in promoted props but tiny; not changed.
