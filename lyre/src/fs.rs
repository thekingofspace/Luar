//! Built-in `File` and `Folder` classes provided by the lyre runtime.
//!
//! Scripts get these as global classes (just like `print`), so the usual
//! pattern is to *extend* them:
//!
//! ```luar
//! class Config extends File {}
//! local c = Config()
//! c:ClaimFile("./config.json")
//! if not c:Exists() then c:Create() end
//! c:Write("{}")
//! ```
//!
//! All paths are resolved *relative to the file that is currently running*:
//!   - `./name`   -> a sibling of the current file
//!   - `../name`  -> one folder up
//!   - `.../name` -> two folders up (each extra leading dot climbs one more)

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use luar::{Interpreter, NativeClassBuilder, Value};

type NativeResult = Result<Vec<Value>, String>;

/// Process-wide set of currently-claimed file paths. A claimed file behaves
/// like an open handle: it cannot be deleted until it is unclaimed.
fn claimed() -> &'static Mutex<HashSet<PathBuf>> {
    static CLAIMED: OnceLock<Mutex<HashSet<PathBuf>>> = OnceLock::new();
    CLAIMED.get_or_init(|| Mutex::new(HashSet::new()))
}

/// Directory of the script that is currently executing (the "current file").
fn current_dir(interp: &Interpreter) -> PathBuf {
    if let Some(src) = interp.current_source() {
        if let Some(parent) = src.parent() {
            if !parent.as_os_str().is_empty() {
                return parent.to_path_buf();
            }
        }
    }
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

/// Resolve a leading-dot path against `base_dir`.
///
/// `./x` stays in `base_dir`; every extra leading dot climbs one folder, so
/// `../x` is one up and `.../x` is two up.
fn resolve_dotted(base_dir: &Path, raw: &str) -> Result<PathBuf, String> {
    let trimmed = raw.trim();
    let bytes = trimmed.as_bytes();
    let mut dots = 0usize;
    while dots < bytes.len() && bytes[dots] == b'.' {
        dots += 1;
    }
    if dots == 0 {
        return Err(format!(
            "path '{raw}' must be relative to the current file: start it with \
             './' (this file's folder), '../' (one up), '.../' (two up), and so on"
        ));
    }
    if dots < bytes.len() {
        let sep = bytes[dots];
        if sep != b'/' && sep != b'\\' {
            return Err(format!(
                "path '{raw}' is malformed: put a separator after the leading dots, \
                 e.g. './name' or '../name'"
            ));
        }
    }
    let rest = trimmed[dots..].trim_start_matches(['/', '\\']);
    let mut path = base_dir.to_path_buf();
    for _ in 1..dots {
        path.push("..");
    }
    for seg in rest.split(['/', '\\']) {
        if seg.is_empty() || seg == "." {
            continue;
        }
        path.push(seg);
    }
    Ok(path)
}

/// Make a path absolute and lexically normalised (no symlink/canonicalisation,
/// so the result is stable whether or not the file exists yet).
fn absolute(path: &Path) -> PathBuf {
    let mut abs = if path.is_absolute() {
        PathBuf::new()
    } else {
        std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
    };
    for comp in path.components() {
        match comp {
            std::path::Component::ParentDir => {
                abs.pop();
            }
            std::path::Component::CurDir => {}
            other => abs.push(other.as_os_str()),
        }
    }
    abs
}

fn resolve_against_current(interp: &Interpreter, raw: &str) -> Result<PathBuf, String> {
    Ok(absolute(&resolve_dotted(&current_dir(interp), raw)?))
}

fn this_of(args: &[Value], who: &str) -> Result<Value, String> {
    args.first()
        .cloned()
        .ok_or_else(|| format!("{who}: missing receiver (call it with ':' on an instance)"))
}

fn bound_path(this: &Value) -> Option<String> {
    match this.field(&Value::str("__path")) {
        Value::Str(s) if !s.is_empty() => Some(s.to_string()),
        _ => None,
    }
}

fn is_claimed(this: &Value) -> bool {
    matches!(this.field(&Value::str("__claimed")), Value::Bool(true))
}

fn require_claimed(this: &Value) -> Result<String, String> {
    if !is_claimed(this) {
        return Err(
            "this File is not claimed yet — call ClaimFile(\"./...\") first".to_string(),
        );
    }
    bound_path(this).ok_or_else(|| "this File has no claimed path".to_string())
}

fn make_array(items: Vec<String>) -> Value {
    let t = Value::table();
    for (i, s) in items.into_iter().enumerate() {
        let _ = t.set_field(Value::Int(i as i64 + 1), Value::str(s));
    }
    t
}

// ---------------------------------------------------------------------------
// File
// ---------------------------------------------------------------------------

fn file_claim(interp: &mut Interpreter, args: Vec<Value>) -> NativeResult {
    let this = this_of(&args, "ClaimFile")?;
    let raw = args
        .get(1)
        .and_then(Value::as_str)
        .ok_or("ClaimFile expects a path string, e.g. file:ClaimFile(\"./data.txt\")")?
        .to_string();
    if is_claimed(&this) {
        return Err(format!(
            "this File has already claimed '{}'; call Unclaim() before claiming another",
            bound_path(&this).unwrap_or_default()
        ));
    }
    let resolved = resolve_against_current(interp, &raw)?;
    {
        let mut set = claimed().lock().unwrap();
        if set.contains(&resolved) {
            return Err(format!(
                "'{}' is already claimed and cannot be claimed twice (it behaves like an open file)",
                resolved.display()
            ));
        }
        set.insert(resolved.clone());
    }
    this.set_field(Value::str("__path"), Value::str(resolved.to_string_lossy().into_owned()))?;
    this.set_field(Value::str("__claimed"), Value::Bool(true))?;
    Ok(vec![this])
}

fn file_unclaim(_i: &mut Interpreter, args: Vec<Value>) -> NativeResult {
    let this = this_of(&args, "Unclaim")?;
    if !is_claimed(&this) {
        return Err("Unclaim: this File is not claimed".to_string());
    }
    if let Some(p) = bound_path(&this) {
        claimed().lock().unwrap().remove(&PathBuf::from(&p));
    }
    this.set_field(Value::str("__claimed"), Value::Bool(false))?;
    Ok(vec![])
}

fn file_exists(_i: &mut Interpreter, args: Vec<Value>) -> NativeResult {
    let this = this_of(&args, "Exists")?;
    let path = require_claimed(&this)?;
    Ok(vec![Value::Bool(Path::new(&path).is_file())])
}

fn file_create(_i: &mut Interpreter, args: Vec<Value>) -> NativeResult {
    let this = this_of(&args, "Create")?;
    let path = require_claimed(&this)?;
    let p = Path::new(&path);
    if !p.exists() {
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("Create: cannot make folder for '{}': {e}", p.display()))?;
        }
        std::fs::File::create(p)
            .map_err(|e| format!("Create: cannot create '{}': {e}", p.display()))?;
    }
    Ok(vec![this])
}

fn file_read(_i: &mut Interpreter, args: Vec<Value>) -> NativeResult {
    let this = this_of(&args, "Read")?;
    let path = require_claimed(&this)?;
    let contents = std::fs::read_to_string(&path)
        .map_err(|e| format!("Read: cannot read '{path}': {e}"))?;
    Ok(vec![Value::str(contents)])
}

fn file_write(_i: &mut Interpreter, args: Vec<Value>) -> NativeResult {
    let this = this_of(&args, "Write")?;
    let path = require_claimed(&this)?;
    let contents = args.get(1).map(|v| v.to_string()).unwrap_or_default();
    if let Some(parent) = Path::new(&path).parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    std::fs::write(&path, contents).map_err(|e| format!("Write: cannot write '{path}': {e}"))?;
    Ok(vec![this])
}

fn file_append(_i: &mut Interpreter, args: Vec<Value>) -> NativeResult {
    use std::io::Write as _;
    let this = this_of(&args, "Append")?;
    let path = require_claimed(&this)?;
    let contents = args.get(1).map(|v| v.to_string()).unwrap_or_default();
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|e| format!("Append: cannot open '{path}': {e}"))?;
    f.write_all(contents.as_bytes())
        .map_err(|e| format!("Append: cannot write '{path}': {e}"))?;
    Ok(vec![this])
}

fn file_delete(_i: &mut Interpreter, args: Vec<Value>) -> NativeResult {
    let this = this_of(&args, "Delete")?;
    let path = bound_path(&this)
        .ok_or("Delete: this File has no path (call ClaimFile first)")?;
    if claimed().lock().unwrap().contains(&PathBuf::from(&path)) {
        return Err(format!(
            "'{path}' is claimed and cannot be deleted — call Unclaim() first"
        ));
    }
    let p = Path::new(&path);
    if p.is_file() {
        std::fs::remove_file(p).map_err(|e| format!("Delete: cannot delete '{path}': {e}"))?;
    }
    Ok(vec![])
}

fn file_path(_i: &mut Interpreter, args: Vec<Value>) -> NativeResult {
    let this = this_of(&args, "Path")?;
    Ok(vec![Value::str(bound_path(&this).unwrap_or_default())])
}

// ---------------------------------------------------------------------------
// Folder
// ---------------------------------------------------------------------------

fn require_open(this: &Value) -> Result<String, String> {
    bound_path(this).ok_or_else(|| {
        "this Folder is not bound to a path — call Open(\"./...\") first".to_string()
    })
}

fn folder_open(interp: &mut Interpreter, args: Vec<Value>) -> NativeResult {
    let this = this_of(&args, "Open")?;
    let raw = args
        .get(1)
        .and_then(Value::as_str)
        .ok_or("Open expects a path string, e.g. folder:Open(\"./assets\")")?
        .to_string();
    let resolved = resolve_against_current(interp, &raw)?;
    this.set_field(Value::str("__path"), Value::str(resolved.to_string_lossy().into_owned()))?;
    Ok(vec![this])
}

fn folder_exists(_i: &mut Interpreter, args: Vec<Value>) -> NativeResult {
    let this = this_of(&args, "Exists")?;
    let path = require_open(&this)?;
    Ok(vec![Value::Bool(Path::new(&path).is_dir())])
}

fn folder_create(_i: &mut Interpreter, args: Vec<Value>) -> NativeResult {
    let this = this_of(&args, "Create")?;
    let path = require_open(&this)?;
    std::fs::create_dir_all(&path)
        .map_err(|e| format!("Create: cannot make folder '{path}': {e}"))?;
    Ok(vec![this])
}

fn list_entries(path: &str, want_dir: bool, who: &str) -> NativeResult {
    let mut names = Vec::new();
    let rd = std::fs::read_dir(path)
        .map_err(|e| format!("{who}: cannot read folder '{path}': {e}"))?;
    for entry in rd {
        let entry = entry.map_err(|e| format!("{who}: {e}"))?;
        let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
        if is_dir == want_dir {
            names.push(entry.file_name().to_string_lossy().into_owned());
        }
    }
    names.sort();
    Ok(vec![make_array(names)])
}

fn folder_list_files(_i: &mut Interpreter, args: Vec<Value>) -> NativeResult {
    let this = this_of(&args, "ListFiles")?;
    let path = require_open(&this)?;
    list_entries(&path, false, "ListFiles")
}

fn folder_list_folders(_i: &mut Interpreter, args: Vec<Value>) -> NativeResult {
    let this = this_of(&args, "ListFolders")?;
    let path = require_open(&this)?;
    list_entries(&path, true, "ListFolders")
}

fn folder_path(_i: &mut Interpreter, args: Vec<Value>) -> NativeResult {
    let this = this_of(&args, "Path")?;
    Ok(vec![Value::str(bound_path(&this).unwrap_or_default())])
}

// ---------------------------------------------------------------------------

/// Register the built-in `File` and `Folder` classes as globals.
pub fn register(interp: &mut Interpreter) {
    interp.define_class(
        NativeClassBuilder::new("File")
            .field("__path", Value::str(""))
            .field("__claimed", Value::Bool(false))
            .method("ClaimFile", file_claim)
            .method("Unclaim", file_unclaim)
            .method("Exists", file_exists)
            .method("Create", file_create)
            .method("Read", file_read)
            .method("Write", file_write)
            .method("Append", file_append)
            .method("Delete", file_delete)
            .method("Path", file_path),
    );
    interp.define_class(
        NativeClassBuilder::new("Folder")
            .field("__path", Value::str(""))
            .method("Open", folder_open)
            .method("Exists", folder_exists)
            .method("Create", folder_create)
            .method("ListFiles", folder_list_files)
            .method("ListFolders", folder_list_folders)
            .method("Path", folder_path),
    );
}

/// The `.luard` declaration file that `lyre init` writes so the editor knows
/// about the built-in `File` and `Folder` classes.
pub const LUARD_DECLARATIONS: &str = include_str!("lyre.luard");

#[cfg(test)]
mod tests {
    use super::*;

    fn unique_temp_dir() -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static N: AtomicU64 = AtomicU64::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!("lyre_test_{}_{}", std::process::id(), n))
    }

    fn interp_in(dir: &Path) -> Interpreter {
        std::fs::create_dir_all(dir).unwrap();
        let script = dir.join("main.luar");
        std::fs::write(&script, "").unwrap();
        let mut interp = Interpreter::new();
        interp.set_module_dir(dir);
        interp.set_source_path(script);
        register(&mut interp);
        interp
    }

    #[test]
    fn resolve_dotted_climbs_one_folder_per_extra_dot() {
        let base = Path::new("/proj/data");
        assert_eq!(resolve_dotted(base, "./x.txt").unwrap(), PathBuf::from("/proj/data/x.txt"));
        assert_eq!(resolve_dotted(base, "../x.txt").unwrap(), PathBuf::from("/proj/data/../x.txt"));
        assert_eq!(
            resolve_dotted(base, ".../x.txt").unwrap(),
            PathBuf::from("/proj/data/../../x.txt")
        );
        assert!(resolve_dotted(base, "x.txt").is_err());
        assert!(resolve_dotted(base, "..x").is_err());
    }

    #[test]
    fn file_round_trips_through_luar() {
        let dir = unique_temp_dir();
        let mut interp = interp_in(&dir);
        interp
            .run_source(
                r#"class Save extends File {}
                   local s = Save()
                   s:ClaimFile("./data.txt")
                   pub local existed = s:Exists()
                   s:Create()
                   s:Write("hello")
                   s:Append(" world")
                   pub local body = s:Read()
                   s:Unclaim()"#,
            )
            .unwrap();
        assert_eq!(interp.get_global("existed"), Some(Value::Bool(false)));
        assert_eq!(interp.get_global("body"), Some(Value::str("hello world")));
        assert!(dir.join("data.txt").is_file());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn claimed_file_cannot_be_deleted_until_unclaimed() {
        let dir = unique_temp_dir();
        let mut interp = interp_in(&dir);
        interp
            .run_source(
                r#"local f = File()
                   f:ClaimFile("./x.txt")
                   f:Create()
                   pub local blocked = pcall(function() f:Delete() end)
                   f:Unclaim()
                   pub local allowed = pcall(function() f:Delete() end)"#,
            )
            .unwrap();
        assert_eq!(interp.get_global("blocked"), Some(Value::Bool(false)));
        assert_eq!(interp.get_global("allowed"), Some(Value::Bool(true)));
        assert!(!dir.join("x.txt").exists());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn same_path_cannot_be_claimed_twice() {
        let dir = unique_temp_dir();
        let mut interp = interp_in(&dir);
        interp
            .run_source(
                r#"local a = File()
                   a:ClaimFile("./shared.txt")
                   local b = File()
                   pub local second = pcall(function() b:ClaimFile("./shared.txt") end)
                   a:Unclaim()
                   local c = File()
                   pub local afterFree = pcall(function() c:ClaimFile("./shared.txt") end)"#,
            )
            .unwrap();
        assert_eq!(interp.get_global("second"), Some(Value::Bool(false)));
        assert_eq!(interp.get_global("afterFree"), Some(Value::Bool(true)));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn folder_lists_and_creates() {
        let dir = unique_temp_dir();
        let mut interp = interp_in(&dir);
        std::fs::write(dir.join("a.txt"), "a").unwrap();
        std::fs::write(dir.join("b.txt"), "b").unwrap();
        interp
            .run_source(
                r#"local here = Folder()
                   here:Open("./")
                   pub local files = table.concat(here:ListFiles(), ",")
                   local sub = Folder()
                   sub:Open("./made")
                   pub local before = sub:Exists()
                   sub:Create()
                   pub local after = sub:Exists()"#,
            )
            .unwrap();
        assert_eq!(interp.get_global("files"), Some(Value::str("a.txt,b.txt,main.luar")));
        assert_eq!(interp.get_global("before"), Some(Value::Bool(false)));
        assert_eq!(interp.get_global("after"), Some(Value::Bool(true)));
        assert!(dir.join("made").is_dir());
        std::fs::remove_dir_all(&dir).ok();
    }
}
