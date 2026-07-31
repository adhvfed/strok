# Strøk

Strøk is a scriptable vector-design toolkit for people and software agents. Its
plain-text `.strok` format keeps geometry, layout, design tokens, and reusable
components editable, reviewable, and easy to generate.

> [!IMPORTANT]
> Strøk is alpha software. The file format and command-line interface may change
> before 1.0.

## What it does

- Authors vector scenes in a readable text format or through CLI commands.
- Renders SVG and PNG, including icon batches and contact sheets.
- Inspects, queries, measures, audits, and visually compares designs.
- Generates React, Solid, vanilla HTML/CSS, Tailwind, and DTCG token output.
- Exposes a stateless Model Context Protocol server over standard input/output.

## Install

Strøk currently installs from source and requires a stable Rust toolchain:

```sh
git clone https://github.com/adrianhelvik/strok.git
cd strok
cargo install --path strok-cli
```

## Quick start

Agents should first run:

```sh
strok agent-intro
strok guide illustration # or icon, logo, diagram
```

The introduction selects an appropriate effort level and teaches the
render-review loop, including high-resolution focal-region inspection and
`render --outline [id1,id2]` for reading exact geometry through paint and shading.

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
re-renders on every save — parse errors show inline while the last good render
stays visible.

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
library, and MCP server are implemented and covered by automated tests. Strøk
does not currently include a graphical editor. Rasterized text uses system
fonts, so exact glyph rendering can vary between machines; deterministic SVG
snapshots are the authoritative rendering tests.

See [ROADMAP.md](ROADMAP.md) for planned work and
[CONTRIBUTING.md](CONTRIBUTING.md) to contribute.

## License

Licensed under either of:

- Apache License, Version 2.0
- MIT License

at your option.
