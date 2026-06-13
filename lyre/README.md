# lyre

`lyre` is the standalone runtime for the Luar language. It loads a script, wires up
`require` (with `luari.json` alias support), exposes the built-in `File` and `Folder`
classes, runs the script, and then drains any waiting coroutines.

## Usage

```
lyre [entry]      run a script (default: main.luar)
lyre init [dir]   scaffold a new project
lyre help         show usage
```

- `entry` defaults to `main.luar` in the current directory.
- A bare name resolves to `entry.luar`, then `entry.luarb` (precompiled bytecode).
- A directory resolves to `main.luar`, `main.luarb`, `init.luar`, or `init.luarb` inside it.

```
cargo run -- scripts/main.luar
```

## `lyre init`

`lyre init [dir]` scaffolds a project in `dir` (default: the current directory). It
writes three files, never overwriting anything that already exists:

- `lyre.luard` — declarations for the built-in `File`/`Folder` classes so the editor
  knows their methods and types.
- `luari.json` — a config that imports `lyre.luard` (via its `"luard"` key) so the
  language server picks it up.
- `main.luar` — a starter script that uses `File` and `Folder`.

## Built-in `File` and `Folder` classes

`lyre` registers two global classes. Use them directly or `extend` them to add your
own typed helpers. All paths are resolved **relative to the file that is running**:

- `./name` — a sibling of the current file
- `../name` — one folder up
- `.../name` — two folders up (each extra leading dot climbs one more)

```luar
class Save extends File {}

local s = Save()
s:ClaimFile("./save.json")   -- like opening the file; it is now locked
if not s:Exists() then
    s:Create()               -- creates the file (and missing parent folders)
end
s:Write("{}")
print(s:Read())
s:Unclaim()                  -- release the lock so it can be deleted again
```

### `File`

| Method | Description |
| --- | --- |
| `ClaimFile(path)` | Claim a `./`-relative path. While claimed the file is locked open and cannot be deleted. The same path cannot be claimed twice. |
| `Unclaim()` | Release the claim. |
| `Exists()` | True if the claimed file exists on disk. |
| `Create()` | Create the claimed file (and missing parent folders) if absent. |
| `Read()` | Read the whole file as a string. |
| `Write(contents)` | Overwrite the file with `contents`. |
| `Append(contents)` | Append `contents` to the file. |
| `Delete()` | Delete the file — fails while it is still claimed. |
| `Path()` | The resolved absolute path. |

`Read`, `Write`, `Append`, `Exists`, and `Create` require the file to be claimed first.

### `Folder`

| Method | Description |
| --- | --- |
| `Open(path)` | Bind a `./`-relative folder path. |
| `Exists()` | True if the folder exists. |
| `Create()` | Create the folder (and missing parents) if absent. |
| `ListFiles()` | Sorted array of file names directly inside the folder. |
| `ListFolders()` | Sorted array of sub-folder names. |
| `Path()` | The resolved absolute path. |

## Require

`require` works exactly like the language spec:

- `./module` resolves relative to the requiring file's directory; each extra leading dot
  climbs one directory (`../`, `.../`).
- A folder containing `init.luar` resolves to that init file.
- Requiring a folder without `init.luar` returns a table of its modules.
- Inside `init.luar`, `./` resolves from the parent directory and `@self` lists siblings.
- Precompiled modules: if `module.luar` is missing but `module.luarb` exists, the
  precompiled bytecode is loaded instead. No custom require is needed for shipping
  bytecode-only modules — precompile with `luar::precompile_source` and write the bytes
  to `name.luarb`.

## Aliases

Place a `luari.json` (or `luari` / `.luari`) next to your scripts (it is searched upward
from the requiring module's directory):

```json
{
    "aliases": {
        "Settings": "./config/settings"
    }
}
```

Then `require("@Settings")` resolves relative to the config file's directory.
`@Settings/extra` appends a subpath.

## Waiting coroutines (dev-provided `wait`)

The runtime does not add any globals. If you embed Luar (or fork this runtime) you can
add your own `wait` with `luar::blocking`, which detaches the coroutine so
`coroutine.resume` returns immediately and all coroutines run together:

```rust
fn wait(_i: &mut luar::Interpreter, args: Vec<luar::Value>) -> Result<Vec<luar::Value>, String> {
    let secs = match args.first() {
        Some(luar::Value::Int(i)) => *i as f64,
        Some(luar::Value::Float(f)) => *f,
        _ => 0.0,
    };
    luar::blocking(move || std::thread::sleep(std::time::Duration::from_secs_f64(secs)))?;
    Ok(vec![])
}

interp.set_global_fn("wait", wait);
```

- `luar::blocking(f)` inside a coroutine: control returns to the resumer right away, `f`
  runs off to the side, and the coroutine continues once `f` finishes and the host pumps.
  The closure must not touch Luar values.
- `coroutine.status` reports `"waiting"` while detached; such a coroutine cannot be
  resumed manually.
- `luar::run_pending()` blocks until every waiting coroutine has finished (this runtime
  calls it after the script ends).
- `luar::pump_ready()` resumes only the coroutines whose blocking call already finished,
  without waiting (it is also called automatically when `blocking` is used on the main
  thread, so a main-thread `wait` lets waiting coroutines continue).

## Launching coroutines from Rust

The host can launch any Luar function — or an object method with `self` bound — as a
coroutine. The call resumes immediately: if the body finishes (or yields) within the
grace window you get classic behavior, otherwise it keeps running in the background and
`run_pending()` drains it.

```rust
let func = interp.get_global("tick").unwrap();
let co = interp.launch(&func, vec![luar::Value::Int(1)])?;

let player = interp.get_global("player").unwrap();
interp.launch_method(&player, "respawn", vec![])?;

luar::run_pending();
```

Both return the coroutine `Value` (or an error if the callee is not callable or fails
immediately). Launch from the host thread only — not from inside another coroutine.

## Tracking which script a value came from

Hosts can find the source behind any call or value — useful for per-module permissions,
path-scoped registries, or debugging:

- `interp.set_source_path(path)` — tag the main script (require'd modules are tagged
  automatically with their file path).
- `interp.current_source()` — inside a native function, the path of the script that is
  currently running (i.e. who called you).
- `interp.source_of_value(&value)` / `interp.script_of_value(&value)` — the script that
  *created* a function, table, or class, wherever it travels.
- `luar::script_source(script_id)` — resolve a script id to its path.

## Example: a host-side Signal

`cargo run --example signal` runs a full demo: Rust owns a Signal, scripts connect
handler functions to it through a native `on_score(fn)`, the host fires every handler
as a coroutine with `interp.launch`, disconnects one by id, and prints which script
file registered each handler via `source_of_value`.
