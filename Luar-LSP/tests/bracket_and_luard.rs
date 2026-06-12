use luar_lsp::completion::{self, FileView, Item};
use luar_lsp::project::Project;
use luar_lsp::types::Type;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

static COUNTER: AtomicU32 = AtomicU32::new(0);

fn temp_project(files: &[(&str, &str)]) -> (PathBuf, Project) {
    let id = COUNTER.fetch_add(1, Ordering::SeqCst);
    let root = std::env::temp_dir().join(format!("luar_brk_{}_{}", std::process::id(), id));
    let _ = std::fs::remove_dir_all(&root);
    for (rel, content) in files {
        let path = root.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&path, content).unwrap();
    }
    let project = Project::load(&root);
    (root, project)
}

fn items_at(p: &Project, file: &PathBuf, src: &str, line0: usize, col: usize) -> Vec<Item> {
    let view = FileView::from_project(p, file).unwrap();
    completion::complete(&view, src, line0, col)
}

#[test]
fn spaced_key_offered_as_bracket_rewrite_on_dot() {
    let src = "local t = {}\nt[\"easter.Test tes\"] = 1\nt.plain = 2\nlocal v = t.";
    let (root, p) = temp_project(&[("main.luar", src)]);
    let main = root.join("main.luar");
    let col = src.lines().last().unwrap().chars().count();
    let items = items_at(&p, &main, src, 3, col);
    let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
    assert!(labels.contains(&"plain"), "{labels:?}");
    assert!(!labels.contains(&"easter.Test tes"), "{labels:?}");
    let bracket = items
        .iter()
        .find(|i| i.label == "[\"easter.Test tes\"]")
        .expect("bracket rewrite item missing");
    assert_eq!(
        bracket.insert_text.as_deref(),
        Some("[\"easter.Test tes\"]")
    );
    let edit = bracket.extra_edit.as_ref().expect("dot-deleting edit");
    assert_eq!(edit.line0, 3);
    assert_eq!(edit.start_col, 11);
    assert_eq!(edit.end_col, 12);
    assert_eq!(edit.new_text, "");
}

#[test]
fn bracket_chain_resolves_for_dot_completion() {
    let src = "local cfg = { [\"a b\"] = { deep = 5 } }\nlocal x = cfg[\"a b\"].";
    let (root, p) = temp_project(&[("main.luar", src)]);
    let main = root.join("main.luar");
    let col = src.lines().last().unwrap().chars().count();
    let items = items_at(&p, &main, src, 1, col);
    let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
    assert!(labels.contains(&"deep"), "{labels:?}");
}

#[test]
fn double_bracket_chain_offers_keys() {
    let src = "local cfg = { [\"a b\"] = { deep = 5 } }\nlocal x = cfg[\"a b\"][\"";
    let (root, p) = temp_project(&[("main.luar", src)]);
    let main = root.join("main.luar");
    let col = src.lines().last().unwrap().chars().count();
    let items = items_at(&p, &main, src, 1, col);
    let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
    assert!(labels.contains(&"deep"), "{labels:?}");
}

#[test]
fn bracket_index_reads_are_typed() {
    let (root, p) = temp_project(&[(
        "main.luar",
        "local cfg = { [\"a b\"] = { deep = 5 } }\nlocal d = cfg[\"a b\"][\"deep\"]\nlocal e = cfg[\"a b\"].deep\n",
    )]);
    let main = root.join("main.luar");
    let info = p.file(&main).unwrap();
    assert_eq!(info.analysis.type_of("d"), Some(&Type::Number));
    assert_eq!(info.analysis.type_of("e"), Some(&Type::Number));
}

#[test]
fn luard_const_globals_reject_reassignment() {
    let (root, p) = temp_project(&[
        ("lib/engine.luard", "const VERSION = \"1.0\"\nlocal count = 0\nTICK = 1\n"),
        (
            "main.luar",
            "VERSION = \"2.0\"\ncount = 5\nTICK = 2\nprint(VERSION, count, TICK)\n",
        ),
    ]);
    let main = root.join("main.luar");
    let info = p.file(&main).unwrap();
    let immut: Vec<&luar_lsp::Diagnostic> = info
        .diagnostics
        .iter()
        .filter(|d| d.message.contains("cannot reassign immutable"))
        .collect();
    assert_eq!(immut.len(), 1, "{:?}", info.diagnostics);
    assert_eq!(immut[0].line, 1);
}

#[test]
fn local_shadow_of_luard_const_stays_mutable() {
    let (root, p) = temp_project(&[
        ("lib/engine.luard", "const VERSION = \"1.0\"\n"),
        (
            "main.luar",
            "local VERSION = \"x\"\nVERSION = \"y\"\nprint(VERSION)\n",
        ),
    ]);
    let main = root.join("main.luar");
    let info = p.file(&main).unwrap();
    assert!(
        !info
            .diagnostics
            .iter()
            .any(|d| d.message.contains("cannot reassign immutable")),
        "{:?}",
        info.diagnostics
    );
}

#[test]
fn param_and_loop_shadows_of_luard_const_do_not_error() {
    let (root, p) = temp_project(&[
        ("lib/engine.luard", "const VERSION = \"1.0\"\n"),
        (
            "main.luar",
            "local function bump(VERSION)\n    VERSION = VERSION .. \"!\"\n    return VERSION\nend\nprint(bump(\"a\"))\nfor VERSION = 1, 3 do\n    print(VERSION)\nend\n",
        ),
    ]);
    let main = root.join("main.luar");
    let info = p.file(&main).unwrap();
    assert!(
        !info
            .diagnostics
            .iter()
            .any(|d| d.message.contains("cannot reassign immutable")),
        "{:?}",
        info.diagnostics
    );
}

#[test]
fn external_luard_imported_via_luari_json() {
    let id = COUNTER.fetch_add(1, Ordering::SeqCst);
    let external = std::env::temp_dir().join(format!(
        "luar_ext_luard_{}_{}",
        std::process::id(),
        id
    ));
    let _ = std::fs::remove_dir_all(&external);
    std::fs::create_dir_all(&external).unwrap();
    std::fs::write(
        external.join("engine.luard"),
        "class Entity {\n  public id: number = 0\n}\nfunction spawn(prefab: string): Entity\nend\nconst ENGINE = \"1.0\"\n",
    )
    .unwrap();
    let ext_str = external
        .join("engine.luard")
        .to_string_lossy()
        .replace('\\', "/");
    let (root, p) = temp_project(&[
        (
            "luari.json",
            &format!("{{\"luard\": [\"{ext_str}\"]}}"),
        ),
        (
            "main.luar",
            "local e = spawn(\"door\")\nlocal n = e.id\nENGINE = \"2.0\"\nprint(n)\n",
        ),
    ]);
    let main = root.join("main.luar");
    let info = p.file(&main).unwrap();
    assert_eq!(
        info.analysis.type_of("e"),
        Some(&Type::Instance("Entity".to_string()))
    );
    assert_eq!(info.analysis.type_of("n"), Some(&Type::Number));
    assert!(
        info.diagnostics
            .iter()
            .any(|d| d.message.contains("cannot reassign immutable 'ENGINE'")),
        "{:?}",
        info.diagnostics
    );
    let _ = std::fs::remove_dir_all(&external);
}

#[test]
fn luard_imports_reload_with_config() {
    let id = COUNTER.fetch_add(1, Ordering::SeqCst);
    let external = std::env::temp_dir().join(format!(
        "luar_ext_reload_{}_{}",
        std::process::id(),
        id
    ));
    let _ = std::fs::remove_dir_all(&external);
    std::fs::create_dir_all(&external).unwrap();
    std::fs::write(external.join("lib.luard"), "TICKRATE = 60\n").unwrap();
    let (root, mut p) = temp_project(&[("main.luar", "local t = TICKRATE\nprint(t)\n")]);
    let main = root.join("main.luar");
    assert_eq!(
        p.file(&main).unwrap().analysis.type_of("t"),
        Some(&luar_lsp::types::Type::Unknown),
        "global should be unknown before the import exists"
    );
    let ext_str = external
        .join("lib.luard")
        .to_string_lossy()
        .replace('\\', "/");
    std::fs::write(
        root.join("luari.json"),
        format!("{{\"luard\": \"{ext_str}\"}}"),
    )
    .unwrap();
    p.reload_aliases();
    assert_eq!(
        p.file(&main).unwrap().analysis.type_of("t"),
        Some(&Type::Number),
        "external luard not picked up after reload"
    );
    let _ = std::fs::remove_dir_all(&external);
}

#[test]
fn call_field_makes_table_types_callable() {
    let (root, p) = temp_project(&[(
        "main.luar",
        "type Caller = {\n    __call: (x: number) -> string\n}\nlocal c: Caller = make()\nlocal out = c(5)\n",
    )]);
    let main = root.join("main.luar");
    let info = p.file(&main).unwrap();
    assert_eq!(info.analysis.type_of("out"), Some(&Type::String));
}
