# Luar Language for VS Code

VS Code support for the **Luar** language (`.luar`, `.luard`): TextMate syntax highlighting, smart indentation with automatic `end` insertion, and a client for the `luar-lsp` language server (diagnostics, hover, completion, inlay hints, and more).

## Settings

| Setting | Type | Default | Description |
| --- | --- | --- | --- |
| `luar.inlayHints` | boolean | `true` | Show inlay hints with the resolved type of each binding. |
| `luar.showMutability` | boolean | `true` | Prefix inlay hints with `imut`/`mut` to show binding mutability. |
| `luar.autoIndent` | boolean | `true` | Automatically insert `end` when pressing Enter after a block opener (`if`/`then`, `do`, `function`, `switch`, `case`, `default`), when the block is not already closed. |
| `luar.serverPath` | string | `""` | Path to the `luar-lsp` server executable. Leave empty to use the bundled server. |
| `luar.trace.server` | enum | `"off"` | LSP message tracing: `off`, `messages`, or `verbose`. |

## How the server binary is located

1. If `luar.serverPath` is set to a non-empty path, that executable is used.
2. Otherwise the extension looks for a bundled binary inside the extension folder: `server/luar-lsp.exe` on Windows, `server/luar-lsp` elsewhere. The Rust build copies the binary into `server/` before packaging.

If no server binary is found, the extension shows a single warning and keeps running: syntax highlighting and auto-`end` still work, only LSP-powered features are disabled. Use the **Luar: Restart Language Server** command after installing or pointing to a server.

## Building

```sh
npm install
npm run build      # bundles src/extension.ts to out/extension.js with esbuild
npm run check      # type-checks with tsc --noEmit
npm run watch      # rebuild on change
```

## Packaging

```sh
npm run package    # runs npx @vscode/vsce package and produces a .vsix
```

Place the `luar-lsp` server binary in `server/` first if you want it bundled in the `.vsix`.

## License

This extension and the Luar language are released under the [MIT License](LICENSE).
