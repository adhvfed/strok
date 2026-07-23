# Roadmap

Strøk is an alpha toolkit with a working text format, renderer, CLI, framework
targets, and MCP server. The roadmap favors a dependable authoring core before
adding broader interfaces.

## Now: harden the foundation

- Improve parser diagnostics and recovery for malformed documents.
- Expand property and fuzz coverage for geometry transformations.
- Define compatibility and migration rules for the `.strok` format.
- Make installation and release artifacts reproducible across macOS and Linux.

## Next: improve authoring

- Add higher-level reusable layout and path-editing operations.
- Improve SVG import fidelity while preserving understandable structure.
- Expand design-token validation and component output.
- Publish more task-oriented examples and guides.

## Later: broaden the interface

- Explore an interactive editor built on the same document model.
- Add plugin boundaries without compromising deterministic builds.
- Evaluate additional export targets based on demonstrated use cases.

Priorities may change as the format is used in real projects. Issues and
focused proposals are welcome; large changes should begin with a design note.
