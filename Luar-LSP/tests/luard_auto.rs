use luar_lsp::completion::{self, FileView};
use luar_lsp::project::{extract_auto_imports, Project};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

static COUNTER: AtomicU32 = AtomicU32::new(0);

fn temp_project(files: &[(&str, &str)]) -> (PathBuf, Project) {
    let id = COUNTER.fetch_add(1, Ordering::SeqCst);
    let root = std::env::temp_dir().join(format!("luar_auto_{}_{}", std::process::id(), id));
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

const LUARD: &str = "const game = {}\n\nAuto {\n    key \"DS\" Variable \"game:GetService(\\\"DataStoreService\\\")\"\n    key \"Net\" Variable \"game:GetService(\\\"NetworkService\\\")\"\n}\n";

#[test]
fn extract_parses_entries_and_strips_block() {
    let (cleaned, entries) = extract_auto_imports(LUARD);
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].0, "DS");
    assert_eq!(entries[0].1, "game:GetService(\"DataStoreService\")");
    assert_eq!(entries[1].0, "Net");
    assert!(!cleaned.contains("Auto"));
    assert!(cleaned.contains("const game"));
    assert_eq!(cleaned.lines().count(), LUARD.lines().count());
}

#[test]
fn auto_entries_complete_with_topline_insert() {
    let main = "local util = require(\"util\")\n\nD\n";
    let (root, p) = temp_project(&[
        ("lib/game.luard", LUARD),
        ("util.luar", "return {}\n"),
        ("main.luar", main),
    ]);
    assert_eq!(p.auto_imports.len(), 2);
    assert!(p.luard_globals.iter().any(|(n, _)| n == "game"));
    let main_path = root.join("main.luar");
    let view = FileView::from_project(&p, &main_path).unwrap();
    let items = completion::complete(&view, main, 2, 1);
    let ds = items
        .iter()
        .find(|i| i.label == "DS")
        .expect("DS auto import offered");
    let auto = ds.auto_import.as_ref().expect("carries auto import edit");
    assert_eq!(auto.line0, 1);
    assert_eq!(
        auto.new_text,
        "local DS = game:GetService(\"DataStoreService\")\n"
    );
    assert!(items.iter().any(|i| i.label == "Net"));
}

#[test]
fn auto_entries_skip_names_already_bound() {
    let main = "local DS = 1\n\nD\n";
    let (root, p) = temp_project(&[("lib/game.luard", LUARD), ("main.luar", main)]);
    let main_path = root.join("main.luar");
    let view = FileView::from_project(&p, &main_path).unwrap();
    let items = completion::complete(&view, main, 2, 1);
    let ds = items.iter().find(|i| i.label == "DS").expect("DS binding");
    assert!(ds.auto_import.is_none());
}

#[test]
fn auto_imports_cluster_alphabetically_with_existing_ones() {
    let luard = "const game = {}\n\nAuto {\n    key \"DataStoreService\" Variable \"game:GetService(\\\"DataStoreService\\\")\"\n    key \"Players\" Variable \"game:GetService(\\\"Players\\\")\"\n    key \"Workspace\" Variable \"game:GetService(\\\"Workspace\\\")\"\n}\n";
    let main = "local Players = game:GetService(\"Players\")\n\nD\n";
    let (root, p) = temp_project(&[("lib/game.luard", luard), ("main.luar", main)]);
    let main_path = root.join("main.luar");
    let view = FileView::from_project(&p, &main_path).unwrap();
    let items = completion::complete(&view, main, 2, 1);
    let ds = items
        .iter()
        .find(|i| i.label == "DataStoreService")
        .expect("DataStoreService offered");
    assert_eq!(ds.auto_import.as_ref().unwrap().line0, 0);
    let ws = items
        .iter()
        .find(|i| i.label == "Workspace")
        .expect("Workspace offered");
    assert_eq!(ws.auto_import.as_ref().unwrap().line0, 1);
}

#[test]
fn malformed_auto_blocks_leave_source_untouched() {
    let src = "Auto { key \"X\" }\nconst game = {}\n";
    let (cleaned, entries) = extract_auto_imports(src);
    assert!(entries.is_empty());
    assert_eq!(cleaned, src);
}
