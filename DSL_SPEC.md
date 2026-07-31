# Strøk DSL Specification

The `.strok` file format is a human-readable text DSL for vector documents. It is the source of truth — the format agents and humans read, write, and edit directly. The CLI renders, previews, and inspects it but does not own the state.

## Design Principles

1. **Readable geometry.** Points are named. Curves are described by intent (catmull-rom, tension) not mechanism (bezier control handles). Coordinates are small numbers in a local space.
2. **Two concepts.** `shape` defines reusable geometry + style. `place` puts an instance in the scene at a position and size.
3. **Indentation-based structure.** Shape operations are indented under the `shape` line. Group children are indented under the `group` line. No braces.
4. **Progressive complexity.** Simple documents are simple. Advanced features (effects, modules, arcs) are available but never required.

---

## Document Structure

```
documentsize 600x800

shape bg template=rectangle
  fill #0d1117

shape face template=ellipse
  fill #f5c6a0

place bg shape=bg at=0,0 size=600x800
place face shape=face at=220,210 size=160x180
```

The `documentsize` line declares canvas dimensions. Everything else is either a shape definition or a scene node (place, group, createlink).

### Comments

```
# This is a line comment
```

Lines starting with `#` are comments. There are no block comments.

### Whitespace

Indentation (two spaces) determines nesting: operations inside shapes, children inside groups. Blank lines are ignored.

---

## Shapes

Shapes define reusable geometry and style. A shape has a name and a template.

```
shape <name> template=<template>
  <operations...>
```

### Templates

| Template | Default points | Closed | Description |
|---|---|---|---|
| `rectangle` | tl, tr, br, bl | yes | Four corners at (0,0), (w,0), (w,h), (0,h) |
| `ellipse` | top, right, bottom, left | yes | Cardinal points with catmull-rom for circle approximation |
| `triangle` | top, br, bl | yes | Three vertices |
| `line` | start, end | no | Two endpoints |
| `path` | (none) | no | Empty — you define all points with `addpoint` |
| `text` | (none) | no | Text element — use `content`, `font-size`, etc. |

### Geometry Operations

Operations are indented under the shape line:

```
shape stem template=path
  addpoint base at=200,385
  addpoint mid at=192,300 after=base mode=catmull-rom tension=0.3
  addpoint tip at=188,220 after=mid mode=catmull-rom tension=0.3
  stroke #3a7d44
  stroke-width 5
```

#### Adding points

```
addpoint <name> at=x,y [after=<point>] [mode=catmull-rom tension=N] [mode=arc rx=N ry=N (sweep=0|1|cw|ccw | bulge=left|right) large=0|1]
addpoint <name> at=x,y [after=<point>] [mode=controls c1=x,y c2=x,y]
addpoint <name> at=x,y [after=<point>] [mode=controls-relative c1=dx,dy c2=dx,dy]
```

- `after=<point>` — insert after this named point (default: append to end)
- Curve modes belong to segments: the mode on point B controls the segment arriving at B (A→B). It does not also change B→C.
- `mode=catmull-rom` — makes the segment from the previous point to this point part of a smooth spline. `tension` controls curvature (0 = smooth default, negative = more curved, positive = tighter). Smooth every consecutive segment endpoint in the intended run; a lone smooth point does not smooth both sides of that anchor. **Use catmull-rom only for organic curves** (leaves, fur, freehand strokes); for UI/geometric shapes (rounded rects, circles, arches, cylinders) use `round-corners`, `mode=arc`, or `mode=sharp` — catmull-rom threaded through near-collinear points produces wavy, faceted edges.
- `mode=arc` — SVG arc segment from previous point to this one. `rx`/`ry` are radii, `large` is the large-arc flag. Pick the bulge side with **`bulge=left|right`** (order-independent — relative to the direction of travel from the previous point, so reversing point order keeps the bulge on the same logical side) **or** the raw SVG `sweep` flag `sweep=0|1` (`cw`/`ccw` accepted as synonyms for `1`/`0`). If both are given, `bulge` wins. Omitting both is unchanged (`sweep` defaults to `1`).
- `mode=controls` — explicit cubic Bézier handles in document coordinates
- `mode=controls-relative` — explicit cubic Bézier handles as offsets from the point itself

Example:

```
addpoint start at=0,0
addpoint crest at=100,80 mode=controls c1=25,0 c2=110,60
addpoint tail at=160,40 mode=controls-relative c1=-30,20 c2=0,0
```

#### Moving points

```
movepoint <point> to=x,y              # absolute position
movepoint <point> dx=N dy=N           # relative delta
pullpoint <point> dir=up|down|left|right N%   # percentage of coord space
```

#### Curve mode

```
smooth <point> [tension=N]            # make the segment arriving here catmull-rom
smooth-corner <point> [tension=N]     # smooth both segments adjacent to this anchor
sharpen <point>                       # make the segment arriving here straight
smooth all [tension=N]                # set ALL points to catmull-rom
sharpen all                           # set ALL points to sharp corners
convert-point <point> to=<mode>       # convert one point's mode in place
```

`convert-point` (C4 / E2.5) changes the segment mode arriving at a single point
while keeping its position. `to=` is one of `sharp`, `smooth`, `arc`, or `controls`. Unlike
`smooth`/`sharpen` it also reaches `arc` (radius derived from the chord) and
explicit cubic `controls` (handles at 1/3 and 2/3 of the chord — a straight
default you then bend). A missing point is a no-op (never an error).

`smooth-corner` is the anchor-oriented form agents usually intend when they say
“smooth this corner.” Internally modes still belong to incoming segments, so the
operation sets the named point and the next point in the same contour to one
Catmull-Rom tension. Use plain `smooth` when controlling a specific segment.

The `smooth all` + `sharpen` pattern is useful for mostly-curved shapes:

```
shape drop template=path
  addpoint tip at=32,8
  addpoint p2 at=48,30
  addpoint p3 at=50,42
  addpoint p4 at=32,56
  addpoint p5 at=14,42
  addpoint p6 at=16,30
  close
  smooth all tension=0.3
  sharpen tip
```

#### Rounded corners

```
round-corners <radius>                 # uniform — every corner the same
round-corners tl=8 tr=8 br=0 bl=0      # per-corner, by point name (C5 / E2.6)
```

Replaces corner points with tangent circular-arc pairs, producing true-radius
fillets on any closed path with at least three points (including triangles,
shields, and logo polygons). The tangent distance is derived from the corner
angle; when adjacent edges are too short the radius shrinks to avoid overlap.

**The radius is measured in placed space:** when the shape is placed with a
`size=`, the corner still measures `<radius>` document units on the final
canvas (a circular corner, even under a non-uniform fit). It is *not* scaled by
`size / authored-bbox` — `round-corners 8` means 8px wherever the element lands.

```
shape badge template=rectangle
  round-corners 8
  fill #3b82f6
```

**Per-corner (C5 / E2.6):** give a radius per corner *by point name* — rectangles
use `tl`/`tr`/`br`/`bl`. A corner not listed (or `=0`) is left sharp. This makes
tabs / cards / corner-folds one op:

```
shape card template=rectangle
  round-corners tl=16 tr=16 br=0 bl=0   # rounded top, square bottom
  fill #c8863a
```

The uniform and per-corner forms each round-trip verbatim.

#### Notch / tail (C5 / E2.6)

```
notch edge=<top|bottom|left|right|p1,p2> dir=<out|in> shape=<square|triangle> pos=N width=N depth=N
```

A first-class primitive for folder tabs, speech-bubble tails, and corner-folds —
the rounded-rect-plus-separate-sharp-path boilerplate authors used to hand-compose
(Feedback #3). It inserts points along an **edge**:

- `edge=` — a rectangle edge (`top`/`bottom`/`left`/`right`), or `p1,p2` to span
  the segment between two named points on a `path`.
- `dir=out` pushes a protrusion outward (folder tab, bubble tail); `dir=in` cuts a
  bite inward (slot, fold). Default `out`.
- `shape=square` is a rectangular step; `shape=triangle` is a point. Default `square`.
- `pos` is the notch center along the edge, `0..1` from the edge's start (default `0.5`).
- `width` is the span along the edge; `depth` is perpendicular. **Both are measured
  in placed space** (like `round-corners`): when the shape is placed with a bbox-fit
  `size=`, `width`/`depth` still measure their authored value on the final canvas
  rather than being scaled by `size / authored-bbox` (EXP-5). At scale ≈ 1 (e.g. a
  24px icon) this is a no-op.

```
shape folder template=rectangle               # a folder tab in one op:
  notch edge=top dir=out shape=square pos=0.28 width=8 depth=3
  round-corners tl=2 tr=2 br=2 bl=2
  stroke currentColor

shape bubble template=rectangle               # a speech bubble, one op:
  notch edge=bottom dir=out shape=triangle pos=0.32 width=8 depth=8
  round-corners tl=6 tr=6 br=6 bl=6
  fill #3b82f6
```

Apply `notch` **before** `round-corners` when you want the corners rounded too
(the inserted notch points are not corner names, so per-corner rounding skips
them). A notch whose edge endpoints aren't found is a no-op (never an error).

#### Topology

```
close                                 # close the path (connect last→first)
open                                  # open a closed path
subpath                               # start a new closed contour at the next point
splitline <from>→<to> name=<name> [t=0.5]   # insert point on segment
deletepoint <point> [reconnect=join|gap]
```

`subpath` begins a fresh contour: the next `addpoint` starts a new `M` in the
emitted path. This lets one `path` shape carry **holes** (a ring = an outer
contour + an inner `subpath` contour, with `fill-rule even-odd`) and **disjoint
pieces**. It is the representation the boolean / `outline-stroke` / `offset` CLI
ops emit; you can also write it by hand. A `subpath` with no following point is a
no-op. (Single-contour paths never need it — omitting it is the default.)

#### Sculpting

```
sculpt at=x,y dx=N dy=N [radius=N] [falloff=N] [axis=x|y] [lock-endpoints]
sculpt at=<point> dx=N dy=N [radius=N] [falloff=N]
```

Pushes nearby points by (dx,dy) with distance-based falloff.

### Attributes

All shapes support these style operations:

```
fill #rrggbb | none | currentColor
fill radial(center, 80%, #d8b480, transparent)
fill linear(top, bottom, #ff0000, #0000ff)
fill-rule nonzero | even-odd          # interior rule for self-intersecting/holey paths
stroke #rrggbb | none | currentColor
stroke-width 4
stroke-linecap round | butt | square
stroke-linejoin miter | round | bevel
stroke-miterlimit 4                    # miter length limit (only with linejoin=miter)
stroke-dasharray 5 3                  # dash-gap pattern
opacity 0.8                           # 0–1
blur 3                                # gaussian blur radius
```

`fill-rule even-odd` is required for shapes with holes (e.g. a ring built from
two nested subpaths) and a prerequisite for the upcoming boolean operators. It
maps to SVG `fill-rule="evenodd"`; `nonzero` is the SVG default and may be
omitted. `stroke-miterlimit` only affects `stroke-linejoin miter` joins.

### Text Operations

For `template=text` shapes:

```
content "Hello world"
font-size 48
font-family "Georgia"
font-weight bold | normal | 100–900
font-style normal | italic
text-anchor start | middle | end
```

Text elements have an **estimated bounding box** (embedded Helvetica metrics,
scaled by `font-size`, `text-anchor`-aware): anchors (`at=label.right`),
`measure`, `query` and relative placement all work on text. The estimate is
accurate for the default sans stack and approximate for other faces — treat it
as layout-grade, not glyph-exact. Text flowed on a path (`textpath=`) has no
bbox. A plain `at=x,y` (no `align=`) positions the run by its **baseline start**.
For node labels and annotations, prefer intent-bearing placement such as
`at=node.center align=center`, `below=node gap=8`, or
`at=node.left align=right offset=-8,0`; these survive font and geometry edits
better than guessed baseline coordinates. `strok guide diagram` shows the full
authoring loop, and `strok audit` reports high-confidence partial text/shape
collisions with a concrete anchor rewrite.

**Relative placement works on text (EXP-5).** `at=<target>.<anchor>`,
`align=<self-anchor>`, `offset=dx,dy` and `below=/above=` behave the same as for
geometric places, measured against the estimated text box: `at=box.right` lands
the box flush against `box`'s right edge, `align=center` centers the text box on
the target point, `below=box gap=8` stacks the text under `box`. (Earlier
versions silently placed anchored text at `(0,0)` — that is now impossible; see
the no-silent-origin rule under *Placement*.)

### Effects

Non-destructive, render-time effects:

```
applyeffect droop 0.2 [direction=down]
applyeffect curl 0.3 [from=<point>]
applyeffect jitter 0.1 [seed=42]
applyeffect taper start=100% end=0%
```

---

## Scene Nodes

### Place

Places a shape instance in the scene:

```
place <name> shape=<ref> at=x,y [size=WxH] [rotation=45deg] [flip=x|y|xy] [skew=deg|degx,degy] [clip=<shape>[,<shape>…]] [mask=<shape>] [textpath=<place>]
```

Shapes render in file order — last placed = on top.

#### Text on a path (C5 / E2.7)

`textpath=<place>` flows a **text** shape's content along the geometry of another
placed shape, emitting `<textPath href="#…">`. The renderer and SVG export share
the referenced path's `d`. Only meaningful when `shape=` resolves to a `text`
shape; ignored otherwise.

```
place arch shape=arch at=0,0
place label shape=label at=0,0 textpath=arch   # "label" rides "arch"
```

#### Transforms (C4 / E2.3)

`rotation`, `flip`, and `skew` all flow through one unified 2-D affine. Translate
and scale (`at`/`size`) bake into the element's path `d`; rotation/skew ride as
the element's `transform` matrix, composed about the element's center. The placed
bounding box is **transform-aware** — a rotated or skewed element's bbox is the
AABB of its mapped corners, so anchors / relative placement land correctly on it.

- `skew=20` skews 20° on X; `skew=20,10` skews X and Y.

#### Clip & mask (C4 / E2.4)

- `clip=<shape>` clips the element to that shape's geometry (hard edge).
  `clip=a,b` clips to the **union** of multiple shapes.
- `mask=<shape>` applies an alpha/luminance mask: the masking shape is treated as
  white, so its luminance (and opacity) gates the element's alpha (soft edge).
  `clip` and `mask` may be combined. Both are available on `place` and `group`.

#### `at` and `size` semantics

- **With `size=WxH`**: bbox-fit. The shape's authored bounding box is scaled into the `W×H` region, with its top-left corner landing at `at`. Works the same for every template: the placed element occupies exactly the `(at, size)` rectangle.
- **Without `size=`**: translate-only. Scale is 1:1 and points render at their authored coordinates plus `at`. Useful when a path is authored in document coordinates (e.g. a bicycle wheel at `(277,380)..(357,540)` placed `at=0,0` renders in-place).

For path shapes with local-coordinate authoring (e.g. points in `0..10 × 0..10`), always use `size=` — `(at, size)` then fully determines where and how big the path renders.

If only a `stroke` is specified (no `fill`), the element is rendered hollow — fill defaults to `none` rather than SVG's implicit black.

#### Direct-geometry placement (sugar for `at`/`size`)

Three sugars desugar at parse time to plain `at` + `size` (they round-trip as
`at`/`size`; everything downstream is unchanged):

```
place slit shape=slit from=16,8 to=8,16       # a line by its two endpoints
place hub  shape=hub  center=12,12 radius=5   # center + radius (size = 2r x 2r)
place e    shape=e    center=12,12 size=8x4   # center + explicit size
place o    shape=o    at=2,2 radius=6,4       # radius alone also works with at=
```

- `from=x1,y1 to=x2,y2` — the placed shape's bbox is fitted from the first point
  to the second; direction is preserved (the desugared size may be negative or
  zero on an axis, e.g. a vertical line). Intended for `template=line` shapes but
  valid for any template. `from` and `to` must appear together and cannot be
  combined with `at`/`size`/`on`/`center`/`radius`/`align`/`below`/`above`.
- `center=cx,cy` — anchors the placement by its center. Needs an extent
  (`size=` or `radius=`). Cannot be combined with `at`/`on`/`align`/`below`/`above`.
- `radius=r` or `radius=rx,ry` — expresses the size as radii
  (`size=2rx x 2ry`). Cannot be combined with `size=`.

These replace the old hand-computed idioms (`place slit at=16,8 size=-8x8` for a
directed line; top-left arithmetic for circles) — those still work verbatim.

#### Relative placement

```
place <name> shape=<ref> at=<target>.<anchor> [align=<self-anchor>] [offset=dx,dy] [size=WxH]
```

Anchor points: `tl  top  tr  left  center  right  bl  bottom  br`

- `at=` picks a point on the target's bounding box
- `align=` picks which point on THIS shape goes there (default: tl)
- `offset=` nudges from the resolved position

```
place body shape=body at=seat.top align=bottom size=70x80
place beak shape=beak at=head.right align=left offset=0,5
place hub shape=hub at=wheel.center align=center size=16x16
```

**Anchors resolve top-to-bottom, and never fall back to the origin (EXP-5).** A
placement whose target does not exist is a positioned error that names the missing
target and suggests the closest element (`did you mean \`head\`?`). A target that
*does* exist but is declared **later** in the file is also an error — anchors are
resolved in document order, so the target must appear before the element that
anchors to it. (This holds for both geometric and `template=text` places; the old
silent `(0,0)` collapse is gone.)

#### Stacking anchors

```
place <name> shape=<ref> below=<other> [gap=5] [size=WxH]
place <name> shape=<ref> above=<other> [gap=5] [size=WxH]
```

#### Inline overrides

Attribute overrides indented under a place line:

```
place eye-dark shape=eye at=60,70 size=20x20
  fill #1a1a1a
  opacity 0.8
```

For `template=text` shapes, `content "…"` is also a valid override — one text
shape (one font/size/fill definition) can serve many labels:

```
shape pct template=text
  font-size 20
  fill #262019

place bar1-pct shape=pct at=760,200
  content "46%"
place bar2-pct shape=pct at=620,260
  content "31%"
```

### Group

Groups establish scope and can carry transforms:

```
group <name> [at=x,y] [rotation=Ndeg] [flip=x|y|xy] [skew=deg|degx,degy] [clip=<shape>[,<shape>…]] [mask=<shape>] [opacity=0.8]
  place ...
  group ...
```

Children are indented under the group line. Group transforms compose with the
child transforms (nested groups multiply through the unified affine), and a
group's `clip`/`mask` apply to all its children as a unit. Multi-shape `clip=a,b`
clips to the union; `mask=` is the soft alpha/luminance counterpart to `clip`.

### Createlink

Linked copy of a shape with its own attribute overrides:

```
createlink <name> from=<shape>
  fill #variant
  opacity 0.8
```

Geometry comes from the source shape; the link carries its own fill/stroke/opacity.

### Defaults

Inherited by all shapes that don't explicitly set the attribute:

```
defaults
  fill #2d5a1e
  stroke-width 1.5
```

---

## Colorschemes

Name colors once in a `palette`, then swap them per theme with `scheme` blocks.
Reference a token anywhere a color is accepted (`fill`/`stroke`) as `$name`.

```
palette
  hero #e8a840
  accent #c8863a

scheme dark
  hero #f4c266          # overrides; tokens not listed fall back to the palette

shape cup template=path
  fill $hero
  stroke $accent
```

Tokens resolve at render time, not in the file — the `.strok` keeps `$name`
intact. A `scheme` only needs to override the tokens that change; everything
else falls back to the base `palette`. Token values must be a hex color,
`currentColor`, or `none` (not gradients or other tokens).

Selecting a scheme is a render/inspect concern, not a file concern:

```
strok -f icon.strok render --out icon.png --scheme dark   # one theme
strok -f icon.strok render --out icon.png                 # base + every scheme,
                                                          # → icon.png, icon-dark.png
strok -f icon.strok inspect --svg --scheme dark           # resolved SVG for a theme
```

---

## `currentColor` — themeable fills/strokes

`fill` and `stroke` accept the CSS keyword **`currentColor`** (case-insensitive)
anywhere a hex color is accepted, including as a `palette` token value:

```
shape icon template=path
  fill none
  stroke currentColor
```

`currentColor` is kept **verbatim** in emitted SVG (`inspect --svg`, `export svg`),
so the element inherits the surrounding CSS `color` — the idiomatic way to let a
host stylesheet (e.g. a design-system token) drive an icon's color without editing
the file. Because a raster target has no inherited color, **PNG render substitutes
a concrete ink** for `currentColor`: pass `--color <hex>` (default black) to
`render`/`export png`/`batch`. This is the recommended coloring model for icon sets
— see also `strok guide icon`, the explicit `new --profile icon-*` choices, and
the `batch` command in `CLI_SPEC.md`.

## Computation: expressions, `let`, `repeat` (C13)

The DSL has a small **compile-time** computation layer: scalar arithmetic
expressions, named `let` bindings, and `repeat` blocks that expand into concrete
nodes. It is **additive and backwards-compatible** — every pre-C13 file parses,
renders, and round-trips unchanged, because a plain number is still a plain
number (the evaluator fast-paths it). Everything here happens at **parse time**:
the scene stores plain numbers, and (except `let`, which keeps its source for
round-trip) the expression text does not survive.

### Scalar expressions

Anywhere a scalar number is accepted on a scene-node or shape-op line you may
write an arithmetic expression:

```
expr   := term (('+' | '-') term)*
term   := factor (('*' | '/' | '%') factor)*
factor := '-' factor | '(' expr ')' | number | '$' name
```

Standard precedence (`* / %` bind tighter than `+ -`), left-associative, with
parentheses and unary minus. Expressions are accepted for **coordinates** (`at=`,
`from=`, `to=`, `center=`, `offset=`), **dimensions** (`size=`, `radius=`),
`rotation=`, `gap=`, `round-corners`, and `addpoint … at=`.

> **Expressions are SPACE-FREE.** The line tokenizer splits on spaces, so an
> expression must be a single token: write `40+$i*60`, not `40 + $i * 60`. A
> `deg` suffix is only valid after a *plain* number (`rotation=45deg`);
> expressions are unitless. A `size=` dimension expression must not reference a
> `$name` containing a literal `x` (the dimension separator) — use `$w*2` rather
> than a var spelled with an `x`.

```
place bar shape=bar at=310,190+$i*60 size=20x20
shape card template=rectangle
  round-corners $radius*2
```

`$name` resolves against the environment built from `let` bindings and `repeat`
loop variables. An undefined name is a positioned error that suggests the closest
known name; division or modulo by zero is an error.

### `let` bindings

A top-level `let <name> <expr>` binds a name to an `f64`:

```
let col 310
let inner $col+20        # later lets may reference earlier ones
```

Reference it as `$name` in any expression. A `let` name may **not** shadow a
`palette`/`tokens` entry of the same name (it would make `$name` ambiguous
between the number and the color token) — that is a positioned error. `let`
blocks round-trip verbatim: `emit` re-emits `let <name> <expr>` in declaration
order, before the shapes.

### `repeat` blocks

`repeat <var> <count>` expands its indented body `count` times. Allowed at the
top level and inside groups.

```
repeat i 4
  place dot shape=dot center=40+$i*60,40 radius=6
```

- `<count>` is an expression (evaluated with the `let` environment); it must be a
  non-negative integer ≤ 10000, else a positioned error.
- For `i` in `0..count`, the body is expanded with `$<var>` bound to `i`.
- Every `place` / `group` / `createlink` **name** defined directly or nested in
  the body gets `-<i>` appended (`dot` → `dot-0`, `dot-1`, …) so names stay
  unique. References inside the same iteration to a sibling name defined in the
  same repeat body (`at=dot.center`, `clip=`, `mask=`, `textpath=`,
  `below=`/`above=`) are rewritten to the suffixed name; references to names
  outside the body are untouched.
- Nested repeats append their index after the outer one (`dot-0`, then inner
  → `dot-0-0`, `dot-0-1`, …).
- A `repeat` var may not shadow a `let` name or an enclosing `repeat` var
  (positioned error).

**Flattening caveat:** expansion happens at parse time, so the scene contains
only the expanded nodes — a `repeat` block does **not** survive in the model. Any
CLI command that re-emits the file (`inspect`, boolean ops, `transform`, …)
writes the concrete expanded nodes, not the original `repeat`.

## Modules

Shapes can be imported from other `.strok` files:

```
use "./face.strok"
use "./face.strok" as face
```

Namespaced import — shapes referenced as `face.eye`, `face.nose`:

```
use "./components.strok" as ui
place btn shape=ui.button at=10,10 size=100x40
```

Module files contain shape definitions (and their own `use` imports). The importing file provides the scene structure.

**Round-trip note:** an imported shape is re-emitted as its `use` line, not
inlined — re-saving a document that imports a module does not duplicate the
module's shape definitions into the file.

### Standard library (EXP-1)

Strøk ships a small standard library of reusable shapes **embedded in the
`strok` binary itself** — no files on disk needed. Import a module with a
`std/` path:

```
use "std/figures" as fig
use "std/arrows" as arr

place p shape=fig.person-standing at=0,0 size=40x100
  fill #2d5a1e
place a shape=arr.arrow-right at=60,20 size=60x24
  fill #c8863a
```

`std/<module>` (with or without the `.strok` suffix, with or without a `./`
prefix) is intercepted before any filesystem lookup — it works from any
document, in any directory, with nothing installed. Omitting `as <ns>` puts
the module's shapes in the bare namespace (`shape=person-standing`).

Available modules (list with `strok lib list`, or explore interactively with
`strok lib show <module>` / `strok lib search <query>`):

| Module | Shapes |
|---|---|
| `std/figures` | `person-standing`, `person-pointing`, `person-sitting`, `person-walking`, `head-side` |
| `std/arrows` | `arrow-right`, `arrow-curved`, `arrow-double`, `chevron` |
| `std/bubbles` | `speech-bubble`, `thought-bubble`, `callout-rounded`, `banner-ribbon` |
| `std/devices` | `phone`, `laptop`, `browser-window`, `projector-screen` |
| `std/furniture` | `table`, `chair-side`, `plant`, `whiteboard`, `sticky-note` |

Every shape is geometry-only (no `fill`/`stroke` baked in) so a `place`
override decides the color — the same convention as any other imported shape.
Each is authored as ONE `template=path` shape using `subpath` for disjoint
parts (head vs. torso vs. limbs, table top vs. legs, …), because only shapes
(not groups or components) survive a cross-module `use` — see
`strok-core/src/dsl_parse.rs::resolve_imports`.

The library's source of truth lives in the repo at `std/<module>.strok` —
ordinary `.strok` files (parsed by the same parser as everything else) baked
into the binary via `include_str!` in `strok-core/src/stdlib.rs`. See
`examples/std-library.strok` for a curated sampler of the embedded modules.

---

## UX primitives: frames, layout, tokens, components (C8)

Above the geometry layer (shapes/places) the DSL has a UX layer for building
real UI: layout containers, generalized design tokens, and reusable components.
These are **additive and backwards-compatible** — every pre-C8 file parses,
renders, and round-trips unchanged. The UX layer is what `strok emit react|solid|
vanilla|tailwind|dtcg` turns into real component trees + token files (see
`CLI_SPEC.md`).

**Text as first-class UI (C9):** a `place` whose shape is `template=text` lowers
to a real, selectable, accessible DOM `<span>` carrying the text content — not
glyphs rasterized inside an SVG. Its `font-family`/`font-size`/`fill`/`text-anchor`
become `font-*`/`text-*` utility classes (symbolic where a `$font.body` token
exists). A `$font.<name>` reference also resolves to its token value in the
*rendered* SVG, so font tokens flow to both the code/token emit and the canvas.
The **`dtcg`** emit target writes the same generalized token system as a W3C DTCG
`design-tokens.json` (`{"$value", "$type"}` grouped by category) for interop with
Style Dictionary / Tokens Studio. The canonical end-to-end demo is
`examples/design-system.strok`.

### `tokens` — generalized design tokens

`palette` carries colors. `tokens` generalizes to every design-system category —
spacing, radius, type scale, shadows, motion — each referenceable as
`$category.name` (dotted spelling) anywhere a value is accepted:

```
tokens
  color.accent   #c8863a     # equivalent to a palette entry
  space.md       16
  radius.md      12
  radius.sm      6
  font.body      "IBM Plex Sans"
```

`palette` colors are *also* tokens, surfaced under the `color` category — so
`$copper` and `$color.copper` resolve to the same value. Tokens flow to the
Tailwind target's `@theme` (`--color-*`, `--spacing-*`, `--radius-*`, `--font-*`,
`--shadow-*`). A bare `name value` line (no dot) is filed under `color`.

The symmetric promise holds at render time: a color defined in a `tokens` block
(bare or `color.<name>`) resolves in `fill`/`stroke` exactly like a `palette`
entry — both the `$name` and `$color.name` spellings. On a name conflict the
`palette` definition wins (schemes only override palette entries).

### `frame` — a layout container

A frame is a clipping/layout box (a Figma frame / artboard) — distinct from
`group`, which is a transform-only geometry grouping. A frame carries a layout
policy and its own fill/radius/opacity, and lowers to a styled `<div>`:

```
frame card layout=flex(col, gap=12, padding=16) size=320x200 at=0,0
  fill $surface
  round-corners $radius.md
  place title shape=ui.heading at=0,0 size=200x20
  place body  shape=ui.paragraph at=0,30 size=200x40
```

Layout values:

```
layout=none
layout=flow
layout=flex(row|col [, gap=N] [, padding=N | x,y | t,r,b,l] [, align=start|center|end|stretch] [, justify=start|center|end|between])
layout=grid(columns=N [, gap=N])
```

A frame's children are `place` / `frame` / `group` / `instance` lines, indented.
Its attrs (`fill`, `round-corners` — a number or `$radius.*` token, `opacity`)
precede the children. In the SVG/PNG preview a frame renders as a `<g>` with a
background rect; in code emit the `layout=` becomes flex/grid CSS.

### `component` / `instance` — reusable UI

A `component` is a named, parameterizable subtree with declared variants and
props; an `instance` places one with overrides. Each component emits its own
component file (`Button.tsx`); instances become `<Button variant=… prop=… />`.

```
component button variants=[primary, ghost] props=[label:text]
  frame root layout=flex(row, gap=8, padding=10,16)
    fill $accent
    round-corners $radius.sm
    place icon shape=ui.dot at=0,0 size=10x10

instance cta from=button variant=primary label="Get started" at=20,160
```

- `variants=[a, b, …]` — declared variant names (first is the default).
- `props=[name:type, …]` — props that drive the emitted component's props.
- `instance <name> from=<component> [variant=v] [prop=value …] [at=x,y] [size=WxH]`.

> **Round-trip + eyes invariant (non-negotiable):** every UX construct survives
> `parse(emit(scene)) == scene` and renders to SVG/PNG, so an agent can always
> inspect it as an image. (Variant-scoped *style overrides* are reserved for a
> later slice; instances carry the variant as a prop today.)

---

## Full Example

```
documentsize 200x200

shape head template=ellipse
  fill #ffcc88

shape eye template=ellipse
  fill #333333

shape mouth template=path
  addpoint left at=0,0
  addpoint mid at=30,15 mode=catmull-rom
  addpoint right at=60,0 mode=catmull-rom
  stroke #a0522d
  stroke-width 3

shape badge template=rectangle
  round-corners 6
  fill #3b82f6
  stroke #2563eb
  stroke-width 1

place head shape=head at=0,0 size=200x200
place left-eye shape=eye at=60,70 size=20x20
place right-eye shape=eye at=120,70 size=20x20
place smile shape=mouth at=70,130 size=60x15
place nametag shape=badge at=50,170 size=100x24
```

---

## File Format Details

### Encoding

UTF-8 text. File extension: `.strok`.

### String Values

Attribute values containing spaces use double quotes:

```
font-family "Helvetica Neue"
content "Hello World"
```

Simple values (colors, numbers, unspaced names) are unquoted.

### Compilation

The CLI compiles `.strok` text to an internal representation for rendering. The text file is always the source of truth.

```
strok -f mascot.strok render --out mascot.png    # parse → render → PNG
strok -f mascot.strok export svg --out mascot.svg # parse → emit SVG
strok -f mascot.strok inspect                     # print DSL
strok -f mascot.strok inspect --svg               # print resolved SVG
```

### Diagnostics & error recovery (C6 / E3.1)

Parse errors are **positioned**: they report the line + column of the offending
token, draw a caret snippet under it, and — for a mistyped keyword, operation, or
attribute — suggest the closest valid spelling ("did you mean …?", by edit
distance). A bare positional `x,y` where a keyed `at=x,y` is required is called
out specifically. See `CLI_SPEC.md § DSL parse diagnostics` for examples.

The library also exposes an **error-recovering** parser
(`dsl_parse::parse_file_recover`): one malformed top-level block no longer aborts
the whole parse — the parser skips it, records a diagnostic, and keeps going,
returning a best-effort scene plus every diagnostic. The DSL itself is unchanged
and remains **additive / backwards-compatible**; diagnostics only improve the
*reporting* of invalid input.
