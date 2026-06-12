# LUAR

LUAR is a small, class-based scripting language inspired by Lua and Luau. It has
a tree-walking interpreter written in Rust, a static checker (Ferrite), a
garbage collector, a precompilation format, a host embedding API, and a VS Code
language server.

- Full documentation: the `docs/` folder (a static HTML site, also publishable
  to GitHub Pages).
- Repository: https://github.com/thekingofspace/Luar

## Repository layout

| Folder | What it is |
| --- | --- |
| `Luar/` | The language crate: lexer, parser, interpreter, Ferrite, GC, precompiler, embedding API. |
| `test/` | A small runner crate that depends on `Luar` and exposes a CLI (`run` / `compile`). |
| `luar LSP/` | The VS Code extension: Rust language server (`server/`), TypeScript client (`client/`), grammar, and `package.json`. |
| `docs/` | The documentation site (plain HTML). |

## Requirements

- Rust (stable). Built and tested with rustc 1.95.
- Node.js (only needed to build the VS Code extension).

## Build and test the language

Everything for the core language lives in `Luar/`.

```sh
cd Luar
cargo build          # compile the crate
cargo test           # run the test suite
```

## Run a script

The `test/` crate is a runner that links against the language crate. Use it to
execute `.luar` files while developing.

```sh
cd test

# run a source file
cargo run -- run path/to/script.luar

# precompile a source file to a .luarc, then run it
cargo run -- compile path/to/script.luar path/to/script.luarc
cargo run -- run path/to/script.luarc

# with no arguments it runs ./script.luar next to the crate
cargo run
```

Relative `require(...)` paths resolve against the script's own folder. See the
docs for folder modules (`init.luar`), `@self`, and `.luarrc` aliases.

## Build the VS Code extension

The extension bundles the Rust language server binary and a TypeScript client.

```sh
cd "luar LSP"
npm install
npm run build        # builds the server (cargo) and the client (esbuild)
```

To package and install it locally:

```sh
npx vsce package --allow-missing-repository
code --install-extension luar-lang-<version>.vsix --force
```

Then reload VS Code. The extension activates on `.luar` and `.luard` files and
provides completion, hover documentation, inlay hints, semantic highlighting, and
live Ferrite diagnostics.

## View the documentation locally

The docs are plain HTML that load their pages over HTTP. Serve the folder with
any static web server:

```sh
cd docs
python -m http.server 8000   # or any static server
# then open http://localhost:8000
```

## License

Luar is released under the [MIT License](LICENSE).
