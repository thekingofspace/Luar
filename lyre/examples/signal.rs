use luar::{Interpreter, Value};
use std::cell::{Cell, RefCell};
use std::path::PathBuf;

struct Slot {
    id: u64,
    handler: Value,
    source: Option<PathBuf>,
}

thread_local! {
    static SCORE_SIGNAL: RefCell<Vec<Slot>> = const { RefCell::new(Vec::new()) };
    static NEXT_ID: Cell<u64> = const { Cell::new(1) };
}

fn on_score(interp: &mut Interpreter, args: Vec<Value>) -> Result<Vec<Value>, String> {
    let handler = match args.into_iter().next() {
        Some(v @ Value::Function(_)) => v,
        _ => return Err("on_score: expected a function".into()),
    };
    let id = NEXT_ID.with(|n| {
        let id = n.get();
        n.set(id + 1);
        id
    });
    let source = interp
        .source_of_value(&handler)
        .or_else(|| interp.current_source());
    SCORE_SIGNAL.with(|s| s.borrow_mut().push(Slot { id, handler, source }));
    Ok(vec![Value::Int(id as i64)])
}

fn off_score(_interp: &mut Interpreter, args: Vec<Value>) -> Result<Vec<Value>, String> {
    let id = match args.first() {
        Some(Value::Int(i)) => *i as u64,
        _ => return Err("off_score: expected the connection id".into()),
    };
    let removed = SCORE_SIGNAL.with(|s| {
        let mut slots = s.borrow_mut();
        let before = slots.len();
        slots.retain(|slot| slot.id != id);
        before != slots.len()
    });
    Ok(vec![Value::Bool(removed)])
}

fn wait(_interp: &mut Interpreter, args: Vec<Value>) -> Result<Vec<Value>, String> {
    let secs = match args.first() {
        Some(Value::Int(i)) => *i as f64,
        Some(Value::Float(f)) => *f,
        _ => 0.0,
    };
    luar::blocking(move || std::thread::sleep(std::time::Duration::from_secs_f64(secs)))?;
    Ok(vec![])
}

fn fire_score(interp: &mut Interpreter, player: &str, points: i64) {
    let slots: Vec<(u64, Value, Option<PathBuf>)> = SCORE_SIGNAL.with(|s| {
        s.borrow()
            .iter()
            .map(|slot| (slot.id, slot.handler.clone(), slot.source.clone()))
            .collect()
    });
    for (id, handler, source) in slots {
        let from = source
            .as_deref()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "<host>".to_string());
        println!("firing handler #{id} (registered by {from}) as a coroutine");
        if let Err(e) = interp.launch(&handler, vec![Value::str(player), Value::Int(points)]) {
            eprintln!("handler #{id} failed: {e}");
        }
    }
}

fn main() {
    let scripts = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("examples")
        .join("signal_scripts");
    let entry = scripts.join("main.luar");
    let source = match std::fs::read_to_string(&entry) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("cannot read {}: {e}", entry.display());
            std::process::exit(1);
        }
    };

    let mut interp = Interpreter::new();
    interp.set_module_dir(&scripts);
    interp.set_source_path(entry.clone());
    interp.set_global_fn("on_score", on_score);
    interp.set_global_fn("off_score", off_score);
    interp.set_global_fn("wait", wait);

    if let Err(e) = interp.run_source(&source) {
        eprintln!("error: {e}");
        std::process::exit(1);
    }

    println!("-- first fire --");
    fire_score(&mut interp, "ana", 10);
    println!("fire returned immediately; handlers run in the background");

    if let Some(Value::Int(id)) = interp.get_global("stats_conn") {
        let _ = off_score(&mut interp, vec![Value::Int(id)]);
        println!("-- disconnected the stats handler (#{id}) --");
    }

    println!("-- second fire --");
    fire_score(&mut interp, "ben", 5);

    luar::run_pending();
    println!("-- all handlers drained --");
}
