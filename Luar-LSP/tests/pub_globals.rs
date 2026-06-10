use luar_lsp::completion::{self, FileView};
use luar_lsp::project::Project;
use luar_lsp::types::Type;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

static COUNTER: AtomicU32 = AtomicU32::new(0);

fn temp_project(files: &[(&str, &str)]) -> (PathBuf, Project) {
    let id = COUNTER.fetch_add(1, Ordering::SeqCst);
    let root = std::env::temp_dir().join(format!("luar_pub_{}_{}", std::process::id(), id));
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

const LIB: &str = "pub class Monster {\n    public Health:number = 100\n    public function Attack(target)\n        return true\n    end\n}\npub enum Phase { Idle Rage }\npub const VERSION = \"1.0\"\npub local counter = 0\nlocal private_thing = 5\n";

#[test]
fn pub_declarations_visible_in_other_files() {
    let (root, p) = temp_project(&[
        ("lib.luar", LIB),
        (
            "main.luar",
            "local m = Monster()\nlocal hp = m.Health\nlocal ph = Phase.Rage\nlocal v = VERSION\nlocal c = counter\nlocal hidden = private_thing\n",
        ),
    ]);
    let main = root.join("main.luar");
    let info = p.file(&main).unwrap();
    assert_eq!(
        info.analysis.type_of("m"),
        Some(&Type::Instance("Monster".to_string()))
    );
    assert_eq!(info.analysis.type_of("hp"), Some(&Type::Number));
    assert_eq!(
        info.analysis.type_of("ph"),
        Some(&Type::EnumValue("Phase".to_string()))
    );
    assert_eq!(info.analysis.type_of("v"), Some(&Type::String));
    assert_eq!(info.analysis.type_of("c"), Some(&Type::Number));
    assert_eq!(
        info.analysis.type_of("hidden"),
        Some(&Type::Unknown),
        "non-pub local leaked across files"
    );
}

#[test]
fn pub_names_in_completion_and_members() {
    let (root, p) = temp_project(&[
        ("lib.luar", LIB),
        ("main.luar", "local x = 1\n\n"),
    ]);
    let main = root.join("main.luar");
    let view = FileView::from_project(&p, &main).unwrap();
    let src = p.file(&main).unwrap().source.clone();
    let labels: Vec<String> = completion::complete(&view, &src, 1, 0)
        .into_iter()
        .map(|i| i.label)
        .collect();
    assert!(labels.contains(&"Monster".to_string()), "pub class missing");
    assert!(labels.contains(&"Phase".to_string()), "pub enum missing");
    assert!(labels.contains(&"VERSION".to_string()), "pub const missing");
    assert!(!labels.contains(&"private_thing".to_string()), "private leaked");

    let member_src = "local m = Monster()\nm:";
    let mut p2 = p;
    p2.update_file(&main, member_src.to_string());
    let view2 = FileView::from_project(&p2, &main).unwrap();
    let labels: Vec<String> = completion::complete(&view2, member_src, 1, 2)
        .into_iter()
        .map(|i| i.label)
        .collect();
    assert!(labels.contains(&"Attack".to_string()), "{labels:?}");
}

#[test]
fn pub_annotation_resolves_cross_file() {
    let (root, p) = temp_project(&[
        ("lib.luar", LIB),
        ("main.luar", "local pet: Monster = spawn()\nlocal h = pet.Health\n"),
    ]);
    let main = root.join("main.luar");
    let info = p.file(&main).unwrap();
    assert_eq!(
        info.analysis.type_of("pet"),
        Some(&Type::Instance("Monster".to_string()))
    );
    assert_eq!(info.analysis.type_of("h"), Some(&Type::Number));
}
