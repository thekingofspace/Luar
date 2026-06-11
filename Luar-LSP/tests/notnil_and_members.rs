use luar_lsp::completion::{self, FileView};
use luar_lsp::project::Project;
use luar_lsp::resolve::TypeEnv;
use luar_lsp::type_syntax::parse_type;
use luar_lsp::types::Type;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

static COUNTER: AtomicU32 = AtomicU32::new(0);

fn temp_project(files: &[(&str, &str)]) -> (PathBuf, Project) {
    let id = COUNTER.fetch_add(1, Ordering::SeqCst);
    let root = std::env::temp_dir().join(format!("luar_notnil_{}_{}", std::process::id(), id));
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
fn notnil_resolves_to_inner_type() {
    let e = TypeEnv::from_source("local x = 1").unwrap();
    assert_eq!(
        e.value_type(&parse_type("NotNil<number>").unwrap()),
        Type::Number
    );
    assert_eq!(
        e.value_type(&parse_type("notnil<number | nil>").unwrap()),
        Type::Number
    );
    assert_eq!(
        e.value_type(&parse_type("NotNil<number?>").unwrap()),
        Type::Number
    );
}

#[test]
fn notnil_assignment_to_nil_errors() {
    let (root, p) = temp_project(&[(
        "main.luar",
        "local hp: NotNil<number> = 100\nhp = nil\nprint(hp)\n",
    )]);
    let main = root.join("main.luar");
    let info = p.file(&main).unwrap();
    let diag = info
        .diagnostics
        .iter()
        .find(|d| d.message.contains("cannot set 'hp' to nil"))
        .expect("NotNil diagnostic missing");
    assert_eq!(diag.line, 2);
    assert_eq!(diag.severity, 1);
}

#[test]
fn notnil_requires_non_nil_initializer() {
    let (root, p) = temp_project(&[(
        "main.luar",
        "local hp: NotNil<number> = nil\nprint(hp)\n",
    )]);
    let main = root.join("main.luar");
    let info = p.file(&main).unwrap();
    assert!(
        info.diagnostics
            .iter()
            .any(|d| d.message.contains("must be initialized")),
        "{:?}",
        info.diagnostics
    );
}

#[test]
fn notnil_allows_normal_assignment() {
    let (root, p) = temp_project(&[(
        "main.luar",
        "local hp: NotNil<number> = 100\nhp = 50\nprint(hp)\n",
    )]);
    let main = root.join("main.luar");
    let info = p.file(&main).unwrap();
    assert!(
        !info.diagnostics.iter().any(|d| d.message.contains("NotNil")),
        "{:?}",
        info.diagnostics
    );
}

#[test]
fn notnil_param_rejects_nil_argument() {
    let (root, p) = temp_project(&[(
        "main.luar",
        "local function heal(target: NotNil<any>, amount: number)\n    print(target, amount)\nend\nheal(nil, 5)\n",
    )]);
    let main = root.join("main.luar");
    let info = p.file(&main).unwrap();
    let diag = info
        .diagnostics
        .iter()
        .find(|d| d.message.contains("cannot be nil"))
        .expect("NotNil param diagnostic missing");
    assert_eq!(diag.line, 4);
}

#[test]
fn notnil_can_be_silenced_with_directive() {
    let (root, p) = temp_project(&[(
        "main.luar",
        "local hp: NotNil<number> = 100\nhp = nil --#disable-line NotNil\nprint(hp)\n",
    )]);
    let main = root.join("main.luar");
    let info = p.file(&main).unwrap();
    assert!(
        !info.diagnostics.iter().any(|d| d.message.contains("NotNil")),
        "{:?}",
        info.diagnostics
    );
}

#[test]
fn class_member_position_offers_public_not_pub() {
    let src = "class Person {\n    pu\n}\n";
    let (root, p) = temp_project(&[("main.luar", src)]);
    let main = root.join("main.luar");
    let labels = labels_at(&p, &main, src, 1, 6);
    assert!(labels.contains(&"public".to_string()), "{labels:?}");
    assert!(!labels.contains(&"pub".to_string()), "{labels:?}");
    assert!(!labels.contains(&"local".to_string()), "{labels:?}");
}

#[test]
fn method_bodies_still_offer_normal_completions() {
    let src = "class Person {\n    function act()\n        \n    end\n}\n";
    let (root, p) = temp_project(&[("main.luar", src)]);
    let main = root.join("main.luar");
    let labels = labels_at(&p, &main, src, 2, 8);
    assert!(labels.contains(&"local".to_string()), "{labels:?}");
    assert!(labels.contains(&"print".to_string()), "{labels:?}");
}
