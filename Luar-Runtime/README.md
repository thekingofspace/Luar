# Luar Runtime

A minimal standalone runtime for Luar. It loads a script, wires up `require` (with
`luari.json` alias support), runs it, and then drains any waiting coroutines.

## Usage

```
luar-runtime [entry]
```

- `entry` defaults to `main.luar` in the current directory.
- A bare name resolves to `entry.luar`, then `entry.luarb` (precompiled bytecode).
- A directory resolves to `main.luar`, `main.luarb`, `init.luar`, or `init.luarb` inside it.

```
cargo run -- scripts/main.luar
```

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
