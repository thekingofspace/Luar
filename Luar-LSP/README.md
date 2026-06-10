# Luar-LSP

Editor tooling for the LUAR language: a type resolver, a language server, and
a VS Code extension. Zero external Rust dependencies — it builds on the real
`luar` crate (`../Luar`) so it understands the whole language.

## Layout

| Piece | Where |
|---|---|
| Type resolver library | `src/` (`luar-lsp` crate) |
| CLI type dumper | `cargo run --bin luar-types -- file.luar [--declared]` |
| LSP server | `cargo run --bin luar-lsp` (stdio JSON-RPC) |
| VS Code extension | `editors/code/` (`luar-lsp-0.1.0.vsix`) |
| Sample `.luard` library | `samples/example.luard` |

## Type resolution

Two sources of truth, with a simple rule: **a manually written type always
wins over an inferred one.**

- **Inference** walks the AST: literals, table shapes, `ClassName(...)` →
  instance, enum members, function return types (per-position unions across
  branches), `switch` expression typing, `and`/`or` operand typing, operator
  overloads resolved by symbol *and arity*, getters before fields along the
  whole parent chain, parent fields before mixin fields, conditional
  assignments unioned at the join point, the seeded builtin globals
  (`math`, `string`, `table`, `bit32`, `os`, `coroutine`, …).
- **Annotations** are extracted straight from the tokens (the core luar
  parser scrubs them), so `local x: T`, `x: T = v`, parameter and return
  annotations, class-field annotations, getter return types, and `expr :: T`
  casts all override inference. Annotated parameter types flow into the
  function body.

Basic types: `thread boolean string class enum number nil table` (plus
`function`). `Point` the class displays as `class Point`, an instance as
`Point`, the enum object as `enum Color`, a member as `Color`.

## Type syntax

`src/type_syntax/` implements the documented annotation grammar
(`docs/pages/type-rules.html`) exactly: `->` loosest, then `|`, `&`, postfix
`?`, atoms; literal types `"on" | "off"` / `0 | 1`; table types
`{ x: number }` / `{ name?: string }` / `{ [string]: number }` / `{ T }`;
function types with named params and `...varargs`; generic slots
`Map<string, Array<number>>`.

- `export type Name = T` exports a type from a module; `type A = B` lets
  types equal each other (alias chains are cycle-guarded, generics
  substitute: `type Box<T> = { value: T }`).
- Unions `"This" | "that"` drive autocompletion when checking (`==`) and
  setting (`=`) an annotated binding.
- Intersections `A & B` fuse record types: the merged table carries the
  fields of both sides.

## Projects, modules, require

`Project::load(root)` indexes every `.luar` file under the root.

- `require("./Name")` — `./` is the file's own directory; each extra dot
  climbs one level (`../` parent, `.../` grandparent, …).
- `require("./dir")` — a folder with `init.luar` resolves to it; a folder
  without one resolves to a table of its modules (name → module return).
- `require` types as whatever the target module returns.
- `local val: modulename.typename` works for types the module `export type`s
  (non-exported aliases stay private).
- **Aliases** come from `luari.json` (or `luari` / `.luari`) at the root:
  `{ "aliases": { "Settings": "./Scenes/Settings" } }` → `require("@Settings")`.
- `@self` is the requiring file's own directory (handy in `init.luar`).

## `.luard` ambient libraries

A `.luard` file is normal LUAR code, but every variable, function, class,
enum, and type it declares is **globally visible in every file of the
project, with no require**. The runtime never loads these files — they exist
for people shipping language-extension libraries. See
`samples/example.luard`.

## LSP server + VS Code extension

Hover (function signatures as `name(param: type, …) -> ret`, plus the `--`
doc comment block written directly above a declaration), completion
(members after `.`, methods after `:`, types after `:`/`::`/`type X =`,
exported types after `module.`, enum variants, require paths including
`@aliases` and directory listings, literal-union strings), and inlay hints.

Settings:

| Setting | Default | Effect |
|---|---|---|
| `luar.inlayHints` | `true` | Show `: type` inlay hints on bindings |
| `luar.showMutability` | `true` | Prefix hints with `imut` / `mut` |
| `luar.autoIndent` | `true` | Pressing Enter after `if … then`, `do`, `function`, `switch`, `case`, `default` inserts the matching `end` — only when the block isn't already closed |
| `luar.serverPath` | `""` | Override the bundled server binary |
| `luar.trace.server` | `off` | LSP trace |

Build & install:

```
cargo build --release
cp target/release/luar-lsp.exe editors/code/server/
cd editors/code && npm install && npm run build && npm run package
code --install-extension luar-lsp-0.1.0.vsix
```

## Robustness

All public analysis entry points run on a 256 MiB worker stack (scoped
threads), alias resolution and the type-syntax parser are depth-capped, class
hierarchies are cycle-guarded, and the LSP repair-parses mid-edit sources by
blanking the offending line and retrying, so completion keeps working while
you type.

## Known limits

- Cross-module instance types flow through `require` returns and
  `module.typename` annotations; arbitrary re-exports of class objects
  through nested tables resolve as far as inference can see.
- Loop-variable annotations (`for i: number = …`) are not yet matched to
  bindings (the AST carries no line for them).
- Two analysis passes propagate require types; chains longer than two hops
  converge on the next edit.
