use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "strok",
    version,
    about = "Strøk — vector design for the agent era.",
    long_about = "\
Strøk — vector design for the agent era.

A .strok file is plain text. You can edit it directly or build it with CLI commands.
Coordinates: origin is top-left, +x is right, +y is down.

AGENTS: Run `strok agent-intro` before authoring. It explains effort levels,
tool discovery, and the render-review loop. Then load the relevant visual guide
with `strok guide illustration|icon|logo|diagram`.

QUICK START — build a simple scene with CLI commands:

  strok new face.strok 200x200
  strok -f face.strok shape head --template ellipse \"fill #ffcc88\"
  strok -f face.strok shape eye --template ellipse \"fill #333333\"
  strok -f face.strok place head --shape head --at 0,0 --size 200x200
  strok -f face.strok place left-eye --shape eye --at 60,70 --size 20x20
  strok -f face.strok place right-eye --shape eye --at 120,70 --size 20x20
  strok -f face.strok render --out face.png

This produces the same file as writing it by hand:

  documentsize 200x200
  shape head template=ellipse
    fill #ffcc88
  shape eye template=ellipse
    fill #333333
  place head shape=head at=0,0 size=200x200
  place left-eye shape=eye at=60,70 size=20x20
  place right-eye shape=eye at=120,70 size=20x20

Shapes are reusable templates (geometry + style). Place puts an instance at a
position and size. You can place the same shape many times.

WORKFLOW — the render loop:

  strok -f face.strok render --out face.png   see what it looks like
  # edit face.strok directly, then render again — repeat
  strok -f face.strok inspect                 read what's in it
  strok -f face.strok inspect --svg           see the resolved SVG

The file is the source of truth. Edit it directly or use the CLI — both work.",
    after_help = "\
DSL REFERENCE — everything you can write inside a .strok file:

  documentsize 800x600

  shape <name> template=<T>
    Templates: rectangle  ellipse  triangle  line  path  text

    Attributes (all shapes):
      fill #rrggbb | none | $token
      fill radial(center, 80%, #d8b480, transparent)
      fill linear(top, bottom, #ff0000, #0000ff)
      stroke #rrggbb | none | $token
      stroke-width 4
      stroke-linecap round | butt | square
      stroke-linejoin miter | round | bevel
      stroke-dasharray 5 3              dash-gap pattern
      opacity 0.8                       0–1
      blur 3                            gaussian blur radius

    Geometry ops (path / line):
      addpoint p1 at=50,100
      addpoint p2 at=200,100 after=p1 mode=catmull-rom tension=0.2
      addpoint a1 at=150,50 after=p1 mode=arc rx=30 ry=20
      addpoint a2 at=150,50 after=p1 mode=arc rx=30 bulge=left|right
      addpoint c1 at=240,120 after=p2 mode=controls c1=220,90 c2=260,140
      addpoint c2 at=300,120 after=c1 mode=controls-relative c1=-30,-20 c2=20,0
      movepoint p1 to=60,90
      movepoint p1 dx=5 dy=-3
      pullpoint top dir=up 20%
      sculpt at=100,200 dx=0 dy=-15 radius=40 falloff=0.5
      subpath                            next point begins an independent contour/route
      close | open
      smooth all [tension=0.3]           set all points to catmull-rom
      sharpen all                        set all points to sharp corners
      smooth <point> [tension=0.3]       curve the segment arriving at this point
      smooth-corner <point> [tension=N]  smooth both sides of this anchor
      sharpen <point>                    straighten the segment arriving here
      convert-point <pt> to=<mode>       convert its incoming segment in place
                                         (sharp|smooth|arc|controls), keeps position
      round-corners 8                    rounded rectangle corners (uniform radius)
      round-corners tl=8 tr=8 br=0 bl=0  per-corner radii (by point name)
      notch edge=top dir=out shape=square pos=0.3 width=8 depth=3
                                         folder tab / bubble tail primitive
                                         (dir=out|in, shape=square|triangle)

    Choosing a curve mode:
      A point's mode controls the segment arriving at it, not both adjacent
      segments. Use smooth-corner for anchor intent; use smooth for a precise
      incoming segment or every consecutive endpoint in a longer curve run.
      Use catmull-rom only for organic curves (leaves, fur, freehand strokes).
      For UI / geometric shapes (rounded rects, circles, arches, cylinders) use
      round-corners / mode=arc / mode=sharp — catmull-rom through near-collinear
      points produces wavy, faceted edges.
      mode=arc bulge side: add bulge=left|right (order-independent) to pick the
      side without reversing points. sweep=cw|ccw (= 1|0) is the raw SVG flag.

    Text ops (text template only):
      content \"Hello world\"
      font-size 48
      font-family \"Georgia\"
      font-weight bold | normal | 100–900
      font-style normal | italic
      text-anchor start | middle | end

    Effects:
      applyeffect droop 0.2
      applyeffect jitter 0.1 seed=42
      applyeffect curl 0.3
      applyeffect taper start=100% end=0%

  place <name> shape=<ref> at=x,y [size=WxH] [rotation=45deg] [flip=x|y|xy] [skew=deg|degx,degy] [clip=<shape>[,…]] [mask=<shape>]
  place <name> shape=<ref> below=<other> [gap=5] [size=WxH]
  place <name> shape=<ref> above=<other> [gap=5] [size=WxH]
    fill #override    inline attribute override, indented under place

  Direct geometry (sugar for at/size):
    place slit shape=slit from=16,8 to=8,16      a line by its two endpoints
    place hub  shape=hub  center=12,12 radius=5  center + radius (size = 2r x 2r)
    place e    shape=e    center=12,12 size=8x4  center + explicit size

  Relative placement:
    place <name> shape=<ref> at=<target>.<anchor> [align=<self-anchor>] [offset=dx,dy] [size=WxH]

    Anchors: tl  top  tr  left  center  right  bl  bottom  br

    at= picks a point on the target's bounding box.
    align= picks which point on THIS shape goes there (default: tl).
    offset= nudges from the resolved position.

    place body shape=body at=seat.top align=bottom size=70x80
    place beak shape=beak at=head.right align=left offset=0,5
    place hub shape=hub at=wheel.center align=center size=16x16

  defaults                               inherited by all shapes
    fill #2d5a1e
    stroke-width 1.5

  Colorschemes — name colors once, swap them per theme:
    palette                              base tokens (also the fallback)
      hero #e8a840
      accent #c8863a
    scheme dark                          a theme overriding some tokens
      hero #f4c266
    Reference a token in any fill/stroke:   fill $hero    stroke $accent
    Render one theme:    render --out icon.png --scheme dark
    Render every theme:  render --out icon.png   →  icon.png + icon-dark.png
    inspect --svg --scheme dark            see resolved colors for a theme

  createlink <name> from=<shape>   # linked copy, own fill/stroke/opacity
    fill #variant

  group <name> [at=x,y] [rotation=Ndeg] [flip=x|y|xy] [skew=deg|degx,degy] [clip=<shape>[,…]] [mask=<shape>] [opacity=0.8]
    place ...

  Computation (C13) — scalar expressions, let bindings, repeat blocks:
    Expressions: any scalar (coords, size, radius, rotation, gap, round-corners,
      addpoint at=) may be a SPACE-FREE arithmetic expression: number | $name |
      + - * / % | ( ). e.g.  at=40+$i*60,190   round-corners $r*2
    let <name> <expr>                    a named f64, referenced as $name
      let col 310
      let inner $col+20                  later lets may use earlier ones
    repeat <var> <count>                 expand the body <count> times (count is
      place dot shape=dot center=40+$var*60,40 radius=6   an expression; ≤10000)
      For i in 0..count, $<var>=i; every place/group/createlink NAME in the body
      gets -<i> appended (dot-0, dot-1, …) and sibling refs are rewritten to match.
      Nested repeats append after the outer (dot-0-1). Expansion is parse-time —
      re-emitting the file flattens repeats to their concrete nodes.

Point names per template:
  rectangle  tl tr br bl
  ellipse    top right bottom left
  triangle   top br bl
  line       start end
  path/text  you define them with addpoint

Z-order: shapes render in file order. Last placed = on top.

SHAPE AUTHORING — how to think about building shapes:

  1. Decompose first. Ask: what geometric primitives make up this shape?
     A badge is a rounded rectangle. A drop is an ellipse pinched at one end.
     A speech bubble is a rectangle + triangle. Start from the template that's
     closest, then sculpt — don't reach for point-by-point tracing first.

  2. Choose the right curve mode for the job:
     arc         — circular/elliptical curves (corners, bubbles, UI elements)
     catmull-rom — organic flowing curves (leaves, waves, letter forms)
     controls    — explicit cubic Bézier segments (logos, glyphs, precise curves)
     sharp       — hard corners (geometric shapes, angles)

     `smooth p` curves only the segment arriving at p. `smooth-corner p`
     curves both adjacent segments and is the safer spelling for corner intent.

  3. Separate concerns: define points first, then curve them.
     The smooth all + sharpen pattern works well for mostly-curved shapes:

       addpoint tip at=32,8
       addpoint p2 at=48,30
       addpoint p3 at=50,42
       addpoint p4 at=32,56
       close
       smooth all tension=0.3    # curve everything
       sharpen tip               # except the tip

  4. Use round-corners for UI shapes:
       shape badge template=rectangle
         round-corners 8
         fill #3b82f6

  5. Iterate with the render loop:
       strok -f icon.strok render --out preview.png
       # look at the image, adjust points/tension, render again

STANDARD LIBRARY — reusable shapes embedded in the binary (EXP-1):

  strok lib list                       every module + shape, one-line meanings
  strok lib show figures               print a module's raw .strok source
  strok lib search person              search names/meanings/tags

  use \"std/figures\" as fig            import — no files on disk needed
  place p shape=fig.person-standing at=0,0 size=40x100
    fill #2d5a1e

  Modules: std/figures  std/arrows  std/bubbles  std/devices  std/furniture"
)]
pub struct Cli {
    /// Target document (.strok file)
    #[arg(short = 'f', long = "file", global = true)]
    pub file: Option<String>,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// Learn how an agent should plan, build, and visually verify Strøk work
    #[command(long_about = "\
Print the agent operating guide before authoring with Strøk.

It explains:
  - how to choose sketch, production, or showcase effort
  - how to discover DSL primitives and standard-library shapes
  - how to build from composition to detail
  - how to use full-frame and region renders, audit, inspect, query, and diff
  - when technically valid output is not visually finished

Start here, then load the guide for the requested output:
  strok agent-intro
  strok guide illustration")]
    AgentIntro,

    /// Learn the visual workflow for a target output
    #[command(long_about = "\
Print an agent-oriented authoring guide. The guide prioritizes final visual
quality over minimizing commands and includes style decisions, geometry traps,
and the render/review loop.

Topics: illustration, icon, logo, diagram

Examples:
  strok guide illustration
  strok guide icon
  strok guide logo
  strok guide diagram")]
    Guide {
        /// Guide topic: icon, logo, or diagram
        topic: String,
    },

    /// Create a new document
    #[command(long_about = "\
Create a new .strok document.
The file starts with `documentsize WxH` — add shapes and places to it.

Examples:
  strok new logo.strok 600x400
  strok new icon.strok --profile icon-solid
  strok new icon.strok --profile icon-outline-angular")]
    New {
        /// Output file path
        path: String,
        /// Canvas dimensions (e.g. 400x400)
        #[arg(default_value = "800x800")]
        size: String,
        /// Seed a visual profile. Recommended: icon-outline-round,
        /// icon-outline-angular, icon-solid, or icon-mixed. `icon` remains an
        /// alias for icon-outline-round. Omit for a bare document.
        #[arg(long)]
        profile: Option<String>,
    },

    /// Live-preview a document in the browser while you edit it
    #[command(long_about = "\
Watch a .strok file and live-preview it in the browser.

Starts a local preview server, opens it in the browser, and re-renders on
every save. Parse errors show in the page while the last good render stays
visible, so the preview never goes blank mid-edit. Built for editing .strok
source by hand in an editor; press Ctrl-C to stop.

Examples:
  strok watch design.strok
  strok -f design.strok watch
  strok watch design.strok --scheme dark --port 4848 --no-open")]
    Watch {
        /// Document to watch (or pass it with -f)
        file: Option<String>,
        /// Port for the preview server (default: any free port)
        #[arg(long, default_value_t = 0)]
        port: u16,
        /// Colorscheme to resolve tokens with (base if omitted)
        #[arg(long)]
        scheme: Option<String>,
        /// Don't open the browser automatically
        #[arg(long)]
        no_open: bool,
    },

    /// Execute a DSL line (appends to the file)
    #[command(long_about = "\
Append any raw DSL line to the file.
Use this for operations the other commands don't cover.
The file is validated before writing — you'll see an error if the line is invalid.

Examples:
  strok -f logo.strok exec \"shape bg template=rectangle\"
  strok -f logo.strok exec \"  fill #faf6f0\"
  strok -f logo.strok exec \"  close\"")]
    Exec {
        /// The DSL line to execute
        line: String,
    },

    /// Add a shape definition
    #[command(long_about = "\
Add a shape definition block to the file.
Ops are DSL lines that go inside the shape block, one per argument.

Templates: rectangle  ellipse  triangle  line  path  text

Examples:
  strok -f logo.strok shape bg --template rectangle \"fill #faf6f0\"
  strok -f logo.strok shape badge --template ellipse \"fill #c8863a\" \"stroke #7a5020\" \"stroke-width 2\"
  strok -f logo.strok shape label --template text \"content \\\"Strøk\\\"\" \"font-size 48\" \"fill #1a1a1a\"")]
    Shape {
        /// Shape name
        name: String,
        /// Template (rectangle, ellipse, triangle, line, path, text)
        #[arg(long)]
        template: String,
        /// Additional operations/attributes as DSL lines (one per arg)
        #[arg(trailing_var_arg = true)]
        ops: Vec<String>,
    },

    /// Place a shape instance
    #[command(long_about = "\
Place a shape instance in the scene.
Shapes render in file order — last placed = on top (front).

Relative placement (chain to another placed element):
  at=<target>.<anchor>   reference a point on another shape's bounding box
  align=<anchor>         which point on THIS shape goes at the target point (default: tl)
  offset=dx,dy           nudge from the resolved position

  Anchor points: tl  top  tr  left  center  right  bl  bottom  br

Direct-geometry placement (sugar for at/size):
  --from x1,y1 --to x2,y2   place a line by its two document-space endpoints
  --center cx,cy            center-anchored placement (with --size or --radius)
  --radius r | rx,ry        size expressed as radii (size = 2r x 2r)

Examples:
  strok -f f.strok place bg --shape bg --at 0,0 --size 400x400
  strok -f f.strok place body --shape body --at seat.top --align bottom --size 70x80
  strok -f f.strok place beak --shape beak --at head.right --align left --offset 0,5
  strok -f f.strok place hub --shape hub --center 12,12 --radius 5
  strok -f f.strok place slit --shape slit --from 16,8 --to 8,16
  strok -f f.strok place eye --shape eye --at head.tr --offset -12,-12 --size 7x7")]
    Place {
        /// Instance name
        name: String,
        /// Shape reference
        #[arg(long)]
        shape: String,
        /// Position as x,y or target.anchor (e.g. body.top, head.right)
        #[arg(long)]
        at: Option<String>,
        /// Size as WxH
        #[arg(long)]
        size: Option<String>,
        /// Rotation in degrees (e.g. 45 or 45deg)
        #[arg(long, allow_hyphen_values = true)]
        rotation: Option<String>,
        /// Flip (x, y, or xy)
        #[arg(long)]
        flip: Option<String>,
        /// Self-anchor: which point on this shape goes at the target (tl, top, center, etc.)
        #[arg(long)]
        align: Option<String>,
        /// Offset nudge as dx,dy (e.g. 5,-3)
        #[arg(long, allow_hyphen_values = true)]
        offset: Option<String>,
        /// Line start point as x,y (use with --to; replaces --at/--size)
        #[arg(long, allow_hyphen_values = true)]
        from: Option<String>,
        /// Line end point as x,y (use with --from)
        #[arg(long, allow_hyphen_values = true)]
        to: Option<String>,
        /// Center point as cx,cy (use with --size or --radius)
        #[arg(long, allow_hyphen_values = true)]
        center: Option<String>,
        /// Radius as r or rx,ry (sugar for --size 2r x 2r)
        #[arg(long)]
        radius: Option<String>,
    },

    /// Create a linked shape instance
    #[command(long_about = "\
Create a linked copy of a shape with different attributes.
Geometry comes from the source shape; the link carries its own fill/stroke/opacity.

Examples:
  strok -f logo.strok createlink petal-dark from=petal \"fill #a82848\" \"opacity 0.8\"
  strok -f logo.strok createlink badge-outline from=badge \"fill none\" \"stroke #c8863a\" \"stroke-width 2\"")]
    Createlink {
        /// Link name
        name: String,
        /// Source shape name
        #[arg(long)]
        from: String,
        /// Additional overrides as DSL lines
        #[arg(trailing_var_arg = true)]
        ops: Vec<String>,
    },

    /// Move a named point
    #[command(long_about = "\
Move a named control point inside a shape definition.
Point must be written as shape.point (e.g. stem.base, badge.top).

Point names by template:
  rectangle  tl tr br bl
  ellipse    top right bottom left
  triangle   top br bl
  line       start end

Examples:
  strok -f logo.strok movepoint stem.base --to 200,390
  strok -f logo.strok movepoint badge.top --dx 0 --dy -10")]
    Movepoint {
        /// Qualified point (shape.point)
        point: String,
        /// X offset
        #[arg(long, allow_hyphen_values = true)]
        dx: Option<f64>,
        /// Y offset
        #[arg(long, allow_hyphen_values = true)]
        dy: Option<f64>,
        /// Absolute position as x,y
        #[arg(long)]
        to: Option<String>,
    },

    /// Inspect document (DSL text, SVG, or a structural snapshot)
    #[command(long_about = "\
Read the document or a specific element.
No selector → print the whole file.
Selector → print just that shape or place block.
--svg → emit the resolved SVG instead of DSL (good for debugging what renders).
--detail full|structural|summary → a structural snapshot (E3.2):
    full        the whole resolved SVG
    structural  element names, kinds + bboxes, no path geometry
    summary     just IDs + types (the lightest snapshot)
--json → machine-readable snapshot (implies a structural snapshot; one stable,
         snapshot-tested schema shared with `query`, `relate` and `measure`).

Examples:
  strok -f logo.strok inspect
  strok -f logo.strok inspect bg
  strok -f logo.strok inspect --svg
  strok -f logo.strok inspect --detail structural
  strok -f logo.strok inspect --json
  strok -f logo.strok inspect --detail summary --json")]
    Inspect {
        /// Shape or place name to inspect
        selector: Option<String>,
        /// Output SVG instead of DSL
        #[arg(long)]
        svg: bool,
        /// Structural snapshot detail level: full, structural, summary
        #[arg(long)]
        detail: Option<String>,
        /// Emit a machine-readable JSON snapshot (E3.2)
        #[arg(long)]
        json: bool,
        /// Resolve palette tokens with this colorscheme (only affects --svg)
        #[arg(long)]
        scheme: Option<String>,
    },

    /// Render document to PNG
    #[command(long_about = "\
Render the document to a PNG.
--out omitted → PNG bytes to stdout (pipe to a file or viewer).
--node → render one named placed element on an otherwise empty canvas;
         useful for previewing a single shape mid-build.

Colorschemes:
  --scheme <name>   render one scheme to --out.
  (omit --scheme)   with --out set, render the base palette to --out plus
                    every scheme to <out>-<scheme>.png.

Examples:
  strok -f logo.strok render --out preview.png
  strok -f logo.strok render --out preview.png --bg '#ffffff'
  strok -f logo.strok render --node badge --out badge.png
  strok -f logo.strok render --outline --out geometry.png
  strok -f logo.strok render --outline badge,rim --region 20,20,80,80 --width 1200 --out detail.png
  strok -f logo.strok render --out icon.png --scheme dark
  strok -f logo.strok render --width 1200 --out hires.png

When only one output dimension is supplied, the other is inferred from the
document aspect ratio. Supply both to stretch intentionally.")]
    Render {
        /// Output file (stdout if omitted)
        #[arg(long)]
        out: Option<String>,
        /// Render width
        #[arg(long)]
        width: Option<u32>,
        /// Render height
        #[arg(long)]
        height: Option<u32>,
        /// Background color
        #[arg(long)]
        bg: Option<String>,
        /// Concrete color to substitute for `currentColor` (default black). Use to
        /// preview themeable icons on dark/light backgrounds.
        #[arg(long)]
        color: Option<String>,
        /// Render only this named node (place name)
        #[arg(long)]
        node: Option<String>,
        /// Render a document-space crop as x,y,w,h. Use this to inspect focal
        /// objects and edge quality without enlarging the entire composition.
        #[arg(long, value_name = "x,y,w,h")]
        region: Option<String>,
        /// Overlay element IDs on the canvas (annotate mode, E3.2) — maps the
        /// rendered pixels back to the names an agent can reference.
        #[arg(long)]
        annotate: bool,
        /// Overlay resolved geometry above the normal scene. Use the bare flag
        /// for every placed element, or pass comma-separated placed IDs.
        #[arg(long, value_name = "id1,id2")]
        outline: Option<Option<String>>,
        /// Colorscheme to resolve tokens with. Omit to render base + every scheme (suffixed).
        #[arg(long)]
        scheme: Option<String>,
    },

    /// Batch-render every .strok in a directory (for icon sets)
    #[command(long_about = "\
Render every `*.strok` file in a directory to SVG and/or PNG — built for icon
sets. No `-f` needed; the directory is the unit of work.

By default emits BOTH a clean themeable SVG (currentColor preserved) and PNGs at
each --size. SVG filename is `<name>.svg`; PNG is `<name>-<size>.png` (or
`<name>.png` when a single size is given).

Design-system outputs (each optional, all driven off the same parsed set so they
never drift):
  --sprite <file.svg>    a <symbol> sprite sheet (host: <use href=\"sprite.svg#name\"/>)
  --sheet  <file.png>    a contact sheet — every icon laid out on a grid (no
                         external `montage` needed); --columns sets the grid width
  --manifest <file.json> a registry: name → meaning/tags/canvas/sizes. Meaning &
                         tags are authored as leading `# @meaning …` / `# @tags a, b`
                         comments in each icon file (additive; existing files stay valid).

Examples:
  strok batch icons/src --out icons/dist
  strok batch icons/src --out icons/dist --sizes 16,24,32
  strok batch icons/src --out icons/dist --png --color '#e6e6e6' --bg '#0d1117'
  strok batch icons/src --svg                       # SVG only, into ./dist
  strok batch icons/src --sprite dist/sprite.svg --manifest dist/manifest.json
  strok batch icons/src --sheet dist/contact.png --columns 8 --color '#e6e6e6'")]
    Batch {
        /// Directory containing `.strok` files to render
        dir: String,
        /// Output directory (default: <dir>/dist)
        #[arg(long)]
        out: Option<String>,
        /// Emit SVG (themeable, currentColor kept). Default: on unless --png given alone.
        #[arg(long)]
        svg: bool,
        /// Emit PNG at each --size. Default: on unless --svg given alone.
        #[arg(long)]
        png: bool,
        /// Comma-separated PNG pixel sizes (square), e.g. 16,24,32. Default: 24.
        #[arg(long)]
        sizes: Option<String>,
        /// Concrete color to substitute for `currentColor` in PNGs (default black)
        #[arg(long)]
        color: Option<String>,
        /// PNG background color (default transparent)
        #[arg(long)]
        bg: Option<String>,
        /// Resolve palette tokens with this colorscheme
        #[arg(long)]
        scheme: Option<String>,
        /// Also write a `<symbol>` sprite sheet to this path
        #[arg(long)]
        sprite: Option<String>,
        /// Also write a contact-sheet PNG (grid of all icons) to this path
        #[arg(long)]
        sheet: Option<String>,
        /// Also write an icon-set manifest (name→meaning/tags/sizes) to this path
        #[arg(long)]
        manifest: Option<String>,
        /// Contact-sheet grid column count (default 8)
        #[arg(long)]
        columns: Option<u32>,
    },

    /// Reorder a placed node (z-order)
    #[command(long_about = "\
Change the z-order of a placed element.
Last in file = renders on top. First = behind everything else.

  front          move to top (last in file)
  back           move to bottom (first place block)
  before=<name>  place immediately below another element
  after=<name>   place immediately above another element

Examples:
  strok -f logo.strok reorder badge front
  strok -f logo.strok reorder shadow back
  strok -f logo.strok reorder label after=badge
  strok -f logo.strok reorder glow before=badge")]
    Reorder {
        /// Name of the placed node to reorder
        name: String,
        /// Position: front, back, before=<name>, after=<name>
        position: String,
    },

    /// Boolean-combine two or more placed shapes into a new path shape
    #[command(long_about = "\
Combine placed shapes with a boolean operation, producing a new filled `path`
shape (authored in document coordinates, placed at 0,0). The inputs are
identified by their PLACE name (the id you gave `place`), so the geometry is
taken exactly where it renders. Self-intersecting / holey inputs are interpreted
with each shape's fill-rule; a holey result gets fill-rule even-odd automatically.

Operations:
  union       area covered by ANY input
  subtract    first input MINUS the rest (knockouts, folder tabs)
  intersect   area covered by ALL inputs
  exclude     symmetric difference (XOR)

The result is an ordinary path shape: it round-trips, renders, and re-edits.

Examples:
  strok -f icon.strok bool subtract folder tab --out folder-cut
  strok -f logo.strok bool union ring-a ring-b ring-c --out rings
  strok -f mark.strok bool intersect circle square --out lens")]
    Bool {
        /// Operation: union, subtract, intersect, exclude
        op: String,
        /// Two or more placed-element names to combine (subject first)
        #[arg(required = true, num_args = 2..)]
        ids: Vec<String>,
        /// Name for the resulting shape + place (default: <op>-result)
        #[arg(long)]
        out: Option<String>,
    },

    /// Convert a stroked path into its filled outline (a new path shape)
    #[command(long_about = "\
Convert a placed stroked path into the filled region the stroke paints, honoring
its width / linecap / linejoin / miterlimit. The result is a new filled `path`
shape (placed at 0,0). Useful for turning outline icons into solid shapes, or to
boolean-combine a stroke with other fills.

Examples:
  strok -f icon.strok outline-stroke wire --out wire-filled")]
    OutlineStroke {
        /// Placed-element name whose stroke to outline
        id: String,
        /// Name for the resulting shape + place (default: <id>-outline)
        #[arg(long)]
        out: Option<String>,
    },

    /// Offset (grow/inset) a placed shape's filled region by a delta
    #[command(long_about = "\
Grow (positive delta) or shrink (negative delta) the filled region of a placed
shape by `delta` document units, producing a new filled `path` shape (placed at
0,0). Corners are rounded (Minkowski offset of a disk): offsetting a circle by r
yields a concentric circle of radius ±r.

Examples:
  strok -f badge.strok offset card 4 --out card-grown
  strok -f ring.strok offset disc -3 --out disc-inset")]
    Offset {
        /// Placed-element name to offset
        id: String,
        /// Offset distance in document units (negative = inset)
        #[arg(allow_hyphen_values = true)]
        delta: f64,
        /// Name for the resulting shape + place (default: <id>-offset)
        #[arg(long)]
        out: Option<String>,
    },

    /// Apply an affine transform (rotate/scale/skew/flip) to a placed element
    #[command(long_about = "\
Apply a 2D affine transform to an existing placed element, in place. All four
operate through the same unified affine (E2.3), so they compose predictably and
keep the placed bbox correct for anchors / relative placement.

  --rotate <deg>     rotate about the element's center
  --scale <factor>   uniform scale (multiplies the element's size)
  --skew <degx[,degy]>  skew about the center
  --flip <x|y|xy>    mirror

Examples:
  strok -f icon.strok transform badge --rotate 15
  strok -f icon.strok transform card --skew 12,0
  strok -f logo.strok transform arrow --flip x")]
    Transform {
        /// Placed-element name to transform
        name: String,
        /// Rotation in degrees about the center
        #[arg(long, allow_hyphen_values = true)]
        rotate: Option<f64>,
        /// Uniform scale factor (multiplies current size)
        #[arg(long, allow_hyphen_values = true)]
        scale: Option<f64>,
        /// Skew in degrees: degx or degx,degy
        #[arg(long, allow_hyphen_values = true)]
        skew: Option<String>,
        /// Flip axis: x, y, or xy
        #[arg(long)]
        flip: Option<String>,
    },

    /// Convert a path point's incoming segment in place (sharp/smooth/arc/controls)
    #[command(long_about = "\
Convert the segment arriving at a single named point, preserving the point's
position (E2.5). The point is written as shape.point.

  --to sharp      hard corner
  --to smooth     catmull-rom smooth
  --to arc        elliptical arc segment (radius from the chord)
  --to controls   explicit cubic bezier handles (a straight default to bend)

Examples:
  strok -f leaf.strok convert-point leaf.tip --to sharp
  strok -f bubble.strok convert-point bubble.p2 --to arc")]
    ConvertPoint {
        /// Qualified point (shape.point)
        point: String,
        /// Target type: sharp, smooth, arc, controls
        #[arg(long)]
        to: String,
    },

    /// Flow a text string along a placed path (`<textPath>`)
    #[command(long_about = "\
Place a text string so it flows along the geometry of another placed shape
(E2.7). Creates a `text` shape with your content and a `place` that references
the path via `textpath=`. The renderer and SVG export share the path's `d`.

The --path argument is the PLACE name of the path to flow along.

Examples:
  strok -f badge.strok text-on-path \"MEMBER SINCE 2024\" --path ring --name label
  strok -f wave.strok text-on-path \"ride the curve\" --path crest --size 32")]
    TextOnPath {
        /// The text content to flow along the path
        text: String,
        /// Place name of the path to flow the text along
        #[arg(long)]
        path: String,
        /// Name for the created text place (default: text-on-path)
        #[arg(long)]
        name: Option<String>,
        /// Font size
        #[arg(long)]
        size: Option<f64>,
        /// Fill color (default: currentColor)
        #[arg(long)]
        fill: Option<String>,
    },

    /// Measure distance / gap / alignment between two placed elements
    #[command(long_about = "\
Report the spatial relationship between two placed elements (E2.7), using the
same bounding-box machinery the anchor / relative-placement resolver uses — so
the numbers match what the canvas shows.

Reports: center-to-center distance, signed center delta, per-axis gap (negative
= overlap depth), an overlap flag, and per-edge alignment deltas (b - a; 0 = that
edge is aligned). Read-only — never modifies the file.

--json prints a stable, machine-readable object (the schema C6's query layer reuses).

Examples:
  strok -f card.strok measure title body
  strok -f card.strok measure title body --json")]
    Measure {
        /// First placed element name
        a: String,
        /// Second placed element name
        b: String,
        /// Emit machine-readable JSON instead of text
        #[arg(long)]
        json: bool,
    },

    /// Query the document by region or overlap (E3.2)
    #[command(long_about = "\
Answer \"what's in this region?\" / \"what overlaps element X?\" using the same
bbox machinery the anchor resolver uses (E3.2). Read-only.

  --box x,y,w,h   every element whose bbox intersects the rectangle
  --overlaps <id> every element whose bbox intersects element <id>'s

--json prints the stable, snapshot-tested schema shared with `inspect`/`relate`.

Examples:
  strok -f ui.strok query --box 0,0,100,50
  strok -f ui.strok query --overlaps button
  strok -f ui.strok query --overlaps button --json")]
    Query {
        /// Region to search: x,y,w,h
        #[arg(long, value_name = "x,y,w,h")]
        r#box: Option<String>,
        /// Find everything overlapping this placed element
        #[arg(long)]
        overlaps: Option<String>,
        /// Emit machine-readable JSON
        #[arg(long)]
        json: bool,
    },

    /// Describe the spatial relation between two elements (E3.2)
    #[command(long_about = "\
Report the qualitative spatial relationship between two placed elements (E3.2):
horizontal (left-of / right-of / x-overlap), vertical (above / below /
y-overlap), overlap flag, containment, and which edges line up. Built on the
`measure` deltas so it stays consistent with the canvas. Read-only.

--json prints the stable, snapshot-tested schema shared with `inspect`/`query`.

Examples:
  strok -f card.strok relate title body
  strok -f card.strok relate icon label --json")]
    Relate {
        /// First placed element name
        a: String,
        /// Second placed element name
        b: String,
        /// Emit machine-readable JSON instead of text
        #[arg(long)]
        json: bool,
    },

    /// Snap a placed element's position to a grid / edge / center
    #[command(long_about = "\
Snap an existing placed element's `at` position to a reference (E2.7):

  grid     round each axis to the nearest multiple of --step (default 8)
  edge     snap each axis to the nearer document edge (0 or width/height)
  center   snap to the document center

Only places positioned with an absolute `at=x,y` are snapped; relative /
parametric placements are left untouched (with a note). Re-emits the document.

Examples:
  strok -f ui.strok snap button grid --step 8
  strok -f ui.strok snap logo center")]
    Snap {
        /// Placed element name to snap
        name: String,
        /// Snap mode: grid, edge, center
        mode: String,
        /// Grid step (grid mode only; default 8)
        #[arg(long)]
        step: Option<f64>,
    },

    /// Analyze file and suggest structural improvements
    #[command(long_about = "\
Analyze a .strok file for patterns that could be simplified and visual traps
that an agent can verify mechanically. Detects mirrored shape pairs (use
flip=x), unused composition features, unsafe text/shape collisions, raw
baseline labels, and other opportunities to improve the source.

Each finding carries a concrete, copy-pasteable `suggestion` (e.g. the exact
`flip=x` place line, or the primitive to swap a rough catmull-rom run for).

This is read-only — it never modifies the file.

Examples:
  strok -f portrait.strok audit
  strok -f portrait.strok audit --json")]
    Audit {
        /// Emit findings as JSON (stable schema) for tooling
        #[arg(long)]
        json: bool,
    },

    /// Cross-check an icon set's `$token` use against a design-token system
    #[command(long_about = "\
Token sync (C10 / E5.3): verify that every `$token` an icon set references is
actually defined by your design-token system, and surface tokens the system
defines that no icon uses.

  --system <file.strok>  the design-system file that DEFINES the tokens (palette
                         + tokens blocks) — the source of truth.
  <dir>                  the icon-set directory whose `.strok` files REFERENCE
                         tokens (fill $accent, stroke $color.ink, …).

Read-only: it reports drift (with --json for tooling), it does not mutate either
side — you fix the source of truth. Exit code is non-zero when an icon references
an undefined token (a broken icon).

Examples:
  strok token-sync icons/src --system ds/design-system.strok
  strok token-sync icons/src --system ds/design-system.strok --json")]
    TokenSync {
        /// Icon-set directory whose `.strok` files reference tokens
        dir: String,
        /// Design-system `.strok` file that defines the tokens
        #[arg(long)]
        system: String,
        /// Emit the sync report as JSON (stable schema)
        #[arg(long)]
        json: bool,
    },

    /// Export to standard formats
    #[command(long_about = "\
Export to SVG or PNG. Equivalent to render/inspect --svg but with an
explicit format argument suitable for scripting.

Examples:
  strok -f logo.strok export svg --out logo.svg
  strok -f logo.strok export png --out logo.png --width 2400

When only one PNG dimension is supplied, the other is inferred from the
document aspect ratio. Supply both to stretch intentionally.")]
    Export {
        /// Format: svg, png
        format: String,
        /// Output file
        #[arg(long)]
        out: Option<String>,
        /// Width (for png)
        #[arg(long)]
        width: Option<u32>,
        /// Height (for png)
        #[arg(long)]
        height: Option<u32>,
        /// Concrete color to substitute for `currentColor` in PNG export (default black)
        #[arg(long)]
        color: Option<String>,
        /// Resolve palette tokens with this colorscheme
        #[arg(long)]
        scheme: Option<String>,
    },

    /// Visually diff two renders, or diff the document across a history point
    #[command(long_about = "\
Perceptual visual diff (E3.3). Two forms:

  Compare two PNG files:
    strok diff a.png b.png --out diff.png
  Reuses the golden-suite comparator: a red overlay marks materially-changed
  pixels (the unchanged background is dimmed). Prints mean delta, the changed
  pixel count/fraction and the changed-region bounding box. With --json the same
  stats print as a machine-readable object. Exit status is 0 when the pair is
  within the golden perceptual tolerance, 1 when it differs.

  Diff the current document against an earlier construction-history point:
    strok -f doc.strok diff --since 2 --out diff.png
  Renders the document as of after op N and compares it to the current render
  (replaying the op log). NOTE: only binary-format .strok documents persist the
  op log; v3 DSL .strok files don't, so use the two-file form for those.

Examples:
  strok diff before.png after.png --out changed.png
  strok diff before.png after.png --json
  strok -f doc.strok diff --since 1 --out step.png")]
    Diff {
        /// First image (omit when using --since)
        a: Option<String>,
        /// Second image (omit when using --since)
        b: Option<String>,
        /// Diff against the document state after this many history ops
        #[arg(long)]
        since: Option<usize>,
        /// Write the diff PNG here
        #[arg(long)]
        out: Option<String>,
        /// Render width for --since (default: document width)
        #[arg(long)]
        width: Option<u32>,
        /// Render height for --since (default: document height)
        #[arg(long)]
        height: Option<u32>,
        /// Concrete color for `currentColor` when rendering --since (default black)
        #[arg(long)]
        color: Option<String>,
        /// Emit machine-readable JSON stats
        #[arg(long)]
        json: bool,
    },

    /// Run a Model Context Protocol server over stdio
    #[command(long_about = "\
Run Strøk as a Model Context Protocol (MCP) server (E3.4), speaking JSON-RPC 2.0
over stdio. Any MCP-capable agent runtime can spawn `strok mcp-server` and drive
Strøk through tools with JSON schemas:

  new            create a document (returns the .strok text)
  exec           append a DSL line to a document
  render         render to PNG — returned as an inline base64 image block
  inspect        structural / summary / full snapshot (JSON)
  query          region / overlap queries (JSON)
  relate         spatial relation between two elements (JSON)
  measure        distance / gap / alignment between two elements (JSON)

Documents are passed as DSL text in each call (stateless, like the CLI), so the
server itself holds no files. Render results come back as MCP image content
(base64 PNG); inspection/query/measure come back as text content carrying the
same stable JSON the CLI `--json` flags emit.

Example (the agent runtime does this for you):
  {\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",...}
  {\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/list\"}
  {\"jsonrpc\":\"2.0\",\"id\":3,\"method\":\"tools/call\",\
\"params\":{\"name\":\"render\",\"arguments\":{\"source\":\"documentsize 24x24\\n...\"}}}")]
    McpServer,

    /// Emit framework code or design tokens
    #[command(long_about = "\
Emit the document as framework code or design tokens.

Targets:
  react      a React component (.tsx)
  solid      a SolidJS component (.tsx)
  vanilla    a dependency-free HTML-string function (.ts)
  tailwind   a Tailwind v4 @theme block from every design token (theme.css)
  dtcg       a W3C DTCG design-tokens.json (interop: Style Dictionary, etc.)

react and solid are co-equal — both render the same framework-neutral UI IR,
so they produce the same DOM structure. Without --out, the emitted file(s)
print to stdout.

UX primitives (C8): `frame layout=flex|grid` lowers to real flex/grid
containers, `tokens` (color/space/radius/font/shadow) flow to @theme symbolically
(bg-<token>), and each `component` emits its own file with `instance`s lowered to
`<Button variant=… prop=… />`. A geometry-only document still emits one inline-SVG
leaf, unchanged. See the UX primitives section in DSL_SPEC.md.

Text as first-class UI (C9): a `place` of a `template=text` shape lowers to a
real, selectable, accessible DOM <span> with the text content — not glyphs
rasterized inside an SVG. Font tokens ($font.body) flow to the @theme/DTCG output
AND resolve in the rendered SVG. The `dtcg` target emits the same token system as
W3C DTCG JSON ({\"$value\", \"$type\"} grouped by category).

Examples:
  strok -f button.strok emit react --name Button
  strok -f button.strok emit solid --out ./out
  strok -f button.strok emit tailwind --out ./theme
  strok -f button.strok emit dtcg --out ./tokens")]
    Emit {
        /// Target: react, solid, vanilla, tailwind, dtcg
        target: String,
        /// Output directory (writes emitted files there; stdout if omitted)
        #[arg(long)]
        out: Option<String>,
        /// Component name override
        #[arg(long)]
        name: Option<String>,
        /// Resolve palette tokens with this colorscheme
        #[arg(long)]
        scheme: Option<String>,
    },

    /// Import an SVG into a .strok document (structure recovery)
    #[command(long_about = "\
Convert an SVG file into an editable .strok document — with STRUCTURE RECOVERY,
not a dumb path dump. Native shapes are recovered (rect → rectangle, circle →
ellipse, line → line), identical geometry collapses to one shape definition reused
by multiple places, repeated colors become palette tokens named by hue, and <g>
becomes a group. No -f is needed (like `strok new`).

What degrades (each with a warning): rotation/shear transforms are baked into path
geometry; gradients become a flat first-stop approximation; filters, masks, <use>
and <image> are skipped; text metrics are estimated.

Examples:
  strok import logo.svg --out logo.strok
  strok import icon.svg --out icon.strok --json")]
    Import {
        /// Input SVG file
        input: String,
        /// Output .strok path
        #[arg(long)]
        out: String,
        /// Report import stats (element counts, tokens, warnings) as JSON
        #[arg(long)]
        json: bool,
    },

    /// Explore the embedded standard shape library (`std/<module>`)
    #[command(long_about = "\
Explore the embedded standard shape library (EXP-1) — reusable shapes
(people, arrows, speech bubbles, devices, furniture) built into the `strok`
binary. Import them into any document with:

  use \"std/figures\" as fig
  place p shape=fig.person-standing at=0,0 size=40x100
    fill #2d5a1e

No files on disk needed — the modules are compiled into the binary from
`std/<module>.strok`.

Examples:
  strok lib list
  strok lib list --json
  strok lib show figures
  strok lib search person
  strok lib search arrow --json")]
    Lib {
        #[command(subcommand)]
        action: LibAction,
    },
}

#[derive(Subcommand)]
pub enum LibAction {
    /// List every embedded module and its shapes, one-line meanings
    List {
        /// Emit as JSON (stable schema) for tooling
        #[arg(long)]
        json: bool,
    },
    /// Print an embedded module's raw `.strok` source
    Show {
        /// Module name (e.g. `figures`, with or without `std/` prefix)
        module: String,
    },
    /// Search module names, shape names, and `@meaning`/`@tags` annotations
    Search {
        /// Case-insensitive search query
        query: String,
        /// Emit matches as JSON (stable schema) for tooling
        #[arg(long)]
        json: bool,
    },
}
