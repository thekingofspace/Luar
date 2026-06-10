use luar_lsp::completion::{self, FileView};
use luar_lsp::project::Project;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

static COUNTER: AtomicU32 = AtomicU32::new(0);

fn temp_project(files: &[(&str, &str)]) -> (PathBuf, Project) {
    let id = COUNTER.fetch_add(1, Ordering::SeqCst);
    let root = std::env::temp_dir().join(format!("luar_ferrite_{}_{}", std::process::id(), id));
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

fn items_at(p: &Project, main: &PathBuf, src: &str, line0: usize, col: usize) -> Vec<String> {
    let view = FileView::from_project(p, main).unwrap();
    completion::complete(&view, src, line0, col)
        .into_iter()
        .map(|i| i.label)
        .collect()
}

#[test]
fn unused_variable_diagnostic_appears() {
    let (root, p) = temp_project(&[("main.luar", "local unusedThing = 5\n")]);
    let main = root.join("main.luar");
    let info = p.file(&main).unwrap();
    let diag = info
        .diagnostics
        .iter()
        .find(|d| d.message.contains("[UnusedVariable]"))
        .expect("ferrite lint missing");
    assert_eq!(diag.severity, 2);
    assert_eq!(diag.line, 1);
}

#[test]
fn global_disable_silences_lint() {
    let (root, p) = temp_project(&[(
        "main.luar",
        "--#disable UnusedVariable\nlocal unusedThing = 5\n",
    )]);
    let main = root.join("main.luar");
    let info = p.file(&main).unwrap();
    assert!(
        !info
            .diagnostics
            .iter()
            .any(|d| d.message.contains("[UnusedVariable]")),
        "{:?}",
        info.diagnostics
    );
}

#[test]
fn disable_all_silences_everything() {
    let (root, p) = temp_project(&[(
        "main.luar",
        "--#disable all\nlocal unusedThing = 5\nif true then print(1) end\n",
    )]);
    let main = root.join("main.luar");
    let info = p.file(&main).unwrap();
    assert!(
        !info.diagnostics.iter().any(|d| d.message.contains('[')),
        "{:?}",
        info.diagnostics
    );
}

#[test]
fn disable_line_silences_immutable_reassign() {
    let (root, p) = temp_project(&[(
        "main.luar",
        "const x = 1\nx = 2 --#disable-line MutateImmutable\nprint(x)\n",
    )]);
    let main = root.join("main.luar");
    let info = p.file(&main).unwrap();
    assert!(
        !info
            .diagnostics
            .iter()
            .any(|d| d.message.contains("cannot reassign immutable")),
        "{:?}",
        info.diagnostics
    );
}

#[test]
fn disable_next_line_silences_immutable_reassign() {
    let (root, p) = temp_project(&[(
        "main.luar",
        "const x = 1\n--#disable-next-line MutateImmutable\nx = 2\nprint(x)\n",
    )]);
    let main = root.join("main.luar");
    let info = p.file(&main).unwrap();
    assert!(
        !info
            .diagnostics
            .iter()
            .any(|d| d.message.contains("cannot reassign immutable")),
        "{:?}",
        info.diagnostics
    );
}

#[test]
fn immutable_reassign_still_fires_without_directive() {
    let (root, p) = temp_project(&[("main.luar", "const x = 1\nx = 2\nprint(x)\n")]);
    let main = root.join("main.luar");
    let info = p.file(&main).unwrap();
    assert!(
        info.diagnostics
            .iter()
            .any(|d| d.message.contains("cannot reassign immutable")),
        "{:?}",
        info.diagnostics
    );
}

#[test]
fn duplicate_enum_variant_can_be_disabled() {
    let (root, p) = temp_project(&[(
        "main.luar",
        "--#disable DuplicateEnumVariant\nenum Grag {\n    test,\n    test\n}\n",
    )]);
    let main = root.join("main.luar");
    let info = p.file(&main).unwrap();
    assert!(
        !info
            .diagnostics
            .iter()
            .any(|d| d.message.contains("duplicate variant")),
        "{:?}",
        info.diagnostics
    );
}

#[test]
fn luard_files_are_not_linted() {
    let (root, p) = temp_project(&[
        ("lib/engine.luard", "local unusedThing = 5\n"),
        ("main.luar", "print(1)\n"),
    ]);
    let luard = root.join("lib").join("engine.luard");
    if let Some(info) = p.file(&luard) {
        assert!(
            !info
                .diagnostics
                .iter()
                .any(|d| d.message.contains("[UnusedVariable]")),
            "{:?}",
            info.diagnostics
        );
    }
}

#[test]
fn directive_keyword_completion() {
    let src = "--#dis";
    let (root, p) = temp_project(&[("main.luar", src)]);
    let main = root.join("main.luar");
    let labels = items_at(&p, &main, src, 0, src.chars().count());
    assert!(labels.contains(&"disable".to_string()), "{labels:?}");
    assert!(labels.contains(&"disable-line".to_string()), "{labels:?}");
    assert!(labels.contains(&"disable-next-line".to_string()), "{labels:?}");
}

#[test]
fn directive_check_name_completion() {
    let src = "--#disable Unu";
    let (root, p) = temp_project(&[("main.luar", src)]);
    let main = root.join("main.luar");
    let labels = items_at(&p, &main, src, 0, src.chars().count());
    assert!(labels.contains(&"UnusedVariable".to_string()), "{labels:?}");
    assert!(labels.contains(&"UnusedParameter".to_string()), "{labels:?}");
    assert!(!labels.contains(&"MutateImmutable".to_string()), "{labels:?}");
}

#[test]
fn directive_check_names_after_comma() {
    let src = "local x = 1 --#disable-line MutateImmutable, Sha";
    let (root, p) = temp_project(&[("main.luar", src)]);
    let main = root.join("main.luar");
    let labels = items_at(&p, &main, src, 0, src.chars().count());
    assert!(labels.contains(&"ShadowedVariable".to_string()), "{labels:?}");
    assert!(!labels.contains(&"UnusedVariable".to_string()), "{labels:?}");
}

#[test]
fn empty_partial_lists_all_plus_editor_checks() {
    let src = "--#disable-next-line ";
    let (root, p) = temp_project(&[("main.luar", src)]);
    let main = root.join("main.luar");
    let labels = items_at(&p, &main, src, 0, src.chars().count());
    assert!(labels.contains(&"all".to_string()), "{labels:?}");
    assert!(labels.contains(&"RequireCycle".to_string()), "{labels:?}");
    assert!(labels.contains(&"DuplicateEnumVariant".to_string()), "{labels:?}");
}

#[test]
fn plain_comments_do_not_complete() {
    let src = "-- just a note abo";
    let (root, p) = temp_project(&[("main.luar", src)]);
    let main = root.join("main.luar");
    let labels = items_at(&p, &main, src, 0, src.chars().count());
    assert!(labels.is_empty(), "{labels:?}");
}
