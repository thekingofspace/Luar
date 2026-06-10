use luar_lsp::completion::{self, FileView};
use luar_lsp::project::Project;
use luar_lsp::resolve::{Resolved, TypeEnv};
use luar_lsp::type_syntax::{TypeExpr, parse_type};
use luar_lsp::types::Type;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

static COUNTER: AtomicU32 = AtomicU32::new(0);

fn temp_project(files: &[(&str, &str)]) -> (PathBuf, Project) {
    let id = COUNTER.fetch_add(1, Ordering::SeqCst);
    let root = std::env::temp_dir().join(format!("luar_tf_{}_{}", std::process::id(), id));
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

fn env(src: &str) -> TypeEnv {
    TypeEnv::from_source(src).expect("parses")
}

#[test]
fn keyof_yields_literal_union() {
    let e = env("type Shape = { test: boolean, other: string }");
    match e.resolve(&parse_type("keyof<Shape>").unwrap()) {
        Resolved::Structural(TypeExpr::Union(parts)) => {
            assert_eq!(parts.len(), 2);
            assert!(parts.contains(&TypeExpr::StringLit("test".to_string())));
            assert!(parts.contains(&TypeExpr::StringLit("other".to_string())));
        }
        other => panic!("expected literal union, got {other:?}"),
    }
    assert_eq!(
        e.value_type(&parse_type("KeyOf<Shape>").unwrap()),
        Type::union_of(vec![
            Type::StringLit("test".to_string()),
            Type::StringLit("other".to_string()),
        ])
    );
}

#[test]
fn keyof_drives_string_autofill() {
    let src = "type Shape = { test: boolean, other: string }\nlocal k: keyof<Shape> = \"test\"\nk = \"";
    let (root, p) = temp_project(&[("main.luar", src)]);
    let main = root.join("main.luar");
    let view = FileView::from_project(&p, &main).unwrap();
    let items = completion::complete(&view, src, 2, 5);
    let labels: Vec<String> = items.into_iter().map(|i| i.label).collect();
    assert_eq!(labels, vec!["other".to_string(), "test".to_string()]);
}

#[test]
fn valueof_picks_field_type() {
    let e = env("type Shape = { test: boolean, other: string }");
    assert_eq!(
        e.value_type(&parse_type("ValueOf<Shape, \"test\">").unwrap()),
        Type::Boolean
    );
    assert_eq!(
        e.value_type(&parse_type("valueof<Shape, \"other\">").unwrap()),
        Type::String
    );
    assert_eq!(
        e.value_type(&parse_type("ValueOf<Shape, keyof<Shape>>").unwrap()),
        Type::union_of(vec![Type::Boolean, Type::String])
    );
}

#[test]
fn generic_function_returns_arg_type() {
    let (root, p) = temp_project(&[(
        "main.luar",
        "local function identity<T>(x: T): T\n  return x\nend\nlocal n = identity(5)\nlocal s = identity(\"hi\")\n",
    )]);
    let main = root.join("main.luar");
    let info = p.file(&main).unwrap();
    assert_eq!(info.analysis.type_of("n"), Some(&Type::Number));
    assert_eq!(info.analysis.type_of("s"), Some(&Type::String));
}

#[test]
fn generic_arity_mismatch_diagnostic() {
    let (root, p) = temp_project(&[(
        "main.luar",
        "type Test<i, t> = {\n    Name: string,\n    test: i\n}\nlocal x: Test<any> = nil\nlocal ok: Test<number, string> = nil\n",
    )]);
    let main = root.join("main.luar");
    let info = p.file(&main).unwrap();
    let diag = info
        .diagnostics
        .iter()
        .find(|d| d.message.contains("expects 2 generic argument"))
        .expect("arity diagnostic");
    assert_eq!(diag.line, 5);
    assert!(diag.message.contains("got 1"));
    assert_eq!(
        info.diagnostics
            .iter()
            .filter(|d| d.message.contains("generic argument"))
            .count(),
        1
    );
}

#[test]
fn missing_generics_entirely_flagged() {
    let (root, p) = temp_project(&[(
        "main.luar",
        "type Box<T> = { value: T }\nlocal b: Box = nil\n",
    )]);
    let main = root.join("main.luar");
    let info = p.file(&main).unwrap();
    assert!(
        info.diagnostics
            .iter()
            .any(|d| d.message.contains("expects 1 generic argument(s), got 0")),
        "{:?}",
        info.diagnostics
    );
}

#[test]
fn duplicate_enum_variant_flagged() {
    let (root, p) = temp_project(&[(
        "main.luar",
        "enum Grag {\n    test,\n    test\n}\n",
    )]);
    let main = root.join("main.luar");
    let info = p.file(&main).unwrap();
    let diag = info
        .diagnostics
        .iter()
        .find(|d| d.message.contains("duplicate variant 'test'"))
        .expect("duplicate diagnostic");
    assert_eq!(diag.line, 3);
    assert_eq!(diag.severity, 1);
}

#[test]
fn tobasic_widens_literals() {
    let e = env("type Shape = { test: boolean, other: string }
type Lit = \"Var\"");
    assert_eq!(e.value_type(&parse_type("ToBasic<\"Var\">").unwrap()), Type::String);
    assert_eq!(e.value_type(&parse_type("ToBasic<5>").unwrap()), Type::Number);
    assert_eq!(e.value_type(&parse_type("tobasic<Lit>").unwrap()), Type::String);
    assert_eq!(
        e.value_type(&parse_type("ToBasic<keyof<Shape>>").unwrap()),
        Type::String
    );
    assert_eq!(
        e.value_type(&parse_type("ToBasic<\"a\" | 5>").unwrap()),
        Type::union_of(vec![Type::String, Type::Number])
    );
    assert_eq!(e.value_type(&parse_type("ToBasic<number>").unwrap()), Type::Number);
}

#[test]
fn tobasic_and_annotations_handle_boolean_literals() {
    let e = env("local x = 1");
    assert_eq!(e.value_type(&parse_type("ToBasic<true>").unwrap()), Type::Boolean);
    assert_eq!(e.value_type(&parse_type("ToBasic<false>").unwrap()), Type::Boolean);
    assert_eq!(e.value_type(&parse_type("true").unwrap()), Type::Boolean);
    assert_eq!(
        e.value_type(&parse_type("ToBasic<true | 5>").unwrap()),
        Type::union_of(vec![Type::Boolean, Type::Number])
    );
    assert_eq!(e.value_type(&parse_type("ToBasic<boolean>").unwrap()), Type::Boolean);
    assert_eq!(e.value_type(&parse_type("ToBasic<nil>").unwrap()), Type::Nil);
}

#[test]
fn tobasic_widens_all_kinds() {
    let e = env("class Dog {
}
enum Color { Red }
interface Pet { name }");
    assert_eq!(
        e.value_type(&parse_type("ToBasic<{ x: number }>").unwrap()).to_string(),
        "table"
    );
    assert_eq!(
        e.value_type(&parse_type("ToBasic<{}>").unwrap()).to_string(),
        "table"
    );
    assert_eq!(e.value_type(&parse_type("ToBasic<Dog>").unwrap()).to_string(), "class");
    assert_eq!(e.value_type(&parse_type("ToBasic<Color>").unwrap()).to_string(), "enum");
    assert_eq!(
        e.value_type(&parse_type("ToBasic<(number) -> nil>").unwrap()).to_string(),
        "function"
    );
    assert_eq!(
        e.value_type(&parse_type("ToBasic<Dog | Color>").unwrap()).to_string(),
        "class | enum"
    );
    assert_eq!(e.value_type(&parse_type("ToBasic<thread>").unwrap()), Type::Thread);
}
