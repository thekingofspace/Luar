use luar_lsp::completion::{self, FileView, Item};
use luar_lsp::project::Project;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

static COUNTER: AtomicU32 = AtomicU32::new(0);

fn temp_project(files: &[(&str, &str)]) -> (PathBuf, Project) {
    let id = COUNTER.fetch_add(1, Ordering::SeqCst);
    let root = std::env::temp_dir().join(format!("luar_autoimp_{}_{}", std::process::id(), id));
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

fn import_of(items: &[Item], label: &str) -> Option<(u32, String)> {
    items
        .iter()
        .find(|i| i.label == label)
        .and_then(|i| i.auto_import.clone())
        .map(|ai| (ai.line0, ai.new_text))
}

#[test]
fn module_offered_with_relative_require_at_top() {
    let src = "local x = 1\n";
    let (root, p) = temp_project(&[("main.luar", src), ("util.luar", "return 1")]);
    let main = root.join("main.luar");
    let items = items_at(&p, &main, src, 1, 0);
    let (line, text) = import_of(&items, "util").expect("util not offered");
    assert_eq!(line, 0);
    assert_eq!(text, "local util = require(\"./util\")\n");
}

#[test]
fn alias_wins_over_relative_path() {
    let src = "local x = 1\n";
    let (root, p) = temp_project(&[
        (
            "luari.json",
            "{\"aliases\": {\"Settings\": \"./Scenes/Settings\"}}",
        ),
        ("Scenes/Settings.luar", "return { volume = 5 }"),
        ("main.luar", src),
    ]);
    let main = root.join("main.luar");
    let items = items_at(&p, &main, src, 1, 0);
    let (_, text) = import_of(&items, "Settings").expect("Settings not offered");
    assert_eq!(text, "local Settings = require(\"@Settings\")\n");
}

#[test]
fn insert_goes_after_existing_requires() {
    let src = "local a = require(\"./a\")\nlocal b = require(\"./b\")\n\nprint(a, b)\n";
    let (root, p) = temp_project(&[
        ("main.luar", src),
        ("a.luar", "return 1"),
        ("b.luar", "return 2"),
        ("extra.luar", "return 3"),
    ]);
    let main = root.join("main.luar");
    let items = items_at(&p, &main, src, 2, 0);
    let (line, _) = import_of(&items, "extra").expect("extra not offered");
    assert_eq!(line, 2);
}

#[test]
fn required_modules_and_self_not_offered() {
    let src = "local a = require(\"./a\")\n\n";
    let (root, p) = temp_project(&[("main.luar", src), ("a.luar", "return 1")]);
    let main = root.join("main.luar");
    let items = items_at(&p, &main, src, 1, 0);
    assert!(import_of(&items, "a").is_none(), "already-required offered");
    assert!(import_of(&items, "main").is_none(), "self offered");
}

#[test]
fn climbing_path_from_nested_dir() {
    let src = "local x = 1\n";
    let (root, p) = temp_project(&[
        ("top.luar", "return 1"),
        ("a/b/deep.luar", src),
    ]);
    let deep = root.join("a").join("b").join("deep.luar");
    let items = items_at(&p, &deep, src, 1, 0);
    let (_, text) = import_of(&items, "top").expect("top not offered");
    assert_eq!(text, "local top = require(\".../top\")\n");
}

#[test]
fn init_module_named_after_folder() {
    let src = "local x = 1\n";
    let (root, p) = temp_project(&[
        ("ui/init.luar", "return { kind = \"ui\" }"),
        ("main.luar", src),
    ]);
    let main = root.join("main.luar");
    let items = items_at(&p, &main, src, 1, 0);
    let (_, text) = import_of(&items, "ui").expect("ui not offered");
    assert_eq!(text, "local ui = require(\"./ui\")\n");
}
