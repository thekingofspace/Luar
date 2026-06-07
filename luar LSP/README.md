# LUAR

VS Code support for the **LUAR** language, with a **Rust** backend.

## Features

- **File icon** — `.luar` files show the LUAR logo ([icons/laur_logo.png](icons/laur_logo.png)).
- **Syntax highlighting** — a TextMate grammar colours keywords (`const`, `local`,
  `pub`, `class`, `function`, `if`/`then`/`end`, …), built-ins, strings, numbers
  and comments ([syntaxes/luar.tmLanguage.json](syntaxes/luar.tmLanguage.json)).
- **Auto-indent / editing** — indentation rules, bracket matching, auto-closing
  pairs and `--` comments ([language-configuration.json](language-configuration.json)).
- **Rust language server** ([server/](server/)) providing:
  - **diagnostics** from LUAR's static checker *Ferrite* (unused variables,
    mutating immutables, unreachable code, duplicate keys, …), live as you type;
  - **semantic tokens** so keywords / modifiers / types / built-ins are coloured
    by the backend.

## Architecture

```
luar LSP/
├─ package.json                 manifest: language, grammar, config, client entry
├─ language-configuration.json  indentation / brackets / comments
├─ syntaxes/luar.tmLanguage.json TextMate grammar (highlighting)
├─ icons/laur_logo.png          the .luar file icon
├─ client/src/extension.ts      thin client; launches the Rust server over stdio
├─ server/                      the Rust LSP backend (tower-lsp)
│  └─ src/main.rs               reuses the `luar` crate's lexer + Ferrite linter
├─ bin/server.exe              the bundled, compiled server
└─ out/extension.js            the bundled client
```

The server reuses the `luar` crate directly (`../../Luar`), so the editor and the
language share one lexer and one linter.

## Build

```bash
npm install
npm run build        # builds the Rust server (release) AND bundles the client
```

Individually: `npm run build:server` (cargo + copy into `bin/`) and
`npm run build:client` (esbuild → `out/extension.js`).

## Package / install

```bash
npm install -g @vscode/vsce
vsce package --allow-missing-repository
code --install-extension luar-lang-0.3.0.vsix --force
```

Then reload VS Code. Open a `.luar` file: keywords are coloured, Enter
auto-indents inside blocks, and Ferrite warnings appear as squiggles.
