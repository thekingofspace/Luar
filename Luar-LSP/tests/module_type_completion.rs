use luar_lsp::completion::{self, FileView};
use luar_lsp::project::Project;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

static COUNTER: AtomicU32 = AtomicU32::new(0);

fn temp_project(files: &[(&str, &str)]) -> (PathBuf, Project) {
    let id = COUNTER.fetch_add(1, Ordering::SeqCst);
    let root = std::env::temp_dir().join(format!("luar_modtc_{}_{}", std::process::id(), id));
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

fn labels_at(p: &Project, main: &PathBuf, src: &str, line0: usize, col: usize) -> Vec<String> {
    let view = FileView::from_project(p, main).unwrap();
    completion::complete(&view, src, line0, col)
        .into_iter()
        .map(|i| i.label)
        .collect()
}

#[test]
fn module_dot_in_type_position_lists_exported_types() {
    let src = "local Enum = require(\"./enums\")\nlocal test: Enum.";
    let (root, p) = temp_project(&[
        (
            "enums.luar",
            "export type EasingStyle = \"Linear\" | \"Quad\"\nexport type Direction = \"In\" | \"Out\"\nreturn {}",
        ),
        ("main.luar", src),
    ]);
    let main = root.join("main.luar");
    let col = src.lines().last().unwrap().chars().count();
    let labels = labels_at(&p, &main, src, 1, col);
    assert!(labels.contains(&"EasingStyle".to_string()), "{labels:?}");
    assert!(labels.contains(&"Direction".to_string()), "{labels:?}");
}

#[test]
fn module_dot_without_space_after_colon() {
    let src = "local Enum = require(\"./enums\")\nlocal test:Enum.";
    let (root, p) = temp_project(&[
        (
            "enums.luar",
            "export type EasingStyle = \"Linear\" | \"Quad\"\nreturn {}",
        ),
        ("main.luar", src),
    ]);
    let main = root.join("main.luar");
    let col = src.lines().last().unwrap().chars().count();
    let labels = labels_at(&p, &main, src, 1, col);
    assert!(labels.contains(&"EasingStyle".to_string()), "{labels:?}");
}

#[test]
fn accessor_return_annotations_offer_types() {
    let bodies = [
        "class Person {\n    private Age: number = 0\n\n    get realAge(): ",
        "class Person {\n    set realAge(value): ",
        "class Person {\n    operator +(): ",
    ];
    for src in bodies {
        let (root, p) = temp_project(&[("main.luar", src)]);
        let main = root.join("main.luar");
        let line0 = src.lines().count() - 1;
        let col = src.lines().last().unwrap().chars().count();
        let labels = labels_at(&p, &main, src, line0, col);
        assert!(labels.contains(&"string".to_string()), "{src:?}: {labels:?}");
        assert!(labels.contains(&"number".to_string()), "{src:?}: {labels:?}");
    }
}

#[test]
fn accessor_param_annotations_offer_types() {
    let bodies = [
        "class Person {\n    set realAge(value: ",
        "class Person {\n    set realAge(value:",
        "class Person {\n    operator +(other:",
        "class Person {\n    constructor(name:",
    ];
    for src in bodies {
        let (root, p) = temp_project(&[("main.luar", src)]);
        let main = root.join("main.luar");
        let line0 = src.lines().count() - 1;
        let col = src.lines().last().unwrap().chars().count();
        let labels = labels_at(&p, &main, src, line0, col);
        assert!(labels.contains(&"number".to_string()), "{src:?}: {labels:?}");
    }
}

#[test]
fn module_dot_partial_type_name_filters() {
    let src = "local Enum = require(\"./enums\")\nlocal test: Enum.Eas";
    let (root, p) = temp_project(&[
        (
            "enums.luar",
            "export type EasingStyle = \"Linear\" | \"Quad\"\nreturn {}",
        ),
        ("main.luar", src),
    ]);
    let main = root.join("main.luar");
    let col = src.lines().last().unwrap().chars().count();
    let labels = labels_at(&p, &main, src, 1, col);
    assert!(labels.contains(&"EasingStyle".to_string()), "{labels:?}");
}
