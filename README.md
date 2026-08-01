# Strøk

Strøk is a scriptable vector-design toolkit for people and software agents. Its
plain-text `.strok` format keeps geometry, layout, design tokens, and reusable
components editable, reviewable, and easy to generate.

## What it does

- Authors vector scenes in a readable text format or through CLI commands.
- Keeps compound silhouettes editable with live `boolean` blocks whose named
  operands recompute on every render.
- Renders SVG and PNG, including icon batches and contact sheets.
- Inspects, queries, measures, audits, and visually compares designs.
- Generates React, Solid, vanilla HTML/CSS, Tailwind, and DTCG token output.
- Exposes a stateless Model Context Protocol server over standard input/output.

## Install

Install Strøk with Homebrew:

```sh
brew install adhvfed/tap/strok
```

Or build it from source with a stable Rust toolchain:

```sh
git clone https://github.com/adhvfed/strok.git
cd strok
cargo install --path strok-cli --locked
```

## Quick start

Agents should first run:

```sh
strok agent-intro
strok guide illustration # or icon, logo, diagram
```

The introduction selects an appropriate effort level and teaches the
render-review loop, including high-resolution focal-region inspection and
`render --outline [id1,id2]` for reading exact geometry through paint and
shading. It also explains how to hand the work to a person for review or direct
editing with `strok watch`.

```sh
strok new face.strok 200x200
strok -f face.strok shape head --template ellipse "fill #ffcc88"
strok -f face.strok place head --shape head --at 0,0 --size 200x200
strok -f face.strok render --out face.png
```

The generated document remains ordinary text:

```text
documentsize 200x200
shape head template=ellipse
  fill #ffcc88
place head shape=head at=0,0 size=200x200
```

Editing by hand? `strok watch face.strok` opens a live browser preview that
re-renders on every save. Its shape editor lets you drag named points and
Bézier controls, split segments, and delete points or retract individual
controls. Hovering the composed preview identifies editable shapes; clicking
one opens its points over the full document, preserving visual context even
for live-boolean operands. Handles stay linked by default, alignment guides help place anchors,
and Shift+C creates an equal, mirrored handle pair. Every gesture is written
back to the plain-text `.strok` source and can be undone with Cmd/Ctrl+Z while
the watcher is running. Arrow keys nudge a selected anchor by one unit, with
Shift for ten units and Alt/Option for a tenth. The canvas supports cursor-anchored pinch or
Cmd/Ctrl-scroll zoom, scroll panning, and keyboard or toolbar zoom controls;
viewport changes never touch the source or edit history. Parse errors show
inline while the last good render stays visible.

Run `strok --help` for the complete workflow and DSL guide. See
[DSL_SPEC.md](DSL_SPEC.md) for the language reference,
[CLI_SPEC.md](CLI_SPEC.md) for command behavior.

## Examples

| Product UI | Shape library | Illustration |
| --- | --- | --- |
| [![Product card](examples/card.png)](examples/card.strok) | [![Shape language](examples/shape-language.png)](examples/shape-language.strok) | [![Quiet Hour](examples/tea.png)](examples/tea.strok) |

The [example gallery](examples/README.md) ranges from a single button to a
tokenized design system, built-in module catalog, and complete illustrations.
Each preview links directly to its editable `.strok` source.

## Project status

The parser, geometry engine, renderer, CLI, framework targets, standard shape
library, and MCP server are implemented and covered by automated tests. The
`watch` editor handles point geometry; scene composition uses the text format
and CLI. Rasterized text uses system fonts. Deterministic SVG snapshots define
the rendering contract.

See [ROADMAP.md](ROADMAP.md) for planned work and
[CONTRIBUTING.md](CONTRIBUTING.md) to contribute.

## License

Licensed under either of:

- Apache License, Version 2.0
- MIT License

at your option.
