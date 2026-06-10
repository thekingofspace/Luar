use luar_lsp::completion::{self, FileView};
use luar_lsp::project::Project;
use luar_lsp::types::Type;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

static COUNTER: AtomicU32 = AtomicU32::new(0);

fn temp_project(files: &[(&str, &str)]) -> (PathBuf, Project) {
    let id = COUNTER.fetch_add(1, Ordering::SeqCst);
    let root = std::env::temp_dir().join(format!("luar_super_{}_{}", std::process::id(), id));
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

const BASE: &str = "class Animal {\n    public name: string = \"a\"\n    function speak(): string\n        return \"...\"\n    end\n    get label(): string\n        return self.name\n    end\n}\nclass Dog extends Animal {\n    override function speak(): string\n        return \"woof\"\n    end\n    function test()\n";

#[test]
fn super_colon_lists_parent_methods() {
    let src = format!("{BASE}        super:\n    end\n}}\n");
    let (root, p) = temp_project(&[("main.luar", &src)]);
    let main = root.join("main.luar");
    let line0 = 14;
    let col = "        super:".chars().count();
    let labels = labels_at(&p, &main, &src, line0, col);
    assert!(labels.contains(&"speak".to_string()), "{labels:?}");
}

#[test]
fn super_dot_lists_parent_fields_and_getters() {
    let src = format!("{BASE}        local n = super.\n    end\n}}\n");
    let (root, p) = temp_project(&[("main.luar", &src)]);
    let main = root.join("main.luar");
    let line0 = 14;
    let col = "        local n = super.".chars().count();
    let labels = labels_at(&p, &main, &src, line0, col);
    assert!(labels.contains(&"name".to_string()), "{labels:?}");
    assert!(labels.contains(&"label".to_string()), "{labels:?}");
}

#[test]
fn super_method_call_type_resolves() {
    let src = format!("{BASE}        local s = super:speak()\n    end\n}}\n");
    let (root, p) = temp_project(&[("main.luar", &src)]);
    let main = root.join("main.luar");
    let info = p.file(&main).unwrap();
    assert_eq!(info.analysis.type_of("s"), Some(&Type::String));
}

#[test]
fn self_in_subclass_sees_inherited_members() {
    let src = format!("{BASE}        self.\n    end\n}}\n");
    let (root, p) = temp_project(&[("main.luar", &src)]);
    let main = root.join("main.luar");
    let line0 = 14;
    let col = "        self.".chars().count();
    let labels = labels_at(&p, &main, &src, line0, col);
    assert!(labels.contains(&"name".to_string()), "{labels:?}");
}
