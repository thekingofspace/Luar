use luar_lsp::completion::{self, FileView};
use luar_lsp::project::Project;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

static COUNTER: AtomicU32 = AtomicU32::new(0);

const SRC: &str = "type test = {\n    Var:boolean,\n    Ve:string\n}\n\nconst test = \"Varg\"\n\n\nlocal function test(...)\n    local var = {...}\nend\n";

fn temp_project(files: &[(&str, &str)]) -> (PathBuf, Project) {
    let id = COUNTER.fetch_add(1, Ordering::SeqCst);
    let root = std::env::temp_dir().join(format!("luar_enter_{}_{}", std::process::id(), id));
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
fn test_offered_on_blank_line_after_const() {
    let (root, p) = temp_project(&[("main.luar", SRC)]);
    let main = root.join("main.luar");
    let names = labels_at(&p, &main, SRC, 7, 0);
    assert!(names.contains(&"test".to_string()), "{} items", names.len());
}

#[test]
fn test_offered_mid_word_after_enter() {
    let typed = SRC.replace("const test = \"Varg\"\n\n", "const test = \"Varg\"\ntes\n");
    let (root, mut p) = temp_project(&[("main.luar", SRC)]);
    let main = root.join("main.luar");
    p.update_file(&main, typed.clone());
    let names = labels_at(&p, &main, &typed, 6, 3);
    assert!(
        names.contains(&"test".to_string()),
        "{} items: {:?}",
        names.len(),
        names.iter().take(8).collect::<Vec<_>>()
    );
}

#[test]
fn test_offered_at_end_of_file() {
    let typed = format!("{SRC}\ntes");
    let (root, mut p) = temp_project(&[("main.luar", SRC)]);
    let main = root.join("main.luar");
    p.update_file(&main, typed.clone());
    let line0 = typed.lines().count() - 1;
    let names = labels_at(&p, &main, &typed, line0, 3);
    assert!(names.contains(&"test".to_string()), "{} items", names.len());
}
