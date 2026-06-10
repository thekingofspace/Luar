use luar_lsp::completion::{self, FileView};
use luar_lsp::project::Project;
use luar_lsp::types::Type;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

static COUNTER: AtomicU32 = AtomicU32::new(0);

fn temp_project(files: &[(&str, &str)]) -> (PathBuf, Project) {
    let id = COUNTER.fetch_add(1, Ordering::SeqCst);
    let root = std::env::temp_dir().join(format!("luar_stab_{}_{}", std::process::id(), id));
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

fn labels_at(p: &Project, path: &PathBuf, src: &str, line0: usize, col0: usize) -> Vec<String> {
    let view = FileView::from_project(p, path).expect("file");
    completion::complete(&view, src, line0, col0)
        .into_iter()
        .map(|i| i.label)
        .collect()
}

#[test]
fn bare_partial_word_after_const() {
    let src = "const test = \"Varg\"\n\ntes";
    let (root, p) = temp_project(&[("main.luar", src)]);
    let main = root.join("main.luar");
    let names = labels_at(&p, &main, src, 2, 3);
    assert!(names.contains(&"test".to_string()), "{} items", names.len());
}

#[test]
fn types_survive_broken_edit() {
    let clean = "class Var {\n    public Test:boolean = false\n}\nconst v = Var()\nlocal n = 5\n";
    let (root, mut p) = temp_project(&[("main.luar", clean)]);
    let main = root.join("main.luar");
    assert_eq!(
        p.file(&main).unwrap().analysis.type_of("n"),
        Some(&Type::Number)
    );

    let broken = "class Var {\n    public Test:boolean = false\n}\nconst v = Var()\nlocal n = 5\nlocal q = (1 +\n";
    p.update_file(&main, broken.to_string());
    let info = p.file(&main).unwrap();
    assert!(info.analysis.classes.contains_key("Var"), "class lost mid-edit");
    assert_eq!(info.analysis.type_of("n"), Some(&Type::Number), "binding lost mid-edit");
    assert_eq!(
        info.analysis.type_of("v"),
        Some(&Type::Instance("Var".to_string()))
    );
}

#[test]
fn immutable_reassign_flagged() {
    let src = "const fixed = 1\nfixed = 2\nfixed = nil\nlocal open = 1\nopen = 2\nbare = 5\nbare = 7\n";
    let (root, p) = temp_project(&[("main.luar", src)]);
    let main = root.join("main.luar");
    let info = p.file(&main).unwrap();
    let immutable_errors: Vec<&luar_lsp::Diagnostic> = info
        .diagnostics
        .iter()
        .filter(|d| d.message.contains("immutable"))
        .collect();
    assert_eq!(immutable_errors.len(), 2, "{:?}", info.diagnostics);
    assert!(immutable_errors.iter().any(|d| d.line == 2 && d.message.contains("'fixed'")));
    assert!(immutable_errors.iter().any(|d| d.line == 7 && d.message.contains("'bare'")));
}

#[test]
fn type_position_offers_type_functions_and_generics() {
    let src = "local function Get<i>(var): ";
    let (root, p) = temp_project(&[("main.luar", src)]);
    let main = root.join("main.luar");
    let names = labels_at(&p, &main, src, 0, 29);
    assert!(names.contains(&"keyof".to_string()), "{names:?}");
    assert!(names.contains(&"ValueOf".to_string()));
    assert!(names.contains(&"i".to_string()), "generic param missing: {names:?}");
}

#[test]
fn generic_param_offered_in_param_annotation() {
    let src = "local function Get<i>(var: ";
    let (root, p) = temp_project(&[("main.luar", src)]);
    let main = root.join("main.luar");
    let names = labels_at(&p, &main, src, 0, 27);
    assert!(names.contains(&"i".to_string()), "{names:?}");
}

#[test]
fn vararg_function_with_table_collect() {
    let src = "local function test(...)\n    local var = {...}\n    return var\nend\nlocal r = test(1, 2)\n";
    let (root, p) = temp_project(&[("main.luar", src)]);
    let main = root.join("main.luar");
    let info = p.file(&main).unwrap();
    assert!(
        !info.diagnostics.iter().any(|d| d.severity == 1),
        "vararg fn should parse cleanly: {:?}",
        info.diagnostics
    );
    match info.analysis.type_of("test") {
        Some(Type::Function(Some(sig))) => assert!(sig.is_vararg),
        other => panic!("expected vararg function, got {other:?}"),
    }
}

#[test]
fn typed_vararg_annotation_accepted() {
    let src = "local function test(...: any)\n    local var = {...}\nend\nlocal function nums(...: number)\nend\n";
    let (root, p) = temp_project(&[("main.luar", src)]);
    let main = root.join("main.luar");
    let info = p.file(&main).unwrap();
    assert!(
        !info.diagnostics.iter().any(|d| d.severity == 1),
        "typed vararg should parse cleanly: {:?}",
        info.diagnostics
    );
    assert!(matches!(
        info.analysis.type_of("test"),
        Some(Type::Function(Some(_)))
    ));
}

#[test]
fn variadic_generics_tolerated() {
    let src = "local function pack<t...>(...: t)\n    return {...}\nend\ntype Fn = (...any) -> ()\n";
    let (root, p) = temp_project(&[("main.luar", src)]);
    let main = root.join("main.luar");
    let info = p.file(&main).unwrap();
    assert!(
        !info.diagnostics.iter().any(|d| d.severity == 1),
        "variadic generics should not error: {:?}",
        info.diagnostics
    );
}

#[test]
fn alias_generics_offered_in_body() {
    let src = "type Box<T, U> = {\n    value: ";
    let (root, p) = temp_project(&[("main.luar", src)]);
    let main = root.join("main.luar");
    let names = labels_at(&p, &main, src, 1, 11);
    assert!(names.contains(&"T".to_string()), "{names:?}");
    assert!(names.contains(&"U".to_string()));
}

#[test]
fn cast_on_declaration_sets_binding_type() {
    let src = "local test = nil::ToBasic<{}>
local n = nil::number
";
    let (root, p) = temp_project(&[("main.luar", src)]);
    let main = root.join("main.luar");
    let info = p.file(&main).unwrap();
    assert_eq!(info.analysis.type_of("test").map(|t| t.to_string()), Some("table".to_string()));
    assert_eq!(info.analysis.type_of("n"), Some(&Type::Number));
    assert!(info.annotations.cast_vars.contains(&("test".to_string(), 1)));
    assert!(info.annotations.cast_vars.contains(&("n".to_string(), 2)));
}

#[test]
fn parenthesized_cast_member_completion() {
    let src = "type T = { this: number, that: string }
local value = nil
local x = (value::T).";
    let (root, p) = temp_project(&[("main.luar", src)]);
    let main = root.join("main.luar");
    let col = src.lines().last().unwrap().chars().count();
    let names = labels_at(&p, &main, src, 2, col);
    assert!(names.contains(&"this".to_string()), "{names:?}");
    assert!(names.contains(&"that".to_string()));
}

#[test]
fn parenthesized_cast_parses_in_language() {
    let src = "local value = nil
local x = (value::any).this
";
    let program = luar_lsp::parse_source_safe(src).expect("(v::T).field should parse");
    assert_eq!(program.len(), 2);
}

#[test]
fn init_dot_resolves_to_parent_dir() {
    let (root, p) = temp_project(&[
        ("other.luar", "return 41"),
        ("pack/init.luar", "local o = require(\"./other\")
return o"),
        ("pack/inner.luar", "return 5"),
    ]);
    let init = root.join("pack").join("init.luar");
    match p.resolve_require(&init, "./other") {
        luar_lsp::project::RequireTarget::Module(t) => {
            assert!(t.ends_with("other.luar"), "{t:?}");
            assert!(!t.to_string_lossy().contains("pack"), "resolved inside pack: {t:?}");
        }
        other => panic!("expected module, got {other:?}"),
    }
    let info = p.file(&init).unwrap();
    assert_eq!(info.analysis.type_of("o"), Some(&Type::Number));
    let inner = root.join("pack").join("inner.luar");
    match p.resolve_require(&inner, "./init") {
        luar_lsp::project::RequireTarget::Module(t) => assert!(t.ends_with("init.luar")),
        other => panic!("non-init ./ changed: {other:?}"),
    }
}
