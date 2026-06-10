use luar_lsp::project::Project;
use luar_lsp::types::Type;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

static COUNTER: AtomicU32 = AtomicU32::new(0);

fn temp_project(files: &[(&str, &str)]) -> (PathBuf, Project) {
    let id = COUNTER.fetch_add(1, Ordering::SeqCst);
    let root = std::env::temp_dir().join(format!("luar_luardf_{}_{}", std::process::id(), id));
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

#[test]
fn luard_table_field_function_is_ambient_and_typed() {
    let (root, p) = temp_project(&[
        (
            "lib/zlib.luard",
            "--solves the type of object\nZLib.TestArc = function(input: boolean): boolean\nend\n",
        ),
        (
            "main.luar",
            "local r = ZLib.TestArc(true)\nlocal z = ZLib\n",
        ),
    ]);
    let main = root.join("main.luar");
    let info = p.file(&main).unwrap();
    assert_eq!(info.analysis.type_of("r"), Some(&Type::Boolean));
    match info.analysis.type_of("z") {
        Some(Type::Table(tt)) => {
            let (_, ty) = tt
                .fields
                .iter()
                .find(|(n, _)| n == "TestArc")
                .expect("TestArc field missing");
            match ty {
                Type::Function(Some(sig)) => {
                    assert_eq!(sig.params.len(), 1);
                    assert_eq!(sig.params[0].name, "input");
                    assert_eq!(sig.params[0].ty, Type::Boolean);
                    assert_eq!(sig.returns, vec![Type::Boolean]);
                }
                other => panic!("expected typed function, got {other}"),
            }
        }
        other => panic!("expected ambient table, got {other:?}"),
    }
}

#[test]
fn luard_declared_table_gets_field_functions() {
    let (root, p) = temp_project(&[
        (
            "lib/zlib.luard",
            "ZLib = {}\nZLib.Speed = function(amount: number): number\nend\n",
        ),
        ("main.luar", "local s = ZLib.Speed(2)\n"),
    ]);
    let main = root.join("main.luar");
    let info = p.file(&main).unwrap();
    assert_eq!(info.analysis.type_of("s"), Some(&Type::Number));
}

#[test]
fn local_assigned_anonymous_function_gets_annotations() {
    let (root, p) = temp_project(&[(
        "main.luar",
        "local test = function(input: boolean): boolean\n    return input\nend\nlocal out = test(true)\n",
    )]);
    let main = root.join("main.luar");
    let info = p.file(&main).unwrap();
    match info.analysis.type_of("test") {
        Some(Type::Function(Some(sig))) => {
            assert_eq!(sig.params[0].ty, Type::Boolean);
            assert_eq!(sig.returns, vec![Type::Boolean]);
        }
        other => panic!("expected typed function, got {other:?}"),
    }
    assert_eq!(info.analysis.type_of("out"), Some(&Type::Boolean));
}
