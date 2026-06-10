use luar_lsp::completion::{self, FileView};
use luar_lsp::project::Project;
use luar_lsp::types::Type;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

static COUNTER: AtomicU32 = AtomicU32::new(0);

fn temp_project(files: &[(&str, &str)]) -> (PathBuf, Project) {
    let id = COUNTER.fetch_add(1, Ordering::SeqCst);
    let root = std::env::temp_dir().join(format!("luar_gk_{}_{}", std::process::id(), id));
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

fn items_at(
    p: &Project,
    path: &PathBuf,
    src: &str,
    line0: usize,
    col0: usize,
) -> Vec<completion::Item> {
    let view = FileView::from_project(p, path).expect("file");
    completion::complete(&view, src, line0, col0)
}

const VEG: &str = "type veg = {\n    Var:boolean,\n    Ve:string\n}\n\nlocal function test<i>(input:keyof<veg>):ValueOf<veg, i>\n    local var = {...}\nend\n\nlocal re = test(\"Var\")\nlocal rs = test(\"Ve\")\n";

#[test]
fn generic_valueof_resolves_from_call_arg() {
    let (root, p) = temp_project(&[("main.luar", VEG)]);
    let main = root.join("main.luar");
    let info = p.file(&main).unwrap();
    assert_eq!(info.analysis.type_of("re"), Some(&Type::Boolean), "re should be boolean");
    assert_eq!(info.analysis.type_of("rs"), Some(&Type::String), "rs should be string");
}

#[test]
fn keyof_param_displays_literal_union() {
    let (root, p) = temp_project(&[("main.luar", VEG)]);
    let main = root.join("main.luar");
    let info = p.file(&main).unwrap();
    match info.analysis.type_of("test") {
        Some(Type::Function(Some(sig))) => {
            let shown = sig.params[0].ty.to_string();
            assert!(
                shown.contains("\"Var\"") && shown.contains("\"Ve\""),
                "param displayed as {shown}"
            );
        }
        other => panic!("expected function, got {other:?}"),
    }
}

#[test]
fn bracket_keys_with_spaces_offered() {
    let src = "local t = {}\nt[\"easter.Test tes\"] = 1\nt.plain = 2\nlocal v = t[\"";
    let (root, p) = temp_project(&[("main.luar", src)]);
    let main = root.join("main.luar");
    let col = src.lines().last().unwrap().chars().count();
    let items = items_at(&p, &main, src, 3, col);
    let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
    assert!(labels.contains(&"easter.Test tes"), "{labels:?}");
    assert!(labels.contains(&"plain"));
}

#[test]
fn bracket_without_quote_inserts_quoted_key() {
    let src = "local t = {}\nt[\"easter.Test tes\"] = 1\nlocal v = t[";
    let (root, p) = temp_project(&[("main.luar", src)]);
    let main = root.join("main.luar");
    let col = src.lines().last().unwrap().chars().count();
    let items = items_at(&p, &main, src, 2, col);
    let item = items
        .iter()
        .find(|i| i.label == "easter.Test tes")
        .expect("bracket key offered");
    assert_eq!(item.insert_text.as_deref(), Some("\"easter.Test tes\"]"));
}

#[test]
fn weird_keys_hidden_from_dot_completion() {
    let src = "local t = {}\nt[\"easter.Test tes\"] = 1\nt.plain = 2\nlocal v = t.";
    let (root, p) = temp_project(&[("main.luar", src)]);
    let main = root.join("main.luar");
    let col = src.lines().last().unwrap().chars().count();
    let items = items_at(&p, &main, src, 3, col);
    let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
    assert!(labels.contains(&"plain"));
    assert!(!labels.contains(&"easter.Test tes"), "{labels:?}");
}

#[test]
fn call_argument_union_literals_offered() {
    let src = "local function go(mode: \"this\" | \"that\", extra: \"x\" | \"y\")
end
go(\"";
    let (root, p) = temp_project(&[("main.luar", src)]);
    let main = root.join("main.luar");
    let col = src.lines().last().unwrap().chars().count();
    let items = items_at(&p, &main, src, 2, col);
    let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
    assert_eq!(labels, vec!["that", "this"], "{labels:?}");

    let src2 = "local function go(mode: \"this\" | \"that\", extra: \"x\" | \"y\")
end
go(\"this\", \"";
    let (root2, p2) = temp_project(&[("main.luar", src2)]);
    let main2 = root2.join("main.luar");
    let col = src2.lines().last().unwrap().chars().count();
    let items = items_at(&p2, &main2, src2, 2, col);
    let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
    assert_eq!(labels, vec!["x", "y"], "{labels:?}");
}

#[test]
fn call_argument_keyof_literals_offered() {
    let src = format!("{VEG}local q = test(\"");
    let (root, p) = temp_project(&[("main.luar", &src)]);
    let main = root.join("main.luar");
    let line0 = src.lines().count() - 1;
    let col = src.lines().last().unwrap().chars().count();
    let items = items_at(&p, &main, &src, line0, col);
    let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
    assert_eq!(labels, vec!["Var", "Ve"], "{labels:?}");
}
