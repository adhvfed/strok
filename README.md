# Strøk

Strøk is a scriptable vector-design toolkit. Its plain-text `.strok` files are
editable by people and agents and render to SVG or PNG.

<p align="center">
  <a href="examples/chess-pieces/pawn.strok"><img src="examples/chess-pieces/pawn.svg" alt="Pawn" width="72"></a>
  <a href="examples/chess-pieces/knight.strok"><img src="examples/chess-pieces/knight.svg" alt="Knight" width="72"></a>
  <a href="examples/chess-pieces/bishop.strok"><img src="examples/chess-pieces/bishop.svg" alt="Bishop" width="72"></a>
  <a href="examples/chess-pieces/rook.strok"><img src="examples/chess-pieces/rook.svg" alt="Rook" width="72"></a>
  <a href="examples/chess-pieces/queen.strok"><img src="examples/chess-pieces/queen.svg" alt="Queen" width="72"></a>
  <a href="examples/chess-pieces/king.strok"><img src="examples/chess-pieces/king.svg" alt="King" width="72"></a>
</p>

## Install

```sh
brew install adhvfed/tap/strok
```

## Start

```sh
strok agent-intro
strok guide illustration
```

## MCP

Agents with a shell use the CLI directly. For MCP-only runtimes, register the
installed binary as a stdio server:

```sh
claude mcp add strok -- strok mcp-server
```

Or in any client's JSON config:

```json
{ "mcpServers": { "strok": { "command": "strok", "args": ["mcp-server"] } } }
```

Tools work on `.strok` files on disk and return renders as inline images.

[Language](DSL_SPEC.md) · [CLI](CLI_SPEC.md) · [Contributing](CONTRIBUTING.md)

Apache-2.0 or MIT.
