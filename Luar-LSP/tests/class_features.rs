use luar_lsp::completion::{self, FileView};
use luar_lsp::project::Project;
use luar_lsp::types::Type;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

static COUNTER: AtomicU32 = AtomicU32::new(0);

const VAR_CLASS: &str = "class Var {\n    public Test:boolean = false\n    public Name:string = \"\"\n    private Int:boolean = false\n    protected Prot:number = 1\n\n    --Use this as a test\n    public function Grag(Input:boolean)\n        \n    end\n\n    private function Hidden()\n    end\n}\n";

fn temp_project(files: &[(&str, &str)]) -> (PathBuf, Project) {
    let id = COUNTER.fetch_add(1, Ordering::SeqCst);
    let root = std::env::temp_dir().join(format!(
        "luar_lsp_class_{}_{}",
        std::process::id(),
        id
    ));
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
fn self_dot_completes_own_members() {
    let src = format!("{VAR_CLASS}");
    let with_self = src.replace("        \n", "        self.\n");
    let (root, mut p) = temp_project(&[("main.luar", &src)]);
    let main = root.join("main.luar");
    p.update_file(&main, with_self.clone());
    let names = labels_at(&p, &main, &with_self, 8, 13);
    assert!(names.contains(&"Test".to_string()), "{names:?}");
    assert!(names.contains(&"Name".to_string()));
    assert!(names.contains(&"Int".to_string()), "private visible via self");
    assert!(names.contains(&"Prot".to_string()));
}

#[test]
fn self_colon_completes_private_methods() {
    let src = VAR_CLASS.replace("        \n", "        self:\n");
    let (root, p) = temp_project(&[("main.luar", &src)]);
    let main = root.join("main.luar");
    let names = labels_at(&p, &main, &src, 8, 13);
    assert!(names.contains(&"Grag".to_string()), "{names:?}");
    assert!(names.contains(&"Hidden".to_string()));
}

#[test]
fn private_members_hidden_outside_class() {
    let src = format!("{VAR_CLASS}\nlocal v = Var()\nlocal q = v.");
    let (root, p) = temp_project(&[("main.luar", &src)]);
    let main = root.join("main.luar");
    let line0 = src.lines().count() - 1;
    let names = labels_at(&p, &main, &src, line0, 12);
    assert!(names.contains(&"Test".to_string()), "{names:?}");
    assert!(!names.contains(&"Int".to_string()), "private leaked: {names:?}");
    assert!(!names.contains(&"Prot".to_string()), "protected leaked");
    let colon_src = format!("{VAR_CLASS}\nlocal v = Var()\nv:");
    let (root2, p2) = temp_project(&[("main.luar", &colon_src)]);
    let main2 = root2.join("main.luar");
    let line0 = colon_src.lines().count() - 1;
    let names = labels_at(&p2, &main2, &colon_src, line0, 2);
    assert!(names.contains(&"Grag".to_string()));
    assert!(!names.contains(&"Hidden".to_string()), "private method leaked");
}

#[test]
fn protected_visible_in_subclass_self() {
    let src = format!(
        "{VAR_CLASS}\nclass Sub extends Var {{\n    public function go()\n        self.\n    end\n}}\n"
    );
    let (root, p) = temp_project(&[("main.luar", &src)]);
    let main = root.join("main.luar");
    let self_line = src.lines().position(|l| l.contains("self.")).unwrap();
    let names = labels_at(&p, &main, &src, self_line, 13);
    assert!(names.contains(&"Test".to_string()), "{names:?}");
    assert!(names.contains(&"Prot".to_string()), "protected hidden in subclass");
    assert!(!names.contains(&"Int".to_string()), "parent private leaked to subclass");
}

#[test]
fn inherited_members_complete_on_subclass_instance() {
    let src = format!(
        "{VAR_CLASS}\nclass Varadic extends Var {{\n}}\n\nlocal test = Varadic()\nlocal x = test.\n"
    );
    let (root, p) = temp_project(&[("main.luar", &src)]);
    let main = root.join("main.luar");
    let dot_line = src.lines().position(|l| l.contains("test.")).unwrap();
    let names = labels_at(&p, &main, &src, dot_line, 15);
    assert!(names.contains(&"Test".to_string()), "inherited field missing: {names:?}");
    assert!(names.contains(&"Name".to_string()));
    let colon_src = src.replace("local x = test.", "test:");
    let (root2, p2) = temp_project(&[("main.luar", &colon_src)]);
    let main2 = root2.join("main.luar");
    let colon_line = colon_src.lines().position(|l| l == "test:").unwrap();
    let names = labels_at(&p2, &main2, &colon_src, colon_line, 5);
    assert!(names.contains(&"Grag".to_string()), "inherited method missing: {names:?}");
}

#[test]
fn unknown_parent_class_reports_warning() {
    let src = format!("{VAR_CLASS}\nclass Varadic extends var {{\n}}\n");
    let (root, p) = temp_project(&[("main.luar", &src)]);
    let info = p.file(&root.join("main.luar")).unwrap();
    let warn = info
        .diagnostics
        .iter()
        .find(|d| d.message.contains("unknown class 'var'"))
        .expect("warning for unknown parent");
    assert_eq!(warn.severity, 2);
    assert!(warn.line > 1);
}

#[test]
fn syntax_error_reports_diagnostic() {
    let src = "local x = (1 + \n";
    let (root, p) = temp_project(&[("main.luar", src)]);
    let info = p.file(&root.join("main.luar")).unwrap();
    assert!(
        info.diagnostics.iter().any(|d| d.severity == 1),
        "no parse diagnostic: {:?}",
        info.diagnostics
    );
}

#[test]
fn no_completions_inside_comments() {
    let src = "local x = 1\n-- typing here \n";
    let (root, p) = temp_project(&[("main.luar", src)]);
    let main = root.join("main.luar");
    let names = labels_at(&p, &main, src, 1, 15);
    assert!(names.is_empty(), "completions inside comment: {names:?}");
}

#[test]
fn call_chains_resolve_through_returns() {
    let src = "class Builder {\n    public Name:string = \"\"\n\n    public function with(n)\n        return self\n    end\n}\n\nlocal b = Builder()\nlocal x = b:with(\"a\"):with(\"b\").\n";
    let (root, p) = temp_project(&[("main.luar", src)]);
    let main = root.join("main.luar");
    let dot_line = src.lines().count() - 1;
    let col = src.lines().nth(dot_line).unwrap().chars().count();
    let names = labels_at(&p, &main, src, dot_line, col);
    assert!(names.contains(&"Name".to_string()), "{names:?}");
    let colon_src = src.replace(":with(\"b\").", ":with(\"b\"):");
    let (root2, p2) = temp_project(&[("main.luar", &colon_src)]);
    let main2 = root2.join("main.luar");
    let col = colon_src.lines().nth(dot_line).unwrap().chars().count();
    let names = labels_at(&p2, &main2, &colon_src, dot_line, col);
    assert!(names.contains(&"with".to_string()), "{names:?}");
}

#[test]
fn class_constructor_call_chain() {
    let src = "class Person {\n    public Name:string = \"\"\n}\nlocal n = Person().\n";
    let (root, p) = temp_project(&[("main.luar", src)]);
    let main = root.join("main.luar");
    let names = labels_at(&p, &main, src, 3, 19);
    assert!(names.contains(&"Name".to_string()), "{names:?}");
}

#[test]
fn dangling_self_keeps_file_analyzed() {
    let src = "class Var {\n    public Test:boolean = false\n\n    public function Grag(Input:boolean)\n        self.\n    end\n}\nlocal v = Var()\n";
    let (root, p) = temp_project(&[("main.luar", src)]);
    let main = root.join("main.luar");
    let info = p.file(&main).unwrap();
    assert!(info.analysis.classes.contains_key("Var"));
    assert_eq!(
        info.analysis.type_of("v"),
        Some(&Type::Instance("Var".to_string()))
    );
}

#[test]
fn module_update_propagates_to_dependents() {
    let (root, mut p) = temp_project(&[
        ("shapes.luar", "return { size = 1 }"),
        (
            "main.luar",
            "local shapes = require(\"./shapes\")\nlocal s = shapes.size",
        ),
    ]);
    let main = root.join("main.luar");
    let shapes = root.join("shapes.luar");
    assert_eq!(
        p.file(&main).unwrap().analysis.type_of("s"),
        Some(&Type::Number)
    );
    p.update_file(&shapes, "return { size = \"big\" }".to_string());
    assert_eq!(
        p.file(&main).unwrap().analysis.type_of("s"),
        Some(&Type::String),
        "dependent file did not pick up module change"
    );
}
