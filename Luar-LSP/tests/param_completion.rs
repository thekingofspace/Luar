use luar_lsp::completion::{self, FileView};
use luar_lsp::project::Project;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

static COUNTER: AtomicU32 = AtomicU32::new(0);

fn temp_project(files: &[(&str, &str)]) -> (PathBuf, Project) {
    let id = COUNTER.fetch_add(1, Ordering::SeqCst);
    let root = std::env::temp_dir().join(format!("luar_paramc_{}_{}", std::process::id(), id));
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
fn function_params_offered_in_body() {
    let src = "local function test(input, count)\n    \nend\n";
    let (root, p) = temp_project(&[("main.luar", src)]);
    let main = root.join("main.luar");
    let labels = labels_at(&p, &main, src, 1, 4);
    assert!(labels.contains(&"input".to_string()), "{labels:?}");
    assert!(labels.contains(&"count".to_string()), "{labels:?}");
}

#[test]
fn method_params_offered_in_body() {
    let src = "class Person {\n    function rename(newName: string)\n        \n    end\n}\n";
    let (root, p) = temp_project(&[("main.luar", src)]);
    let main = root.join("main.luar");
    let labels = labels_at(&p, &main, src, 2, 8);
    assert!(labels.contains(&"newName".to_string()), "{labels:?}");
}

#[test]
fn anonymous_function_params_offered() {
    let src = "local cb = function(payload)\n    \nend\n";
    let (root, p) = temp_project(&[("main.luar", src)]);
    let main = root.join("main.luar");
    let labels = labels_at(&p, &main, src, 1, 4);
    assert!(labels.contains(&"payload".to_string()), "{labels:?}");
}

#[test]
fn params_not_offered_outside_function() {
    let src = "local function test(secret)\nend\nlocal after = 1\n";
    let (root, p) = temp_project(&[("main.luar", src)]);
    let main = root.join("main.luar");
    let labels = labels_at(&p, &main, src, 2, 14);
    assert!(!labels.contains(&"secret".to_string()), "{labels:?}");
}

#[test]
fn nested_function_sees_outer_params() {
    let src = "local function outer(a)\n    local function inner(b)\n        \n    end\nend\n";
    let (root, p) = temp_project(&[("main.luar", src)]);
    let main = root.join("main.luar");
    let labels = labels_at(&p, &main, src, 2, 8);
    assert!(labels.contains(&"a".to_string()), "{labels:?}");
    assert!(labels.contains(&"b".to_string()), "{labels:?}");
}

#[test]
fn partial_word_filters_params() {
    let src = "local function go(direction)\n    dir\nend\n";
    let (root, p) = temp_project(&[("main.luar", src)]);
    let main = root.join("main.luar");
    let labels = labels_at(&p, &main, src, 1, 7);
    assert!(labels.contains(&"direction".to_string()), "{labels:?}");
}
