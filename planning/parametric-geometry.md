# Design note: parametric geometry

Status: draft
Scope: DSL + core resolve/emit; no renderer changes
Phases: 1 = parametric templates (this note's implementation target),
2 = stroke scaling policy, 3 = center-form arcs

## Motivation (field evidence)

Observed in a live human+agent review session (the "bolt" icon): decorating a
48x48 icon with a four-point sparkle and a spiral motion mark required

- a sparkle authored as eight hand-derived `addpoint` coordinates;
- a spiral authored twice: the first attempt used freehand cubics and read as
  scribble, the second chained three semicircle `mode=arc` segments whose
  radii and endpoints had to be hand-computed so the turns would not collide.

Nothing in the language sits between the fixed primitives
(rectangle/ellipse/triangle/line) and raw point-by-point paths. Every
"geometric but not a rectangle" motif is re-derived from coordinates, which
costs agents render round-trips per guess and invites the scribble failure
mode. Two adjacent weaknesses surfaced in the same session and are recorded
here as phases 2 and 3.

## Phase 1: parametric templates

Four new templates whose parameters live on the `shape` line and round-trip
verbatim:

```
shape hex   template=polygon sides=6
shape burst template=star points=4 inner=0.38
shape curl  template=spiral turns=2.5
shape gauge template=arc from=0 to=270
```

### Shared semantics

- Geometry is generated into the shape's authored coordinate space, inscribed
  in the box like `ellipse` (non-square boxes stretch elliptically).
- The result is ordinary named points, so every existing operation composes:
  `round-corners` fillets a polygon or star, `movepoint`/`sculpt` nudge
  individual vertices, `audit`/`query`/`measure` see real geometry, and the
  watch editor can offer the points like any path.
- Emit writes the template and its parameters back on the `shape` line,
  byte-stable; parse → emit → parse is the identity.

### `polygon sides=N`

Regular N-gon (N ≥ 3, default 6), first vertex at 12 o'clock, clockwise.
Point names `v1..vN`. Closed, sharp corners.

### `star points=K inner=F`

2K vertices alternating outer/inner radius (K ≥ 3, default 5; 0 < F < 1,
default 0.4). Starts with an outer vertex at 12 o'clock, clockwise. Point
names interleave `tip1, dip1, tip2, …`. Closed, sharp.

### `spiral turns=T`

Geometric spiral: a 3/4 outer sweep (top → bottom → left, clockwise) followed
by tangent-continuous semicircular half-turns along the horizontal midline,
radii shrinking linearly toward the center (T > 0, default 2, capped at 12,
rounded to the nearest constructible sweep). Point names `start, turn1, …,
end`. Open; authors give it a stroke.

Field-tested revision: the first construction used half-turns only, whose
bounding box never touched the top of the authored box; bbox-fit placement
then stretched square-placed spirals ~14% vertically ("the swirls look a
little elongated" — reviewer). Pre-compensating radii cannot fix this (the
authored aspect cancels out of the placed ratio), so the construction itself
must pin the bounding box to the authored box: hence the 3/4 outer sweep,
which touches all four edges with arcs whose aspect matches the box. The
general rule recorded for future templates: **a template's bounding box must
fill its authored box with curve aspect matching the box aspect**, or
bbox-fit placement will distort it.

### `arc from=A to=B`

An open elliptical arc segment: angles in degrees, 0° at 12 o'clock,
clockwise positive, drawn from A to B (span clamped to (0, 360)). Spans over
120° are split into equal `mode=arc` segments so each stays well-conditioned.
Point names `start, mid1, …, end`. Defaults `from=0 to=270` (a gauge sweep).

### Strictness change

The `shape` line today ignores unknown tokens, so `template=star insner=0.4`
would silently produce a default star. With parameters on the line this
becomes a real trap: unknown `key=value` tokens on a `shape` line become a
parse error with the standard caret diagnostic. (Alpha-stage break, judged
worth it; the parser has always rejected unknown *operations*.)

## Phase 2 (proposal): stroke scaling policy

Verified semantics today: placement bakes geometry into document space
(`size=` scales the path data) while `stroke-width` stays absolute in
document units. That is the right default for icon sets, but it is a fixed
policy: a stroked motif placed at several sizes keeps one absolute stroke
while its internal gaps scale, so small instances go optically heavy and
self-merge (the spiral "cinnamon bun" in the field session). The only
workaround is a hand-computed per-place `stroke-width` override — exactly the
one-off widths the icon guide forbids.

Proposal: an opt-in `scaled` keyword, `stroke-width 1.4 scaled` on the shape
(and as a place override), meaning the width scales with the placement's
bbox fit (geometric mean of sx, sy for anisotropic fits). Emits/round-trips
the keyword; default behavior unchanged.

## Phase 3 (proposal): center-form arcs

`mode=arc` is endpoint-parameterized (SVG heritage); predicting the bulge
side still takes a render or careful simulation even with `bulge=`.
Proposal: `mode=arc center=x,y` as an alternative parameterization — endpoint
plus center fully determine radius and sweep; `large=` selects the major
arc. Mutually exclusive with `rx/ry/bulge/sweep`.

## Out of scope

- A generic user-defined parametric/macro system (components already cover
  reuse; this note adds vocabulary, not a language).
- Donut/ring fills (two-contour subpaths already express them; revisit if
  demand shows up).
