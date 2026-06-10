use luar_lsp::project::Project;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

static COUNTER: AtomicU32 = AtomicU32::new(0);

fn temp_project(files: &[(&str, &str)]) -> (PathBuf, Project) {
    let id = COUNTER.fetch_add(1, Ordering::SeqCst);
    let root = std::env::temp_dir().join(format!("luar_cyc_{}_{}", std::process::id(), id));
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

#[test]
fn require_completion_excludes_current_module() {
    let (root, p) = temp_project(&[
        ("main.luar", "local x = require(\"./\")"),
        ("other.luar", "return 1"),
        ("third.luar", "return 2"),
    ]);
    let main = root.join("main.luar");
    let items = p.complete_require(&main, "./");
    assert!(items.contains(&"other".to_string()));
    assert!(items.contains(&"third".to_string()));
    assert!(
        !items.contains(&"main".to_string()),
        "current module listed: {items:?}"
    );
}

#[test]
fn directory_require_type_excludes_requirer() {
    let (root, p) = temp_project(&[
        ("ui/init.luar", "local parts = require(\"@self\")\nreturn parts"),
        ("ui/button.luar", "local parts = require(\"./\")\nreturn 1"),
        ("ui/slider.luar", "return 2"),
    ]);
    let button = root.join("ui").join("button.luar");
    let info = p.file(&button).unwrap();
    match info.analysis.type_of("parts") {
        Some(luar_lsp::Type::Table(tt)) => {
            assert!(
                !tt.fields.iter().any(|(n, _)| n == "button"),
                "requirer listed in its own ./ table: {:?}",
                tt.fields
            );
            assert!(tt.fields.iter().any(|(n, _)| n == "slider"));
        }
        other => panic!("expected table, got {other:?}"),
    }
}

#[test]
fn two_file_cycle_warned() {
    let (root, p) = temp_project(&[
        ("a.luar", "local b = require(\"./b\")\nreturn 1"),
        ("b.luar", "local a = require(\"./a\")\nreturn 2"),
    ]);
    let a = root.join("a.luar");
    let info = p.file(&a).unwrap();
    let diag = info
        .diagnostics
        .iter()
        .find(|d| d.message.contains("require cycle"))
        .unwrap_or_else(|| panic!("no cycle warning: {:?}", info.diagnostics));
    assert_eq!(diag.severity, 2);
    assert_eq!(diag.line, 1);
    assert!(diag.message.contains("a -> b -> a"), "{}", diag.message);
}

#[test]
fn three_file_cycle_warned() {
    let (root, p) = temp_project(&[
        ("a.luar", "local b = require(\"./b\")\nreturn 1"),
        ("b.luar", "local c = require(\"./c\")\nreturn 2"),
        ("c.luar", "local a = require(\"./a\")\nreturn 3"),
    ]);
    let a = root.join("a.luar");
    let info = p.file(&a).unwrap();
    assert!(
        info.diagnostics
            .iter()
            .any(|d| d.message.contains("require cycle")),
        "{:?}",
        info.diagnostics
    );
}

#[test]
fn self_require_warned() {
    let (root, p) = temp_project(&[("loopy.luar", "local me = require(\"./loopy\")\nreturn 1")]);
    let f = root.join("loopy.luar");
    let info = p.file(&f).unwrap();
    assert!(
        info.diagnostics
            .iter()
            .any(|d| d.message.contains("require cycle")),
        "{:?}",
        info.diagnostics
    );
}

#[test]
fn no_false_cycle_for_diamond() {
    let (root, p) = temp_project(&[
        ("a.luar", "local b = require(\"./b\")\nlocal c = require(\"./c\")\nreturn 1"),
        ("b.luar", "local d = require(\"./d\")\nreturn 2"),
        ("c.luar", "local d = require(\"./d\")\nreturn 3"),
        ("d.luar", "return 4"),
    ]);
    let a = root.join("a.luar");
    let info = p.file(&a).unwrap();
    assert!(
        !info
            .diagnostics
            .iter()
            .any(|d| d.message.contains("require cycle")),
        "false positive: {:?}",
        info.diagnostics
    );
}

#[test]
fn user_repro_messy_file_cycle_warned() {
    let (root, p) = temp_project(&[
        (
            "tester.luar",
            "local mod = require(\"./grag\")

local var = require(\"./tester\")

tester

require(\"test\")

return true
",
        ),
        ("grag.luar", "return 1"),
        ("test.luar", "local back = require(\"./tester\")
return 2"),
    ]);
    let tester = root.join("tester.luar");
    let info = p.file(&tester).unwrap();
    let cycles: Vec<&luar_lsp::Diagnostic> = info
        .diagnostics
        .iter()
        .filter(|d| d.message.contains("require cycle"))
        .collect();
    assert!(
        cycles.iter().any(|d| d.message.contains("./tester")),
        "self-require not warned: {:?}",
        info.diagnostics
    );
    assert!(
        cycles.iter().any(|d| d.message.contains("\"test\"")),
        "bare require statement cycle not warned: {:?}",
        info.diagnostics
    );
}

#[test]
fn require_inside_function_counts_for_cycles() {
    let (root, p) = temp_project(&[
        (
            "a.luar",
            "local function load()
    return require(\"./b\")
end
return 1",
        ),
        ("b.luar", "local a = require(\"./a\")
return 2"),
    ]);
    let a = root.join("a.luar");
    let info = p.file(&a).unwrap();
    assert!(
        info.diagnostics
            .iter()
            .any(|d| d.message.contains("require cycle")),
        "{:?}",
        info.diagnostics
    );
}

#[test]
fn alias_reload_picks_up_new_aliases() {
    let (root, mut p) = temp_project(&[
        ("Scenes/Settings.luar", "return { volume = 5 }"),
        ("main.luar", "local s = require(\"@Settings\")
local v = s.volume
"),
    ]);
    let main = root.join("main.luar");
    assert_eq!(
        p.file(&main).unwrap().analysis.type_of("v"),
        Some(&luar_lsp::Type::Unknown),
        "alias should be unresolved before config exists"
    );
    std::fs::write(
        root.join("luari.json"),
        "{\"aliases\": {\"Settings\": \"./Scenes/Settings\"}}",
    )
    .unwrap();
    p.reload_aliases();
    assert_eq!(
        p.file(&main).unwrap().analysis.type_of("v"),
        Some(&luar_lsp::Type::Number),
        "alias not picked up after reload"
    );
    let items = p.complete_require(&main, "@");
    assert!(items.contains(&"@Settings".to_string()));
}
