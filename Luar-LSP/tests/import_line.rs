use luar_lsp::completion::{self, FileView};
use luar_lsp::project::Project;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

static COUNTER: AtomicU32 = AtomicU32::new(0);

fn temp_project(files: &[(&str, &str)]) -> (PathBuf, Project) {
    let id = COUNTER.fetch_add(1, Ordering::SeqCst);
    let root = std::env::temp_dir().join(format!(
        "luar_import_line_{}_{}",
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

const GEG: &str = "export type Test = { Name: string }\nlocal module = {}\nmodule.Name = \"hi\"\nreturn module\n";

fn labels(p: &Project, main: &PathBuf, src: &str, line0: usize, col0: usize) -> Vec<String> {
    let view = FileView::from_project(p, main).expect("file");
    completion::complete(&view, src, line0, col0)
        .into_iter()
        .map(|i| i.label)
        .collect()
}

#[test]
fn autofill_on_line_below_import() {
    let src = "local greg = require(\"./geg\")\n\nlocal var:Test = nil::any\n\nvar.Name = nil\n";
    let (root, mut p) = temp_project(&[("geg.luar", GEG), ("main.luar", src)]);
    let main = root.join("main.luar");

    let names = labels(&p, &main, src, 1, 0);
    assert!(names.contains(&"greg".to_string()), "empty line: {} items", names.len());

    let typed = "local greg = require(\"./geg\")\nva\nlocal var:Test = nil::any\n\nvar.Name = nil\n";
    p.update_file(&main, typed.to_string());
    let names = labels(&p, &main, typed, 1, 2);
    assert!(
        names.contains(&"greg".to_string()),
        "mid-word under import: {} items: {names:?}",
        names.len()
    );

    let dotted = "local greg = require(\"./geg\")\ngreg.\nlocal var:Test = nil::any\n\nvar.Name = nil\n";
    p.update_file(&main, dotted.to_string());
    let names = labels(&p, &main, dotted, 1, 5);
    assert!(
        names.contains(&"Name".to_string()),
        "greg. under import: {names:?}"
    );
}

#[test]
fn module_type_shows_returned_variable_name() {
    let src = "local greg = require(\"./geg\")\n";
    let (root, p) = temp_project(&[("geg.luar", GEG), ("main.luar", src)]);
    let main = root.join("main.luar");
    let info = p.file(&main).unwrap();
    let ty = info.analysis.type_of("greg").expect("greg binding");
    assert_eq!(ty.to_string(), "module");
    let names = labels(&p, &main, "local greg = require(\"./geg\")\nlocal x = greg.\n", 1, 15);
    let _ = names;
}

#[test]
fn module_type_shows_annotated_name_when_present() {
    let geg = "export type Test = { Name: string }\nlocal module: Test = { Name = \"x\" }\nreturn module\n";
    let src = "local greg = require(\"./geg\")\n";
    let (root, p) = temp_project(&[("geg.luar", geg), ("main.luar", src)]);
    let main = root.join("main.luar");
    let info = p.file(&main).unwrap();
    let ty = info.analysis.type_of("greg").expect("greg binding");
    assert_eq!(ty.to_string(), "Test");
}

#[test]
fn named_module_table_members_still_complete() {
    let src = "local greg = require(\"./geg\")\nlocal x = greg.\n";
    let (root, p) = temp_project(&[("geg.luar", GEG), ("main.luar", src)]);
    let main = root.join("main.luar");
    let names = labels(&p, &main, src, 1, 15);
    assert!(names.contains(&"Name".to_string()), "{names:?}");
}

#[test]
fn member_function_keyword_in_class_with_modifier_prefix() {
    let src = "class MonoBehaviour {\n    public fun\n}\n";
    let (root, p) = temp_project(&[("main.luar", src)]);
    let main = root.join("main.luar");
    let names = labels(&p, &main, src, 1, 14);
    assert!(names.contains(&"function".to_string()), "{names:?}");
    let bare = "class MonoBehaviour {\n    \n}\n";
    let (root2, p2) = temp_project(&[("main.luar", bare)]);
    let main2 = root2.join("main.luar");
    let names = labels(&p2, &main2, bare, 1, 4);
    assert!(names.contains(&"function".to_string()));
    assert!(names.contains(&"private".to_string()));
}
