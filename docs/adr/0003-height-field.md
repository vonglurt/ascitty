# 0003 — A height field, not a set of boxes

**Status:** accepted

## Context

A city of towers could be stored as a list of boxes with positions and
extents, and rendered by intersecting rays against them. That is the obvious
model and it is what a raytracer would do.

## Decision

The world is a grid where **every cell carries one height**. A building is a
run of cells that happen to share a lot record.

## Consequences

**The renderer becomes a walk rather than an intersection test.** A height
field can be traversed front to back with a DDA in a single pass per column,
and the cost is proportional to the number of cells crossed rather than to
the number of buildings in the scene. There is no acceleration structure,
because there is nothing to accelerate.

**Setbacks are free.** A tower that steps inward as it rises is a lot whose
edge cells are shorter than its middle. No extra geometry, no per-height
footprint, and it reads correctly from every angle. The whole wedding-cake
silhouette of a 1920s tower is four lines in `cell_height`.

**Occlusion is one number per column.** The topmost screen row anything has
claimed. A far building draws only above it. There is no depth buffer and no
sorting.

**A sixty-storey tower costs the same to look at as a shed.** Nothing is
stored per floor; the facade is sampled from a hash when a ray lands on it.

### What it costs

**Nothing can pass under anything.** No bridges, no elevated track, no
expressway, no arcades, no tunnels. This is the real price and it is not
small. A second height field for "floor height" would express it and would
roughly double the walk's inner loop; it is in the backlog as **maybe**,
because it is only worth it if there is something to drive on up there.

**Buildings are axis-aligned and grid-quantised.** No diagonal streets, no
curved facades, no round towers. The grid is Manhattan, which is the city
this is meant to be, so the constraint and the subject agree.
