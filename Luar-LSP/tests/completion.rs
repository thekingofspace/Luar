use luar_lsp::completion::{self, FileView};
use luar_lsp::project::Project;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

static COUNTER: AtomicU32 = AtomicU32::new(0);

fn temp_project(files: &[(&str, &str)]) -> (PathBuf, Project) {
    let id = COUNTER.fetch_add(1, Ordering::SeqCst);
    let root = std::env::temp_dir().join(format!(
        "luar_lsp_comp_{}_{}",
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

fn labels(items: &[completion::Item]) -> Vec<String> {
    items.iter().map(|i| i.label.clone()).collect()
}

fn complete_at(
    project: &Project,
    path: &PathBuf,
    line0: usize,
    col0: usize,
) -> Vec<completion::Item> {
    let view = FileView::from_project(project, path).expect("file in project");
    let text = project.file(path).unwrap().source.clone();
    completion::complete(&view, &text, line0, col0)
}

#[test]
fn members_after_dot_on_instance() {
    let src = "class Point {\n  public x: number = 0\n  get size() return 1 end\n  function move() return true end\n}\nlocal p = Point(1)\nlocal q = p.";
    let (root, p) = temp_project(&[("main.luar", src)]);
    let main = root.join("main.luar");
    let items = complete_at(&p, &main, 6, 12);
    let names = labels(&items);
    assert!(names.contains(&"x".to_string()));
    assert!(names.contains(&"size".to_string()));
    assert!(!names.contains(&"move".to_string()));
}

#[test]
fn methods_after_colon_on_instance() {
    let src = "class Point {\n  public x: number = 0\n  function move() return true end\n}\nlocal p = Point(1)\np:";
    let (root, p) = temp_project(&[("main.luar", src)]);
    let main = root.join("main.luar");
    let items = complete_at(&p, &main, 5, 2);
    let names = labels(&items);
    assert!(names.contains(&"move".to_string()));
    assert!(!names.contains(&"x".to_string()));
    let move_item = items.iter().find(|i| i.label == "move").unwrap();
    assert!(move_item.is_snippet);
}

#[test]
fn enum_variants_after_dot() {
    let src = "enum Color { Red Green Blue }\nlocal c = Color.";
    let (root, p) = temp_project(&[("main.luar", src)]);
    let main = root.join("main.luar");
    let items = complete_at(&p, &main, 1, 16);
    assert_eq!(
        labels(&items),
        vec!["Red".to_string(), "Green".to_string(), "Blue".to_string()]
    );
}

#[test]
fn string_literal_union_autofill_on_assignment_and_comparison() {
    let src = "local mode: \"on\" | \"off\" = \"on\"\nmode = \"\nif mode == \"";
    let (root, p) = temp_project(&[("main.luar", src)]);
    let main = root.join("main.luar");
    let set_items = complete_at(&p, &main, 1, 8);
    assert_eq!(
        labels(&set_items),
        vec!["off".to_string(), "on".to_string()]
    );
    let check_items = complete_at(&p, &main, 2, 12);
    assert_eq!(
        labels(&check_items),
        vec!["off".to_string(), "on".to_string()]
    );
}

#[test]
fn literal_union_through_alias() {
    let src = "type Mode = \"fast\" | \"slow\"\nlocal m: Mode = \"fast\"\nm = \"";
    let (root, p) = temp_project(&[("main.luar", src)]);
    let main = root.join("main.luar");
    let items = complete_at(&p, &main, 2, 5);
    assert_eq!(
        labels(&items),
        vec!["fast".to_string(), "slow".to_string()]
    );
}

#[test]
fn type_position_offers_types_not_values() {
    let src = "class Dog {\n}\nenum Color { Red }\ntype Alias = number\nlocal x: ";
    let (root, p) = temp_project(&[("main.luar", src)]);
    let main = root.join("main.luar");
    let items = complete_at(&p, &main, 4, 9);
    let names = labels(&items);
    assert!(names.contains(&"number".to_string()));
    assert!(names.contains(&"Dog".to_string()));
    assert!(names.contains(&"Color".to_string()));
    assert!(names.contains(&"Alias".to_string()));
    assert!(!names.contains(&"local".to_string()));
}

#[test]
fn type_alias_body_offers_types() {
    let src = "class Dog {\n}\ntype Test = {\n    Name:\n}";
    let (root, p) = temp_project(&[("main.luar", src)]);
    let main = root.join("main.luar");
    let items = complete_at(&p, &main, 3, 9);
    let names = labels(&items);
    assert!(names.contains(&"string".to_string()), "{names:?}");
    assert!(names.contains(&"boolean".to_string()));
    assert!(names.contains(&"Dog".to_string()));
    let nested = "type Outer = {\n    inner: {\n        deep:\n    }\n}";
    let (root2, p2) = temp_project(&[("main.luar", nested)]);
    let main2 = root2.join("main.luar");
    let items = complete_at(&p2, &main2, 2, 13);
    assert!(labels(&items).contains(&"number".to_string()));
}

#[test]
fn cast_position_offers_types() {
    let src = "local v = 1\nlocal x = v :: ";
    let (root, p) = temp_project(&[("main.luar", src)]);
    let main = root.join("main.luar");
    let items = complete_at(&p, &main, 1, 15);
    let names = labels(&items);
    assert!(names.contains(&"number".to_string()));
    assert!(names.contains(&"string".to_string()));
}

#[test]
fn module_types_after_dot_in_type_position() {
    let (root, p) = temp_project(&[
        (
            "shapes.luar",
            "export type Shape = { width: number }\nreturn {}",
        ),
        (
            "main.luar",
            "local shapes = require(\"./shapes\")\nlocal s: shapes.",
        ),
    ]);
    let main = root.join("main.luar");
    let items = complete_at(&p, &main, 1, 16);
    assert!(labels(&items).contains(&"Shape".to_string()));
}

#[test]
fn require_path_completion() {
    let (root, p) = temp_project(&[
        ("Scenes/Title.luar", "return 1"),
        ("Scenes/Base.luar", "return 1"),
        ("main.luar", "local t = require(\"./Scenes/\")"),
    ]);
    let main = root.join("main.luar");
    let items = complete_at(&p, &main, 0, 28);
    let names = labels(&items);
    assert!(names.contains(&"Base".to_string()));
    assert!(names.contains(&"Title".to_string()));
}

#[test]
fn value_position_offers_bindings_keywords_globals() {
    let src = "local count = 1\nlocal x = ";
    let (root, p) = temp_project(&[("main.luar", src)]);
    let main = root.join("main.luar");
    let items = complete_at(&p, &main, 1, 10);
    let names = labels(&items);
    assert!(names.contains(&"count".to_string()));
    assert!(names.contains(&"print".to_string()));
    assert!(names.contains(&"switch".to_string()));
    assert!(names.contains(&"math".to_string()));
}

#[test]
fn table_module_fields_after_dot() {
    let (root, p) = temp_project(&[
        (
            "shapes.luar",
            "local M = {}\nfunction M.area(r)\n  return r\nend\nM.version = 2\nreturn M",
        ),
        (
            "main.luar",
            "local shapes = require(\"./shapes\")\nlocal a = shapes.",
        ),
    ]);
    let main = root.join("main.luar");
    let items = complete_at(&p, &main, 1, 17);
    let names = labels(&items);
    assert!(names.contains(&"area".to_string()));
    assert!(names.contains(&"version".to_string()));
}

#[test]
fn luard_globals_in_value_completion() {
    let (root, p) = temp_project(&[
        ("lib/std.luard", "function spawnEntity(kind: string): number\nend"),
        ("main.luar", "local x = "),
    ]);
    let main = root.join("main.luar");
    let items = complete_at(&p, &main, 0, 10);
    let spawn = items.iter().find(|i| i.label == "spawnEntity");
    assert!(spawn.is_some());
}

#[test]
fn no_space_annotation_offers_types() {
    let src = "local test:";
    let (root, p) = temp_project(&[("main.luar", src)]);
    let main = root.join("main.luar");
    let items = complete_at(&p, &main, 0, 11);
    let names = labels(&items);
    assert!(names.contains(&"boolean".to_string()), "got: {names:?}");
    assert!(names.contains(&"number".to_string()));
    let src2 = "local test:bo";
    let (root2, p2) = temp_project(&[("main.luar", src2)]);
    let main2 = root2.join("main.luar");
    let items2 = complete_at(&p2, &main2, 0, 13);
    assert!(labels(&items2).contains(&"boolean".to_string()));
}

#[test]
fn class_body_offers_member_keywords() {
    let src = "class Point {\n  \n}";
    let (root, p) = temp_project(&[("main.luar", src)]);
    let main = root.join("main.luar");
    let items = complete_at(&p, &main, 1, 2);
    let names = labels(&items);
    for kw in [
        "public", "private", "protected", "static", "get", "set", "operator", "constructor",
        "function",
    ] {
        assert!(names.contains(&kw.to_string()), "missing {kw}: {names:?}");
    }
    let items = complete_at(&p, &main, 1, 2);
    assert!(labels(&items).contains(&"local".to_string()));
}

#[test]
fn modifier_prefix_still_offers_member_keywords() {
    let src = "class Point {\n  public \n}";
    let (root, p) = temp_project(&[("main.luar", src)]);
    let main = root.join("main.luar");
    let items = complete_at(&p, &main, 1, 9);
    let names = labels(&items);
    assert!(names.contains(&"function".to_string()));
    assert!(names.contains(&"static".to_string()));
    assert!(!names.contains(&"public".to_string()) || names.iter().filter(|n| *n == "public").count() <= 1);
}

#[test]
fn member_keywords_not_offered_outside_class() {
    let src = "local x = 1\n";
    let (root, p) = temp_project(&[("main.luar", src)]);
    let main = root.join("main.luar");
    let items = complete_at(&p, &main, 1, 0);
    let names = labels(&items);
    assert!(!names.contains(&"private".to_string()));
    assert!(!names.contains(&"operator".to_string()));
}

#[test]
fn switch_completion_inserts_parens() {
    let src = "local x = 1\n";
    let (root, p) = temp_project(&[("main.luar", src)]);
    let main = root.join("main.luar");
    let items = complete_at(&p, &main, 1, 0);
    let sw = items
        .iter()
        .find(|i| i.label == "switch")
        .expect("switch offered");
    assert_eq!(sw.insert_text.as_deref(), Some("switch($1)"));
    assert!(sw.is_snippet);
}

#[test]
fn hover_helpers() {
    let text = "-- Adds two numbers.\n-- @return number\nlocal function add(a, b)\n  return a + b\nend";
    let docs = luar_lsp::lsp::doc_comment_above(text, Some(3));
    let docs = docs.expect("docs found");
    assert!(docs.contains("Adds two numbers."));
    assert!(docs.contains("@return number"));
}

#[test]
fn uri_path_round_trip() {
    let path = PathBuf::from("c:\\Users\\test\\proj\\main.luar");
    let uri = luar_lsp::lsp::path_to_uri(&path);
    assert_eq!(uri, "file:///c%3A/Users/test/proj/main.luar");
    let back = luar_lsp::lsp::uri_to_path(&uri).unwrap();
    assert_eq!(back, path);
}

#[test]
fn at_alias_completion_inserts_without_duplicate_at() {
    let (root, p) = temp_project(&[
        ("luari.json", "{\"aliases\": {\"Common\": \"./Utils/Common\"}}"),
        ("Utils/Common.luar", "return 1"),
        ("main.luar", "local x = require(\"@"),
    ]);
    let main = root.join("main.luar");
    let view = FileView::from_project(&p, &main).unwrap();
    let src = p.file(&main).unwrap().source.clone();
    let items = completion::complete(&view, &src, 0, 20);
    let common = items
        .iter()
        .find(|i| i.label == "@Common")
        .expect("alias offered");
    assert_eq!(common.insert_text.as_deref(), Some("Common"));
    let selfless = items.iter().find(|i| i.label == "@self");
    assert!(selfless.is_none(), "@self outside init");
}
