# 2026-10-04. Plants as small parametric models, previewed in software
Status: accepted
Summary: Each species is a seeded, triangle-budgeted low-poly model built from tubes, lumpy blobs, cones and ribbons with vertex colours, and a software rasteriser draws a contact sheet so shapes are judged without a GPU.

## Context
An endless forest holds tens of thousands of plants; one draw call per object (the engine's rule today) is out of the question and loading modelled assets for twenty species is not how this engine works (ADR 0008: procedural assets only). The plants also need to look good: Marcel is meant to feel warm and storybook-like, and the only way to judge a shape here is to see it, on a machine with no display.

## Decision
`procgen::shapes::build(species, height, seed, tint)` returns a `Geo` (positions, normals, vertex colours, indices) for each of the 20 species, built from `procgen::geo` primitives: tapered tubes, lumpy ellipsoids with smooth normals (foliage shades like a cloud of leaves, not a crystal), cones, and double-sided ribbons for leaves, petals and blades (the scene pipeline culls back faces). A darker underside on every blob stands in for ambient occlusion. A seed varies lean, proportion and clumping; the same seed is always the same plant. Models are budgeted in triangles per kind (tree 560, shrub 260, flower 110, grass 40; tested for every species at both ends of its height range) because chunks hold thousands. Flowers grow as small clumps of two or three stems with exaggerated heads, so a field reads as a field at the placement density already chosen.

`red_engine2 flora sheet.png [--species oak --variants 6]` draws the models with a small software rasteriser (z-buffer, one light plus a sky hemisphere, 2x supersampling) and labels them with common name, Latin name, height and triangle count. It judges shape and colour, not the engine's lighting; the renderer shows the same meshes in the world.

## Consequences
Easier: shapes iterate in a fraction of a second with a picture, anywhere; the triangle budget and the winding are tests; a new species is one table row and one function. Harder: the look is hand-tuned, so "recognisable" is checked by eye, not by test (the tests check size, budget, colour family and determinism); budgets will need tuning against measured frame time once the chunk renderer exists (phase 4), and far plants will need a cheaper version. Undo: remove `shapes.rs`, `geo.rs` and `tools/flora_sheet.rs`.
