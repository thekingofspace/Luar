use luar_lsp::project::{Project, RequireTarget};
use luar_lsp::types::Type;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

static COUNTER: AtomicU32 = AtomicU32::new(0);

fn temp_project(files: &[(&str, &str)]) -> (PathBuf, Project) {
    let id = COUNTER.fetch_add(1, Ordering::SeqCst);
    let root = std::env::temp_dir().join(format!(
        "luar_lsp_test_{}_{}",
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

#[test]
fn require_relative_module() {
    let (root, p) = temp_project(&[
        ("main.luar", "local shapes = require(\"./shapes\")\nlocal a = shapes.area(1)"),
        (
            "shapes.luar",
            "local M = {}\nfunction M.area(r)\n  return r * 3\nend\nreturn M",
        ),
    ]);
    let main = root.join("main.luar");
    match p.resolve_require(&main, "./shapes") {
        RequireTarget::Module(t) => assert!(t.ends_with("shapes.luar")),
        other => panic!("expected module, got {other:?}"),
    }
    let info = p.file(&main).unwrap();
    assert_eq!(info.analysis.type_of("a"), Some(&Type::Number));
    match info.analysis.type_of("shapes") {
        Some(Type::Table(tt)) => {
            assert!(tt.fields.iter().any(|(n, _)| n == "area"));
        }
        other => panic!("expected table, got {other:?}"),
    }
}

#[test]
fn dots_climb_directories() {
    let (root, p) = temp_project(&[
        ("top.luar", "return 1"),
        ("a/mid.luar", "return \"mid\""),
        (
            "a/b/deep.luar",
            "local one = require(\"../mid\")\nlocal two = require(\".../top\")",
        ),
    ]);
    let deep = root.join("a").join("b").join("deep.luar");
    match p.resolve_require(&deep, "../mid") {
        RequireTarget::Module(t) => assert!(t.ends_with("mid.luar")),
        other => panic!("expected module, got {other:?}"),
    }
    match p.resolve_require(&deep, ".../top") {
        RequireTarget::Module(t) => assert!(t.ends_with("top.luar")),
        other => panic!("expected module, got {other:?}"),
    }
    let info = p.file(&deep).unwrap();
    assert_eq!(info.analysis.type_of("one"), Some(&Type::String));
    assert_eq!(info.analysis.type_of("two"), Some(&Type::Number));
}

#[test]
fn alias_resolution() {
    let (root, p) = temp_project(&[
        (
            "luari.json",
            "{\n  \"aliases\": {\n    \"Settings\": \"./Scenes/Settings\",\n    \"Game\": \"./\"\n  }\n}",
        ),
        ("Scenes/Settings.luar", "return { volume = 5 }"),
        ("main.luar", "local s = require(\"@Settings\")\nlocal v = s.volume"),
    ]);
    let main = root.join("main.luar");
    match p.resolve_require(&main, "@Settings") {
        RequireTarget::Module(t) => assert!(t.ends_with("Settings.luar")),
        other => panic!("expected module, got {other:?}"),
    }
    let info = p.file(&main).unwrap();
    assert_eq!(info.analysis.type_of("v"), Some(&Type::Number));
}

#[test]
fn folder_with_init_resolves_to_init() {
    let (root, p) = temp_project(&[
        ("ui/init.luar", "return { kind = \"ui\" }"),
        ("ui/button.luar", "return 1"),
        ("main.luar", "local ui = require(\"./ui\")\nlocal k = ui.kind"),
    ]);
    let main = root.join("main.luar");
    match p.resolve_require(&main, "./ui") {
        RequireTarget::Module(t) => assert!(t.ends_with("init.luar")),
        other => panic!("expected module, got {other:?}"),
    }
    let info = p.file(&main).unwrap();
    assert_eq!(info.analysis.type_of("k"), Some(&Type::String));
}

#[test]
fn directory_without_init_lists_modules() {
    let (root, p) = temp_project(&[
        ("Utils/Common.luar", "return 7"),
        ("Utils/Strings.luar", "return \"s\""),
        ("main.luar", "local utils = require(\"./Utils\")\nlocal c = utils.Common"),
    ]);
    let main = root.join("main.luar");
    match p.resolve_require(&main, "./Utils") {
        RequireTarget::Directory(_, listing) => {
            let names: Vec<&str> = listing.iter().map(|(n, _)| n.as_str()).collect();
            assert_eq!(names, vec!["Common", "Strings"]);
        }
        other => panic!("expected directory, got {other:?}"),
    }
    let info = p.file(&main).unwrap();
    assert_eq!(info.analysis.type_of("c"), Some(&Type::Number));
}

#[test]
fn at_self_in_init_lists_siblings() {
    let (root, p) = temp_project(&[
        ("ui/init.luar", "local parts = require(\"@self\")\nreturn parts"),
        ("ui/button.luar", "return \"button\""),
        ("ui/slider.luar", "return 2"),
    ]);
    let init = root.join("ui").join("init.luar");
    match p.resolve_require(&init, "@self") {
        RequireTarget::Directory(_, listing) => {
            let names: Vec<&str> = listing.iter().map(|(n, _)| n.as_str()).collect();
            assert_eq!(names, vec!["button", "slider"]);
        }
        other => panic!("expected directory, got {other:?}"),
    }
    let info = p.file(&init).unwrap();
    match info.analysis.type_of("parts") {
        Some(Type::Table(tt)) => {
            assert!(
                tt.fields
                    .iter()
                    .any(|(n, t)| n == "button" && *t == Type::String)
            );
            assert!(
                tt.fields
                    .iter()
                    .any(|(n, t)| n == "slider" && *t == Type::Number)
            );
        }
        other => panic!("expected table, got {other:?}"),
    }
}

#[test]
fn cross_module_export_type() {
    let (root, p) = temp_project(&[
        (
            "shapes.luar",
            "export type Shape = { width: number, height: number }\ntype Hidden = number\nlocal M = {}\nreturn M",
        ),
        (
            "main.luar",
            "local shapes = require(\"./shapes\")\nlocal s: shapes.Shape = make()\nlocal h: shapes.Hidden = make()",
        ),
    ]);
    let main = root.join("main.luar");
    let info = p.file(&main).unwrap();
    match info.analysis.type_of("s") {
        Some(Type::Table(tt)) => {
            assert!(tt.fields.iter().any(|(n, _)| n == "width"));
        }
        other => panic!("expected structural table, got {other:?}"),
    }
    assert_eq!(info.analysis.type_of("h"), Some(&Type::Unknown));
}

#[test]
fn luard_declarations_are_ambient_globals() {
    let (root, p) = temp_project(&[
        (
            "lib/engine.luard",
            "class Vector {\n  public x: number = 0\n  public y: number = 0\n  constructor(x, y) end\n}\nfunction log(msg: string): nil\nend\nVERSION = \"1.0\"\nenum Mode { Fast Slow }",
        ),
        (
            "main.luar",
            "local v = Vector(1, 2)\nlocal vx = v.x\nlocal r = log(\"hi\")\nlocal ver = VERSION\nlocal m = Mode.Fast",
        ),
    ]);
    let main = root.join("main.luar");
    let info = p.file(&main).unwrap();
    assert_eq!(
        info.analysis.type_of("v"),
        Some(&Type::Instance("Vector".to_string()))
    );
    assert_eq!(info.analysis.type_of("vx"), Some(&Type::Number));
    assert_eq!(info.analysis.type_of("ver"), Some(&Type::String));
    assert_eq!(
        info.analysis.type_of("m"),
        Some(&Type::EnumValue("Mode".to_string()))
    );
}

#[test]
fn luard_types_usable_in_annotations() {
    let (root, p) = temp_project(&[
        ("lib/types.luard", "export type Id = number\nclass Entity {\n}"),
        ("main.luar", "local id: Id = next_id()\nlocal e: Entity = spawn()"),
    ]);
    let main = root.join("main.luar");
    let info = p.file(&main).unwrap();
    assert_eq!(info.analysis.type_of("id"), Some(&Type::Number));
    assert_eq!(
        info.analysis.type_of("e"),
        Some(&Type::Instance("Entity".to_string()))
    );
}

#[test]
fn update_file_reanalyzes() {
    let (root, mut p) = temp_project(&[("main.luar", "local x = 1")]);
    let main = root.join("main.luar");
    assert_eq!(
        p.file(&main).unwrap().analysis.type_of("x"),
        Some(&Type::Number)
    );
    p.update_file(&main, "local x = \"now a string\"".to_string());
    assert_eq!(
        p.file(&main).unwrap().analysis.type_of("x"),
        Some(&Type::String)
    );
}

#[test]
fn require_completion_lists_targets() {
    let (root, p) = temp_project(&[
        (
            "luari.json",
            "{\"aliases\": {\"Common\": \"./Utils/Common\"}}",
        ),
        ("Utils/Common.luar", "return 1"),
        ("Scenes/Title.luar", "return 1"),
        ("Scenes/Base.luar", "return 1"),
        ("main.luar", "local x = 1"),
    ]);
    let main = root.join("main.luar");
    let items = p.complete_require(&main, "./Scenes/");
    assert_eq!(items, vec!["Base".to_string(), "Title".to_string()]);
    let aliases = p.complete_require(&main, "@");
    assert!(aliases.contains(&"@Common".to_string()));
    assert!(
        !aliases.contains(&"@self".to_string()),
        "@self offered outside init.luar"
    );
    let init = root.join("Scenes").join("init.luar");
    std::fs::write(&init, "return 1").unwrap();
    let p2 = luar_lsp::project::Project::load(&root);
    let aliases = p2.complete_require(&init, "@");
    assert!(
        aliases.contains(&"@self".to_string()),
        "@self missing in init.luar: {aliases:?}"
    );
}
