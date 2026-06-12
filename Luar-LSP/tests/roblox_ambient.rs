use luar_lsp::project::Project;
use luar_lsp::types::Type;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

static COUNTER: AtomicU32 = AtomicU32::new(0);

fn temp_project(files: &[(&str, &str)]) -> (PathBuf, Project) {
    let id = COUNTER.fetch_add(1, Ordering::SeqCst);
    let root = std::env::temp_dir().join(format!("luar_rblx_{}_{}", std::process::id(), id));
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

const AMBIENT: &str = r#"--#disable all
export type Connection = {
    Connected:boolean,
    Disconnect:(self:Connection) -> (),
}

export type Signal<T...> = {
    Connect:(self:Signal<T...>, callback:(T...) -> () ) -> Connection,
    Wait:(self:Signal<T...>) -> T...,
}

export type Player = {
    Name:string,
    UserId:number,
}

export type Players = {
    LocalPlayer:Player,
    GetPlayers:(self:Players) -> {Player},
    PlayerAdded:Signal<Player>,
}

export type DataStoreService = {
    GetDataStore:(self:DataStoreService, name:string, scope:string?) -> any,
}

export type Services = {
    DataStoreService: DataStoreService,
    Players:Players
}

Auto {
    key "DataStoreService" Variable "game:GetService(\"DataStoreService\")"
    key "Players" Variable "game:GetService(\"Players\")"
}

export type DataModel = {
    GetService:<_, service>(self:DataModel, Service:keyof<Services>) -> ValueOf<Services, service>
}

const game:NotNil<DataModel> = nil
"#;

fn has_field(ty: &Type, field: &str) -> bool {
    matches!(ty, Type::Table(tt) if tt.fields.iter().any(|(n, _)| n == field))
}

#[test]
fn full_ambient_datamodel_resolves_service_types() {
    let main = "local svc = game:GetService(\"Players\")\nlocal lp = svc.LocalPlayer\nprint(lp)\n";
    let (root, p) = temp_project(&[("Roblox.luard", AMBIENT), ("main.luar", main)]);
    assert_eq!(p.auto_imports.len(), 2);
    let game = p
        .luard_globals
        .iter()
        .find(|(n, _)| n == "game")
        .expect("game global registered");
    assert!(has_field(&game.1, "GetService"), "{:?}", game.1);
    let info = p.file(&root.join("main.luar")).unwrap();
    let svc = info.analysis.type_of("svc").expect("svc binding");
    assert!(has_field(svc, "LocalPlayer"), "{svc:?}");
    assert!(has_field(svc, "PlayerAdded"), "{svc:?}");
    let lp = info.analysis.type_of("lp").expect("lp binding");
    assert!(has_field(lp, "Name"), "{lp:?}");
    assert!(has_field(lp, "UserId"), "{lp:?}");
}

#[test]
fn colon_angle_without_space_parses_in_type_fields() {
    let src = "export type T = {\n    Get:<g>(self:T, name:string) -> any\n}\nlocal t: T = nil\nprint(t)\n";
    let (root, p) = temp_project(&[("main.luar", src)]);
    let info = p.file(&root.join("main.luar")).unwrap();
    assert!(
        !info
            .diagnostics
            .iter()
            .any(|d| d.message.contains("expected")),
        "{:?}",
        info.diagnostics
    );
}

#[test]
fn auto_blocks_in_luar_files_get_a_clear_diagnostic() {
    let main = "Auto {\n    key \"DS\" Variable \"game:GetService(\\\"DataStoreService\\\")\"\n}\n";
    let (root, p) = temp_project(&[("main.luar", main)]);
    let info = p.file(&root.join("main.luar")).unwrap();
    assert!(
        info.diagnostics
            .iter()
            .any(|d| d.message.contains("Auto blocks only work in .luard")),
        "{:?}",
        info.diagnostics
    );
}
