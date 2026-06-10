use luar_lsp::completion::{self, FileView};
use luar_lsp::project::Project;
use std::path::PathBuf;

const SRC: &str = "local test:boolean = nil\n\ntype test = {\n    Grag:boolean,\n\n}\n\nclass Person {\n    public test:boolean = false\n}\n\nlocal grag:test = Person()\n\n\n\nswitch(\"test\")\n   case \"test\"\n     \n   end\nend\n";

fn setup() -> (PathBuf, Project) {
    let root = std::env::temp_dir().join(format!("luar_user_repro_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("main.luar"), SRC).unwrap();
    let project = Project::load(&root);
    (root, project)
}

fn complete_at(p: &Project, path: &PathBuf, src: &str, line0: usize, col0: usize) -> Vec<String> {
    let view = FileView::from_project(p, path).expect("file");
    completion::complete(&view, src, line0, col0)
        .into_iter()
        .map(|i| i.label)
        .collect()
}

#[test]
fn bindings_present() {
    let (root, p) = setup();
    let info = p.file(&root.join("main.luar")).unwrap();
    let names: Vec<&str> = info.analysis.bindings.iter().map(|b| b.name.as_str()).collect();
    assert!(names.contains(&"test"), "{names:?}");
    assert!(names.contains(&"grag"), "{names:?}");
    assert!(names.contains(&"Person"), "{names:?}");
}

#[test]
fn value_completion_at_end_of_file() {
    let (root, p) = setup();
    let main = root.join("main.luar");
    let mut src = SRC.to_string();
    src.push('\n');
    p.file(&main).unwrap();
    let names = complete_at(&p, &main, &src, 20, 0);
    assert!(names.contains(&"grag".to_string()), "missing grag: {} items", names.len());
    assert!(names.contains(&"test".to_string()), "missing test");
    assert!(names.contains(&"Person".to_string()), "missing Person");
}

#[test]
fn value_completion_inside_case_body() {
    let (root, p) = setup();
    let main = root.join("main.luar");
    let names = complete_at(&p, &main, SRC, 17, 5);
    assert!(names.contains(&"grag".to_string()), "missing grag: {} items", names.len());
}

#[test]
fn type_completion_in_this_file() {
    let (root, p) = setup();
    let main = root.join("main.luar");
    let src = format!("{SRC}local another:");
    let names = complete_at(&p, &main, &src, 20, 14);
    assert!(names.contains(&"boolean".to_string()), "missing boolean: {names:?}");
    assert!(names.contains(&"test".to_string()), "missing alias test");
    assert!(names.contains(&"Person".to_string()), "missing class Person");
}

#[test]
fn member_completion_on_grag() {
    let (root, p) = setup();
    let main = root.join("main.luar");
    let src = format!("{SRC}local v = grag.");
    let names = complete_at(&p, &main, &src, 20, 15);
    assert!(names.contains(&"Grag".to_string()), "missing Grag field: {names:?}");
}
