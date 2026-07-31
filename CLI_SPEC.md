# CLI Specification

Binary name: `strok` (alias: `strøk` where supported)

## Global Flags

```
strok -f <file> <command> [args...]
```

`-f <file>` — target document (`.strok` format). Required for file-bound
commands; `agent-intro`, `guide`, `new`, `import`, `lib`, and `mcp-server` do not
need it.

Every command reads the file, performs the operation, writes it back. No session state, no running process. The file is the single source of truth.

## Document Lifecycle

### `agent-intro`

Agents should start here before authoring:

```
strok agent-intro
```

The introduction treats Strøk as a visual construction and feedback system. It
defines sketch, production, and showcase effort levels; teaches command and
standard-library discovery; and gives a full-frame/region render loop with
explicit completion gates. The same introduction is exposed as the MCP
`agent_intro` tool.

### `guide`

Read the visual-quality workflow before authoring a key asset:

```
strok guide illustration
strok guide icon
strok guide logo
strok guide diagram
```

The guides make art direction explicit, steer geometric work toward the right
primitives, and require review at both shipping size and enlarged size. The same
guidance is exposed as the MCP `guide` tool so an agent can load it before
creating source.

The illustration guide teaches layered composition, semantic object
construction, material-specific geometry, lighting/depth, and high-resolution
focal-region review. It includes concrete checks for vessels, books, organic
forms, attachments, perspective, and edge quality.

The diagram guide additionally teaches relational label placement, the raw SVG
baseline semantics of plain text `at=x,y`, and the `audit`/`query --overlaps`
verification loop.

### `new`

Create a new document.

```
strok new <file> [<WxH>] [--profile <name>]
```

- Default canvas: 800x800 (pass a `WxH` dimension to change it, e.g. `strok new logo.strok 400x300`)
- Creates a `.strok` file with a `documentsize` line and an empty scene
- Icon profiles seed a 24×24 grid (or the explicit size), a 2px live-area
  convention, an explicit visual grammar, and the render/audit review loop:

  - `icon-outline-round` — no fill, 2px `currentColor`, round caps/joins.
  - `icon-outline-angular` — no fill, 2px `currentColor`, butt caps/miter joins.
  - `icon-solid` — `fill currentColor`, no stroke; silhouette-first.
  - `icon-mixed` — solid primary mass with sparse stroked secondary detail.

  `icon` remains a compatibility alias for `icon-outline-round`, but new work
  should choose an explicit profile after `strok guide icon` rather than treating
  rounded outline as the definition of an icon.

  ```
  strok new icons/src/close.strok --profile icon-outline-angular
  strok new icons/src/status.strok --profile icon-solid
  strok new icons/src/dot.strok 16x16 --profile icon-outline-round
  ```

### `import`

Convert an SVG file into an editable `.strok` document with **structure
recovery** (EXP-3) — not a dumb path dump. This is the bridge that lets
image-generation → vectorization pipelines land in Strøk as editable, semantic
documents. No `-f` is needed (like `new`).

```
strok import <input.svg> --out <file.strok> [--json]
```

What is recovered:

- **Native templates:** `<rect>` → `rectangle` (`rx`/`ry` → `round-corners`),
  `<circle>`/`<ellipse>` → `ellipse`, `<line>` → `line` (placed `from=/to=`), all
  with readable `at`/`size` placement.
- **Paths:** `<path>`/`<polygon>`/`<polyline>` → a `path` shape; the `d` is
  decomposed into `addpoint` ops (lines → sharp, cubics/quadratics → `mode=controls`,
  arcs → `mode=arc`). Quadratics are converted to cubics; unrepresentable pieces are
  flattened with a warning.
- **Reuse:** identical geometry + style used in several places collapses to ONE
  shape definition with multiple `place`s.
- **Palette:** every fill/stroke color used ≥ 2× becomes a `palette` token named by
  hue (`blue-1`, `red-1`, …); single-use colors stay inline.
- **Grouping:** `<g>` → `group` (name from `id`/`class`, else `group-N`); trivial
  single-child untransformed groups are flattened. Style inherits down the tree.
- **Transforms:** `translate`/`scale` are recovered into `at`/`size`;
  `rotate`/`skew`/`matrix` shear is **baked into path geometry** (with a warning) so
  the render stays faithful.

What degrades (each emits a warning): rotation/shear transforms (baked); gradients
(flat first-stop approximation — the DSL gradient forms are not reconstructed);
`<filter>`, `<mask>`, `<clipPath>`, `<use>`, `<image>` (skipped); text metrics
(estimated). Malformed SVG never fails the command — it yields warnings plus a
best-effort document.

`--json` reports import stats: per-kind element counts, extracted tokens, and the
warnings (with source lines where known), via the shared `strok-core::json` builder.

```
strok import logo.svg --out logo.strok
strok import icon.svg --out icon.strok --json
```

### `open`

Planned alias for `import` (SVG → `.strok`). Use `import` today.

```
strok open <input.svg> --out <file.strok>   # not yet implemented — see `import`
```

### `save`

Export the document back to its native format (no-op conceptually — the file is always saved after each command). Mainly useful for explicit "save as":

```
strok -f logo.strok save --out logo-copy.strok
```

---

## Element Creation

### `append`

Add an element as the **last child** of a parent (renders on top of siblings).

```
strok -f <file> append <parent-id> '<svg-snippet>'
```

Example:
```
strok -f logo.strok append root '<rect id="bg" width="400" height="300" fill="#1a1a2e" rx="8" />'
strok -f logo.strok append card '<text x="20" y="40" font-size="18">Title</text>'
```

- `root` is always a valid parent, referring to the document root
- If the SVG snippet includes an `id` attribute, it is honored as the element's ID
- If the `id` already exists, the command fails with an error showing the conflicting element
- If no `id` is provided, an 8-character random alphanumeric ID is generated
- The snippet can be a single element or a `<g>` group containing a subtree
- Reads from `--stdin` if the snippet argument is `-`

### `prepend`

Add an element as the **first child** of a parent (renders behind siblings).

```
strok -f <file> prepend <parent-id> '<svg-snippet>'
```

Same behavior as `append`, but inserts at position 0 in the child list.

### `insert`

Add an element at a specific position among a parent's children.

```
strok -f <file> insert <parent-id> <index> '<svg-snippet>'
```

- Index is 0-based
- Equivalent to `prepend` at index 0, `append` at index -1 or end

---

## Element Modification

### `set`

Modify properties of an existing element.

```
strok -f <file> set <id> [--fill <color>] [--stroke <color>] [--stroke-width <n>]
                         [--stroke-linecap round|butt|square]
                         [--stroke-linejoin miter|round|bevel]
                         [--stroke-dasharray <n> <n>...]
                         [--opacity <0-1>] [--blur <n>]
                         [--font-size <n>] [--font-family <name>]
                         [--font-weight <n>] [--font-style <s>]
                         [--text-anchor start|middle|end]
                         [--rx <n>] [--ry <n>]
                         [--width <n>] [--height <n>]
                         [--<any-svg-attr> <value>]
```

- Flag names mirror SVG attribute names
- Multiple flags in one call: `strok -f f.strok set bg --fill "#ff0000" --opacity 0.5`
- Unknown flags are passed through as custom attributes (extensible without updating the CLI)

### `move-point`

Move one named point inside a DSL-defined `path` by a relative offset.

```
strok -f <file> move-point <path> <point> <dx> <dy>
```

- Works only for paths that have DSL point data (`point ...` blocks)
- Fails for raw imported SVG `d="..."` paths
- Supports negative deltas directly, e.g. `-30 20`
- Alias for: `point move <path> <point> <dx> <dy>`

### `point`

Point editing command group for DSL-defined paths.

```
strok -f <file> point list <path> [--json]
strok -f <file> point move <path> <point> <dx> <dy> [--handles preserve|free]
strok -f <file> point insert <path> --after <point> --name <point> --at <x,y> [--mode sharp|catmull-rom|arc] [--tension <n>] [--rx <n>] [--ry <n>] [--sweep 0|1] [--large 0|1]
strok -f <file> point delete <path> <point> [--reconnect auto|line|smooth]
strok -f <file> point rename <path> <old> <new>
strok -f <file> point mode <path> <point> sharp|catmull-rom [<tension>]
strok -f <file> point mode <path> <point> controls <c1x,c1y> <c2x,c2y>
strok -f <file> point mode <path> <point> controls-relative <c1x,c1y> <c2x,c2y>
strok -f <file> point mode <path> <point> arc <rx> [<ry>]
strok -f <file> point split <path> <from> <to> --name <point> [--t <0..1>]
strok -f <file> point pull <path> <point> <dx> <dy> [--radius <n>] [--falloff <n>] [--handles preserve|free]
strok -f <file> point sculpt <path> [--at <x,y> | --at-point <point> | --at-segment <from> <to> [--t <0..1>]] <dx> <dy> [--axis xy|tangent|normal] [--lock-endpoints] [--radius <n>] [--falloff <n>] [--handles preserve|free]
```

### `path`

Path-level generation commands.

```
strok -f <file> path branch <path> <from-point> --id <new-id> --length <n> [--angle <deg>] [--bend <n>] [--stroke-width <n>]
strok -f <file> path sprout <path> <from-point> --id-prefix <prefix> [--count <n>] --length <n> [--angle <deg>] [--spread <deg>] [--bend <n>] [--jitter <0..1>] [--stroke-width <n>]
```

### `replace`

Replace an element entirely with new SVG. The new element occupies the same position in the tree.

```
strok -f <file> replace <id> '<svg-snippet>'
```

- The old element's tree position (parent + index) is preserved
- The new snippet's `id` replaces the old `id` (or inherits it if not specified)

### `remove`

Remove an element and all its children.

```
strok -f <file> remove <id>
```

### `rename`

Change an element's ID.

```
strok -f <file> rename <old-id> <new-id>
```

- Fails if `new-id` already exists

---

## Transform Convenience Commands

These are sugar over SVG transform attributes. They do the right thing without requiring the agent to reason about transform matrices.

### `move`

Translate an element by a relative offset.

```
strok -f <file> move <id> <dx> <dy>
```

### `moveto`

Move an element to an absolute position (sets the element's x/y or adjusts transform to place the bounding box origin at the given coordinates).

```
strok -f <file> moveto <id> <x> <y>
```

### `resize`

Set the width and height of an element.

```
strok -f <file> resize <id> <width> <height>
```

### `rotate`

Rotate an element around its center.

```
strok -f <file> rotate <id> <degrees>
```

- Rotates around the element's bounding box center by default
- `--cx <x> --cy <y>` to specify a custom pivot point

### `scale`

Scale an element by a factor.

```
strok -f <file> scale <id> <factor>
```

- Uniform scale. For non-uniform: `--sx <x> --sy <y>`
- Scales around the element's bounding box center

---

## Structure Commands

### `group`

Wrap one or more elements in a new group.

```
strok -f <file> group <id1> <id2> [<id3>...] [--id <group-id>]
```

- Elements must share the same parent
- The group takes the position of the first element listed
- If `--id` is not provided, a random ID is generated

In the DSL, groups support transform attributes:

```
group <name> [at=x,y] [rotation=Ndeg] [flip=x|y|xy] [clip=<shape>] [opacity=0.8]
  place ...
```

- `at=` translates the entire group (children use local coordinates)
- `rotation=` rotates the group around its origin
- `flip=` mirrors the group (x, y, or xy)

### `ungroup`

Dissolve a group. Children move to the group's parent, preserving order.

```
strok -f <file> ungroup <group-id>
```

### `reorder`

Move an element to a different position within its parent's children.

```
strok -f <file> reorder <id> <index>
```

- `0` = bottom (renders first / behind), `-1` = top (renders last / in front)
- Shorthand: `strok -f f.strok reorder bg --front` / `--back`

### `reparent`

Move an element to a different parent.

```
strok -f <file> reparent <id> <new-parent-id> [--index <n>]
```

- Defaults to appending as the last child of the new parent

---

## Vector Operations

Boolean operations, stroke-to-outline conversion, and path offsetting.
Each takes one or more **placed-element names** (the id you gave `place`), so the
geometry is read exactly where it renders (document space), and writes a new
filled `path` **shape + place** to the file (placed `at=0,0`, identity). The
result is an ordinary path shape — it round-trips, renders, and re-edits like
hand-authored geometry. Self-intersecting / holey inputs are interpreted with
each input's `fill-rule`; a holey result is tagged `fill-rule even-odd`
automatically. Geometry library: `i_overlay` (boolean core) + `kurbo`
(flattening, stroke expansion, and offsetting).

### `bool`

Combine two or more placed shapes with a boolean operation.

```
strok -f <file> bool <union|subtract|intersect|exclude> <id> <id…> [--out <name>]
```

- `union` — area covered by any input
- `subtract` — first input minus the rest (knockouts, folder tabs)
- `intersect` — area covered by all inputs
- `exclude` — symmetric difference (XOR)
- `--out` names the result shape + place (default `<op>-result`)

```
strok -f icon.strok bool subtract folder tab --out folder-cut
strok -f logo.strok bool union ring-a ring-b ring-c --out rings
```

### `outline-stroke`

Convert a placed stroked path into the filled region the stroke paints, honoring
its width / linecap / linejoin / miterlimit.

```
strok -f <file> outline-stroke <id> [--out <name>]
```

- `--out` default `<id>-outline`. Errors if the input has no positive stroke-width.

```
strok -f icon.strok outline-stroke wire --out wire-filled
```

### `offset`

Grow (positive delta) or shrink (negative delta) a placed shape's filled region
by `delta` document units. Corners are rounded (Minkowski offset of a disk):
offsetting a circle by r yields a concentric circle of radius ±r.

```
strok -f <file> offset <id> <delta> [--out <name>]
```

- `--out` default `<id>-offset`.

```
strok -f badge.strok offset card 4 --out card-grown
strok -f ring.strok offset disc -3 --out disc-inset
```

### `transform`

(C4 / E2.3) Apply a 2-D affine transform to an existing placed element in place.
`rotate`/`scale`/`skew`/`flip` all run through one unified affine, so they compose
predictably and keep the placed bbox correct under rotation/skew.

```
strok -f <file> transform <name> [--rotate <deg>] [--scale <factor>] [--skew <degx[,degy]>] [--flip <x|y|xy>]
```

- `--rotate` and `--skew` accumulate onto any existing rotation/skew.
- `--scale` multiplies the element's `size=` (requires an explicit size).
- The whole document is re-emitted from the parsed scene, so the result is
  guaranteed to round-trip.

```
strok -f icon.strok transform badge --rotate 15
strok -f icon.strok transform card --skew 12,0
strok -f logo.strok transform arrow --flip x
```

### `convert-point`

(C4 / E2.5) Convert a single path point's anchor type in place, preserving its
position. `shape.point` form; `--to` is `sharp|smooth|arc|controls`.

```
strok -f <file> convert-point <shape.point> --to <sharp|smooth|arc|controls>
```

```
strok -f leaf.strok convert-point leaf.tip --to sharp
strok -f bubble.strok convert-point bubble.p2 --to arc
```

### `text-on-path`

(C5 / E2.7) Flow a text string along a placed path, emitting `<textPath>`. Creates
a `text` shape with your content and a `place` referencing the path via `textpath=`.
`--path` is the **place name** of the path to flow along.

```
strok -f <file> text-on-path "<text>" --path <place> [--name <id>] [--size <px>] [--fill <color>]
```

```
strok -f badge.strok text-on-path "MEMBER SINCE 2024" --path ring --name label
```

### `measure`

(C5 / E2.7) Report the spatial relationship between two placed elements, using the
same bbox machinery as anchors/relative placement. Read-only.

```
strok -f <file> measure <a> <b> [--json]
```

Reports center-to-center distance, signed center delta, per-axis gap (negative =
overlap depth), an overlap flag, and per-edge alignment deltas (`b - a`; 0 ⇒ that
edge is aligned). `--json` prints a **stable, flat object** of named scalar fields.
As of C6 (E3.2) the JSON is built with the shared `strok-core::json` helper that
backs every `--json` surface (`inspect`/`query`/`relate`/`measure`) — the C5 seam
is now *generalized*, and the `measure` schema is unchanged (backwards-compatible).

```
strok -f card.strok measure title body
strok -f card.strok measure title body --json
```

### `snap`

(C5 / E2.7) Snap a placed element's absolute `at=` to a grid / edge / center.

```
strok -f <file> snap <name> <grid|edge|center> [--step <n>]
```

- `grid` rounds each axis to the nearest multiple of `--step` (default 8).
- `edge` snaps each axis to the nearer document edge (0 or width/height).
- `center` snaps to the document center.
- Relative / parametric placements are left untouched (with a note).

```
strok -f ui.strok snap button grid --step 8
strok -f ui.strok snap logo center
```

---

## Inspection

### `inspect`

Read the document, an element, or a **structural snapshot** (C6 / E3.2).

```
strok -f <file> inspect [<id>] [--svg] [--detail full|structural|summary] [--json]
```

- No selector / no flags: prints the file as-is.
- `<id>`: prints just that shape or place block.
- `--svg`: emits the resolved SVG (the whole doc, or the named element).
- `--detail <level>` (C6): a structural snapshot of the document —
  - `full` — the whole resolved SVG;
  - `structural` — element names, kinds and bboxes, **no path geometry**;
  - `summary` — just IDs + types (the lightest snapshot).
- `--json` (C6): a machine-readable snapshot (implies a `structural` snapshot if
  `--detail` is omitted). One **stable, snapshot-tested schema** shared with
  `query`/`relate`/`measure`. Shape:

  ```json
  {
    "detail": "structural",
    "document": { "width": 200, "height": 100 },
    "count": 3,
    "elements": [
      { "name": "card", "kind": "rectangle", "shape": "box",
        "bbox": { "x": 10, "y": 10, "w": 80, "h": 40 } }
    ]
  }
  ```

  `--detail full --json` additionally carries an `"svg"` field; `--detail summary`
  drops `shape`/`bbox`. A `text` element whose bbox isn't computed reports
  `"bbox": null` (consistent with `measure`, which also needs a concrete bbox).

```
strok -f logo.strok inspect --detail structural
strok -f logo.strok inspect --json
strok -f logo.strok inspect --detail summary --json
```

### `query` (C6 / E3.2)

Ask "what's in this region?" / "what overlaps element X?" using the same bbox
machinery the anchor resolver uses. Read-only.

```
strok -f <file> query --box <x,y,w,h> [--json]
strok -f <file> query --overlaps <id> [--json]
```

- `--box x,y,w,h` — every element whose bbox intersects the rectangle.
- `--overlaps <id>` — every *other* element whose bbox intersects `<id>`'s.
- `--json` prints `{ "query", "count", "matches": [ <element> … ] }` (each match
  is a `structural`-detail element object — same schema as `inspect`).

```
strok -f ui.strok query --box 0,0,100,50
strok -f ui.strok query --overlaps button --json
```

### `relate` (C6 / E3.2)

Describe the qualitative spatial relation between two placed elements: horizontal
(`left-of`/`right-of`/`x-overlap`), vertical (`above`/`below`/`y-overlap`), an
overlap flag, containment (`a-contains-b`/`b-contains-a`/`none`), and which edges
line up. Built on the `measure` deltas. Read-only.

```
strok -f <file> relate <a> <b> [--json]
```

JSON shape:

```json
{
  "a": "card", "b": "badge",
  "horizontal": "right-of", "vertical": "y-overlap",
  "overlaps": false, "containment": "none",
  "aligned_edges": ["top"], "gap_x": 30, "gap_y": -40
}
```

```
strok -f card.strok relate title body
strok -f card.strok relate icon label --json
```

### `layers`

Print the document's layer/element tree.

```
strok -f <file> layers
```

Output:
```
root
├── bg (rect)
├── coach (group)
│   ├── coach-backing (path)
│   ├── body (path)
│   └── whistle (ellipse)
├── ball (group)
└── text-ring (group)
    ├── title (text)
    └── subtitle (text)
```

- Shows element ID, type, and tree structure
- `--verbose`: also shows bounding box and key properties per element

### `info`

Get computed properties of a specific element.

```
strok -f <file> info <id>
```

Output:
```
id:          coach-backing
type:        path
parent:      coach
index:       0
bbox:        x=280 y=200 w=240 h=350
fill:        #f4d35e
opacity:     0.9
transform:   none
children:    0
```

### `points`

Print the point table for a DSL-defined path.

```
strok -f <file> points <path>
```

### `audit`

Analyze a file for patterns that could be simplified, and flag geometry traps.
Read-only; never modifies the file. Detections:

- **near_mirror** — a `-l`/`-r` (or `-left`/`-right`) shape pair that is
  approximately x-mirrored (define once, place with `flip=x`).
- **unused_composition** — `flip=x` / `createlink` available but unused where the
  pattern applies.
- **rough_catmull** — a `catmull-rom` run threaded through near-collinear points
  (the wavy/faceted-edge trap, feedback #2).
- **isolated_catmull** — multiple disconnected `catmull-rom` segments in a
  closed contour; usually a mistaken attempt to smooth symmetric anchors on
  both sides even though a point's mode controls only the segment arriving at it.
  Suggests the anchor-oriented `smooth-corner` operation.
- **text_collision** — a text run partially intersects a closed shape. Text fully
  contained by a rectangle/ellipse/triangle and labels owned by a smaller
  foreground badge are treated as intentional; paths and lines are ignored
  because their bounding boxes are too coarse. The suggestion gives a
  relative-anchor rewrite that clears the neighboring shape.
- **unanchored_label** — one raw-baseline text run appears centered inside its
  smallest containing shape. Multi-line hosts stay silent; single-label nodes get
  the concrete `at=<host>.center align=center` rewrite.

**Compositional findings (EXP-2) — the tool teaching its own idioms.** These
detect a *missed abstraction* and print the concrete rewrite, so an agent
discovers `repeat` / `let` / relative anchors from the audit output itself:

- **repeated_place_rhythm** — N≥3 places of the same shape forming an arithmetic
  progression on one axis (equal or arithmetic sizes) → the exact `repeat i N`
  block (C13 syntax) that replaces them.
- **near_duplicate_groups** — ≥2 groups with structurally identical children
  differing only in position/fill → `repeat` (if their positions form a rhythm)
  or define-once (`component` / a reused `group`), with a sketch of the shared body.
- **magic_number_rhythm** — the same value (|v|≥4) used ≥4× as one exclusive size
  dimension in a layout document with no `let` bindings → `let col-w`/`let row-h`.
- **unanchored_adjacency** — two absolutely-`at=`-placed elements that touch *and*
  share a flush edge → the `at=<target>.<anchor> align=… offset=…` rewrite (top 5).

  `magic_number_rhythm` and `unanchored_adjacency` are **gated to layout-shaped
  documents** (those using `tokens`, `component`s, or a `frame`) — on a freehand
  illustration those suggestions are noise, so they stay silent there.

Every finding carries a concrete, copy-pasteable **`suggestion`** (C10 / E5.3):
the exact `place … flip=x` line for a mirror, the primitive to swap to for a
rough catmull-rom run (`mode=sharp` / `mode=arc` / `round-corners tl=/tr=/br=/bl=`
/ `notch`), or the `repeat` / `let` / anchor rewrite for the compositional findings.

```
strok -f <file> audit [--json]
```

- `--json` emits the stable schema (deferred to here from C6 / E3.2):

  ```json
  {
    "count": 2,
    "total_line_savings": 9,
    "findings": [
      { "kind": "near_mirror", "message": "…", "detail": "…",
        "suggestion": "place eye-r shape=eye-l flip=x", "line_savings": 5 }
    ]
  }
  ```

```
strok -f portrait.strok audit
strok -f portrait.strok audit --json
```

---

## Rendering

### `render`

Rasterize the document to PNG.

```
strok -f <file> render [--out <file.png>] [--width <px>] [--height <px>]
                       [--region <x,y,w,h>] [--node <name>] [--annotate]
                       [--outline [<id1,id2>]]
                       [--color <hex>] [--bg <color>] [--scheme <name>]
```

- Defaults to document dimensions if no width/height is specified
- If only one dimension is specified, the other is inferred from the document
  aspect ratio; specify both to stretch intentionally
- If `--out` is omitted, writes to stdout (for piping)
- `--node <name>`: render only a named place or shape on an otherwise empty canvas
- `--region <x,y,w,h>`: render a document-space crop. The output aspect ratio is
  inferred from the region when only one output dimension is supplied. This is
  the preferred way to inspect fine geometry, material edges, and focal objects
  at high resolution:

  ```
  strok -f scene.strok render --region 450,340,220,180 --width 1200 --out /tmp/cup.png
  ```
- `--annotate` (C6 / E3.2): overlay each element's ID on the canvas (a top-left
  label per place/group, white-haloed for legibility) so an agent can map the
  rendered pixels back to the names it can reference. The underlying geometry is
  byte-identical to a normal render — only an additive `<g id="strok-annotations">`
  overlay is appended. Requires a v3 scene document.
- `--outline`: overlay every named placed element's exact resolved geometry
  above the normal scene. `--outline id1,id2` limits the overlay to the named
  placed elements. The black-haloed white stroke preserves placement sizing,
  flips, rotations, enclosing group transforms, clips, masks, and text layout;
  it does not alter DSL styling or replace the painted scene. Its stroke does not
  scale when used with `--region`, making it useful for inspecting silhouettes,
  Bézier closure, joins, and part attachments at high resolution. Unknown IDs,
  an explicit empty value, and IDs not present in a `--node` render are errors.
  Requires a v3 scene document.

  ```
  strok -f scene.strok render --outline --out /tmp/all-geometry.png
  strok -f scene.strok render --outline cup-body,cup-rim \
    --region 450,340,220,180 --width 1200 --out /tmp/cup-geometry.png
  ```
- `--bg <color>`: background color (default: transparent)
- `--color <hex>`: concrete ink to substitute for `currentColor` when rasterizing
  (default black). Preview a themeable icon light-on-dark with
  `--color '#e6e6e6' --bg '#0d1117'`.
- `--scheme <name>`: resolve `$token` colors with the named colorscheme (see DSL_SPEC § Colorschemes)
- With `--out` set and `--scheme` omitted, renders the base palette to `--out` plus every defined scheme to `<out>-<scheme>.png`

### `watch`

Live-preview a document in the browser while editing it — the human-facing
complement to the agent render loop.

```
strok watch <file> [--port <port>] [--scheme <name>] [--no-open]
strok -f <file> watch
```

Starts a local HTTP server bound to `127.0.0.1`, opens the preview page in the
default browser, and re-renders the resolved SVG on every save (mtime + content
polling, so rename-style editor saves work). The page updates over Server-Sent
Events with no manual refresh.

- Parse errors are shown in the page with their caret diagnostics while the
  **last good render stays visible** (dimmed), so the preview never goes blank
  mid-edit.
- **Edit shape** opens a focused geometry view for any local named shape. Drag
  anchors or explicit Bézier handles, use the midpoint `+` controls to split a
  segment, and remove the selected anchor or retract the selected control with
  the button or Delete/Backspace. Alignment guides snap anchors to peer x/y
  coordinates. Bézier handles are linked across their anchor by default:
  Alt-drag moves only one handle, Shift-drag constrains its direction to 45°,
  and Shift+C creates or resets the selected anchor's pair to equal, opposite
  handles. Undo/redo buttons and Cmd/Ctrl+Z / Cmd/Ctrl+Shift+Z restore browser
  edits (up to 100 steps); editing the file externally clears incompatible
  history. Edits are immediately written back to the watched `.strok` file;
  imported module shapes remain read-only and are not offered in the editor.
- The preview backdrop cycles checkerboard → white → black to judge
  transparency and both polarities.
- `--port` defaults to an ephemeral free port; the chosen URL is printed on
  startup.
- `--scheme <name>`: resolve `$token` colors with the named colorscheme.
- `--no-open`: don't launch the browser (print the URL only).
- This is the only long-running command besides `mcp-server`; stop it with
  Ctrl-C. The file on disk remains the single source of truth. Browser edits
  round-trip through the same parser and canonical DSL emitter used by CLI
  mutations.

### `batch`

Render **every `*.strok` in a directory** to SVG and/or PNG — built for icon sets.
No `-f`; the directory is the unit of work.

```
strok batch <dir> [--out <dir>] [--svg] [--png] [--sizes <list>] [--color <hex>] [--bg <color>] [--scheme <name>]
                  [--sprite <file.svg>] [--sheet <file.png>] [--manifest <file.json>] [--columns <n>]
```

- `--out <dir>`: output directory (default `<dir>/dist`); created if missing
- `--svg` / `--png`: which formats to emit. **Default is both**; pass one flag to
  restrict to it.
- `--sizes <list>`: comma-separated square PNG sizes, e.g. `16,24,32` (default `24`)
- `--color <hex>`: ink for `currentColor` in PNGs (default black); SVGs keep
  `currentColor` verbatim so a stylesheet can theme them
- `--bg <color>`: PNG background (default transparent)
- `--scheme <name>`: resolve `$token` colors with the named colorscheme

Output naming: `<name>.svg`, and `<name>-<size>.png` per size (or `<name>.png` when
a single size is given).

After rendering, `batch` prints the set's coarse visual-grammar distribution
(`outline-round`, `outline-angular`, `solid`, `mixed`, etc.). This is descriptive,
not a failure: consistency within a set is useful, while the summary makes an
accidental “everything is rounded outline” decision visible to the authoring agent.

```
strok batch icons/src --out icons/dist --sizes 16,24,32
strok batch icons/src --png --color '#e6e6e6' --bg '#0d1117'   # dark-theme PNGs only
strok batch icons/src --svg                                    # themeable SVGs only
```

#### Design-system outputs (C10 / E5.1, E5.2)

All driven off the **same parse** as the per-icon files, so they can never drift.
Each is optional and writes to an arbitrary path (parent dir created if missing):

- `--sprite <file.svg>`: a `<symbol>` **sprite sheet**. Each icon becomes
  `<symbol id="<name>" viewBox="…">` carrying the icon's inner markup;
  `currentColor` is preserved (icons stay themeable). The host references one
  symbol with `<use href="sprite.svg#<name>"/>`. Symbols are emitted in sorted
  (file-stem) order.
- `--sheet <file.png>`: a **contact sheet** — every icon laid out on a grid in
  one PNG (no external `montage` needed). `--columns <n>` sets the grid width
  (default 8); tiles render at the largest `--sizes` value and are centered in
  each cell. `--bg`/`--color` apply.
- `--manifest <file.json>`: an icon-set **manifest** (`name` → `meaning`, `tags`,
  `canvas`, `sizes`). Meaning and tags are authored as **leading comment
  annotations** in each icon file — additive, so existing files stay valid:

  ```
  # @meaning Close or dismiss the current view
  # @tags close, dismiss, x
  documentsize 24x24
  …
  ```

  Schema (version 1):

  ```json
  {
    "version": 1,
    "count": 2,
    "icons": [
      { "name": "close", "meaning": "…", "tags": ["close","x"],
        "canvas": { "w": 24, "h": 24 }, "sizes": [16, 24] }
    ]
  }
  ```

```
strok batch icons/src --sprite dist/sprite.svg --manifest dist/manifest.json
strok batch icons/src --sheet dist/contact.png --columns 8 --color '#e6e6e6'
```

### `token-sync`

Cross-check an icon set's `$token` references against a design-token system —
the design-system half of `$token` resolution (C10 / E5.3). Read-only: it reports
drift; it does not mutate either side. **No `-f`** (it works over a directory + a
system file).

```
strok token-sync <dir> --system <file.strok> [--json]
```

- `<dir>`: the icon-set directory whose `.strok` files **reference** tokens
  (`fill $accent`, `stroke $color.ink`, …)
- `--system <file.strok>`: the design-system file that **defines** the tokens
  (its `palette` + `tokens` blocks) — the source of truth
- Reports **undefined** references (icon uses a token the system lacks — a broken
  icon), **unused** tokens (defined, no icon uses), and **matched** references.
  A color token matches either its dotted (`$color.ink`) or bare (`$copper`)
  spelling.
- **Exit code is non-zero** when any reference is undefined (CI-gateable).
- `--json` emits the stable report schema:
  `{ "in_sync", "defined": [...], "matched": [...], "undefined": [...], "unused": [...] }`

```
strok token-sync icons/src --system ds/design-system.strok
strok token-sync icons/src --system ds/design-system.strok --json
```

### `annotate`

Render with element IDs overlaid visually on the canvas.

```
strok -f <file> annotate [--out <file.png>] [--width <px>] [--height <px>]
```

- Same as `render`, but each element gets a semi-transparent label showing its ID
- Useful when the agent needs to map visual positions to element references

---

## Export

### `export`

Export the document to standard formats.

```
strok -f <file> export svg [--out <file.svg>]
strok -f <file> export png [--out <file.png>] [--width <px>] [--height <px>] [--color <hex>] [--scale <n>]
strok -f <file> export pdf [--out <file.pdf>]
```

- `svg` exports clean, standard SVG (no Strøk-specific extensions); `currentColor`
  is preserved verbatim for CSS theming
- `png --color <hex>`: ink to substitute for `currentColor` (default black)
- `png` is equivalent to `render` but named for "final output" intent
- If only one PNG dimension is specified, the other is inferred from the
  document aspect ratio; specify both to stretch intentionally
- `pdf` preserves vector data

---

## Visual diff (C7 / E3.3)

### `diff`

Perceptual visual diff between two renders, or across a construction-history
point. Reuses the **same** comparator the golden suite uses (`strok-render`'s
`compare`): a pixel is "materially changed" if any channel moves by more than 40;
the pair is "within tolerance" when both the mean per-channel delta (≤ 6/255) and
the changed-pixel fraction (≤ 6%) stay under the golden thresholds.

```
strok diff <a.png> <b.png> [--out <diff.png>] [--json]
strok -f <file> diff --since <op-index> [--out <diff.png>] [--width <px>] [--height <px>] [--color <hex>] [--json]
```

- **Two-file form** (`-f` not required): compares two PNGs. With `--out`, writes a
  diff PNG (changed pixels in red over a dimmed background). Prints the mean delta,
  changed-pixel count/fraction and the **changed-region bounding box**.
- **`--since <n>` form**: renders the document as of after op `n` (replaying the
  op log via `Document::replay_to`) and compares it to the current render.
- `--json` emits the stable stats object:
  `{ mean_abs, changed_pixels, total_pixels, changed_fraction, changed_bbox|null, within_tolerance }`.
- **Exit status:** `0` when the pair is within the golden perceptual tolerance,
  `1` when it differs (scriptable as a regression check).
- **Limitation (honesty register):** `--since` replays the op log, which only the
  **binary-format** `.strok` files persist. v3 DSL `.strok` files do not serialize
  the op log, so `--since` on a DSL document reports that and points you at the
  two-file form (render two versions, then `strok diff before.png after.png`).

---

## MCP server (C7 / E3.4)

### `mcp-server`

Run Strøk as a Model Context Protocol server speaking JSON-RPC 2.0 over stdio, so
any MCP-capable agent runtime can drive Strøk through schema'd tools.

```
strok mcp-server
```

- **Transport:** newline-delimited JSON-RPC 2.0 on stdin/stdout. Methods:
  `initialize`, `tools/list`, `tools/call`, `ping`; `notifications/*` are
  acknowledged with no reply.
- **Tools** (each takes the document as DSL `source` text — stateless, like the
  file-based CLI): `new`, `exec`, `render`, `inspect`, `query`, `relate`,
  `measure`. They reuse the **same** core/render code paths as the CLI verbs.
- **Images:** `render` returns an MCP `image` content block (`{type:"image",
  data:<base64 PNG>, mimeType:"image/png"}`). Inspection/query/measure return
  `text` content carrying the same stable JSON the CLI `--json` flags emit.
- **Tool errors** are returned as a successful call with `isError: true` (the
  model sees the message), not as a transport failure.
- **Decision D-3 (recorded):** implemented as a thin stdio shim rather than the
  `rmcp` SDK to keep the transport stateless and the dependency surface small.

---

## Undo / Redo

### `undo`

Revert the last operation.

```
strok -f <file> undo
```

### `redo`

Re-apply the last undone operation.

```
strok -f <file> redo
```

### `history`

Show the operation log.

```
strok -f <file> history [--limit <n>]
```

Output:
```
  1  new (800x800)
  2  append root rect#bg
  3  append root group#coach
  4  set coach-backing --fill "#f4d35e"
  5  move ball 10 -5
> 6  scale ball 0.7
```

- Arrow indicates current position in the history
- Agents can use this to understand what changed since their last interaction

---

## ID System

- Auto-generated IDs: 8 characters, lowercase alphanumeric (`[a-z0-9]{8}`)
- On collision during generation: retry with a new random value, then extend by one character if retries are exhausted
- Agent-provided IDs (via `id` attribute in SVG snippets): honored exactly as given
- If an agent-provided ID conflicts with an existing ID, the command fails with an error
- ID resolution: exact match only (no prefix matching needed since IDs are short)
- `root` is a reserved ID referring to the document root node

---

## Error Handling

All errors go to stderr. Exit code 0 on success, non-zero on failure.

Error format:
```
error: id 'header-bg' already exists (rect, child of root at index 2)
```

Errors include enough context for an agent to self-correct:
- ID conflicts: show the existing element's type and location
- Missing IDs: suggest similar IDs if any exist
- Malformed SVG: show the parse error location
- Invalid operations: explain why (e.g., "cannot ungroup: element 'bg' is not a group")

### DSL parse diagnostics (C6 / E3.1)

`.strok` parse errors carry a **position (line + column), a caret snippet, and a
"did you mean" suggestion** for a mistyped keyword/operation/attribute (computed
by edit distance). Example — a typo'd shape operation:

```
error: unknown operation 'storke' in shape block
 --> line 5, column 3
  |
5 |   storke
  |   ^^^^^^ did you mean `stroke`?
```

Writing a bare positional coordinate where a keyed one is required is recognized
specifically:

```
error: addpoint requires `at=x,y` (coordinates must be keyed)
 --> line 3, column 14
  |
3 |   addpoint a 5,5
  |              ^^^ did you mean `at=5,5`?
```

**Error recovery:** the library exposes `dsl_parse::parse_file_recover(input) ->
(Scene, Vec<Diagnostic>)` — one malformed top-level block no longer aborts the
whole parse; the parser skips the bad block, records a diagnostic, and keeps
going, returning a best-effort scene plus *all* diagnostics. (The CLI still uses
the fail-fast `parse_file` and reports the first diagnostic; the recovering parser
feeds the future GUI/MCP "show every problem at once" loop.)

---

## Output Conventions

- `append`, `prepend`, `insert`: prints the assigned ID to stdout (useful when no explicit ID was provided)
- `inspect`, `layers`, `info`, `history`: prints to stdout
- `render`, `annotate`, `export`: writes file (or stdout if no `--out`)
- `set`, `remove`, `move`, `rotate`, etc.: silent on success (exit code 0)
- All commands: `--json` flag for machine-readable output where applicable
