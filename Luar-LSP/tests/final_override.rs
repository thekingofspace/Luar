use luar_lsp::project::Project;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

static COUNTER: AtomicU32 = AtomicU32::new(0);

fn temp_project(files: &[(&str, &str)]) -> (PathBuf, Project) {
    let id = COUNTER.fetch_add(1, Ordering::SeqCst);
    let root = std::env::temp_dir().join(format!("luar_final_{}_{}", std::process::id(), id));
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

fn messages(p: &Project, file: &PathBuf) -> Vec<String> {
    p.file(file)
        .unwrap()
        .diagnostics
        .iter()
        .map(|d| d.message.clone())
        .collect()
}

#[test]
fn overriding_a_final_method_is_an_error() {
    let src = "class Inst {\n    final public function Test()\n    end\n}\nclass Rest extends Inst {\n    public function Test()\n    end\n}\n";
    let (root, p) = temp_project(&[("main.luar", src)]);
    let msgs = messages(&p, &root.join("main.luar"));
    assert!(
        msgs.iter()
            .any(|m| m.contains("cannot override final method 'Test'")
                && m.contains("[FinalOverride]")),
        "{msgs:?}"
    );
}

#[test]
fn final_override_diagnostic_points_at_the_subclass_method() {
    let src = "class Inst {\n    final public function Test()\n    end\n}\nclass Rest extends Inst {\n    public function Test()\n    end\n}\n";
    let (root, p) = temp_project(&[("main.luar", src)]);
    let info = p.file(&root.join("main.luar")).unwrap();
    let diag = info
        .diagnostics
        .iter()
        .find(|d| d.message.contains("[FinalOverride]"))
        .expect("expected a FinalOverride diagnostic");
    assert_eq!(diag.line, 6, "{:?}", info.diagnostics);
    assert_eq!(diag.severity, 1);
}

#[test]
fn overriding_a_non_final_method_is_fine() {
    let src = "class Inst {\n    public function Test()\n    end\n}\nclass Rest extends Inst {\n    public function Test()\n    end\n}\n";
    let (root, p) = temp_project(&[("main.luar", src)]);
    let msgs = messages(&p, &root.join("main.luar"));
    assert!(
        !msgs.iter().any(|m| m.contains("[FinalOverride]")),
        "{msgs:?}"
    );
}

#[test]
fn final_methods_are_found_through_grandparents() {
    let src = "class A {\n    final function Lock()\n    end\n}\nclass B extends A {\n}\nclass C extends B {\n    function Lock()\n    end\n}\n";
    let (root, p) = temp_project(&[("main.luar", src)]);
    let msgs = messages(&p, &root.join("main.luar"));
    assert!(
        msgs.iter()
            .any(|m| m.contains("cannot override final method 'Lock'")
                && m.contains("from class 'A'")),
        "{msgs:?}"
    );
}

#[test]
fn extending_a_final_class_is_an_error() {
    let src = "final class Sealed {\n}\nclass Sub extends Sealed {\n}\n";
    let (root, p) = temp_project(&[("main.luar", src)]);
    let msgs = messages(&p, &root.join("main.luar"));
    assert!(
        msgs.iter()
            .any(|m| m.contains("cannot extend final class 'Sealed'")
                && m.contains("[FinalOverride]")),
        "{msgs:?}"
    );
}

#[test]
fn final_override_respects_disable_directive() {
    let src = "--#disable FinalOverride\nclass Inst {\n    final function Test()\n    end\n}\nclass Rest extends Inst {\n    function Test()\n    end\n}\n";
    let (root, p) = temp_project(&[("main.luar", src)]);
    let msgs = messages(&p, &root.join("main.luar"));
    assert!(
        !msgs.iter().any(|m| m.contains("[FinalOverride]")),
        "{msgs:?}"
    );
}

#[test]
fn final_method_in_required_module_is_enforced() {
    let base = "pub class Inst {\n    final public function Test()\n    end\n}\nreturn Inst\n";
    let main = "local Inst = require(\"base\")\nclass Rest extends Inst {\n    public function Test()\n    end\n}\n";
    let (root, p) = temp_project(&[("base.luar", base), ("main.luar", main)]);
    let msgs = messages(&p, &root.join("main.luar"));
    assert!(
        msgs.iter()
            .any(|m| m.contains("cannot override final method 'Test'")),
        "{msgs:?}"
    );
}
