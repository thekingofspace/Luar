use luar_lsp::completion::{self, FileView};
use luar_lsp::project::Project;
use luar_lsp::types::Type;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

static COUNTER: AtomicU32 = AtomicU32::new(0);

fn temp_project(files: &[(&str, &str)]) -> (PathBuf, Project) {
    let id = COUNTER.fetch_add(1, Ordering::SeqCst);
    let root = std::env::temp_dir().join(format!("luar_dotann_{}_{}", std::process::id(), id));
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

const TYPES: &str = "export type DataStore = {\n    GetAsync:(self:DataStore, key:string) -> any,\n    SetAsync:(self:DataStore, key:string, value:any) -> (),\n}\n\nexport type DataStoreService = {\n    GetDataStore:(self:DataStoreService, name:string, scope:string?) -> DataStore,\n}\n\nexport type Services = {\n    DataStoreService: DataStoreService,\n}\n";

fn has_field(ty: &Type, field: &str) -> bool {
    matches!(ty, Type::Table(tt) if tt.fields.iter().any(|(n, _)| n == field))
}

#[test]
fn keyof_resolves_in_plain_annotations() {
    let src = format!("{TYPES}\nlocal key: keyof<Services> = nil\n");
    let (root, p) = temp_project(&[("main.luar", src.as_str())]);
    let info = p.file(&root.join("main.luar")).unwrap();
    assert_eq!(
        info.analysis.type_of("key"),
        Some(&Type::StringLit("DataStoreService".into()))
    );
}

#[test]
fn dotted_function_annotations_attach() {
    let src = format!(
        "{TYPES}\nconst game = {{}}\n\nfunction game.GetService<service>(Service:keyof<Services>):ValueOf<Services, service>\n\nend\n\nlocal svc = game.GetService(\"DataStoreService\")\n"
    );
    let (root, p) = temp_project(&[("main.luar", src.as_str())]);
    let info = p.file(&root.join("main.luar")).unwrap();
    let svc = info.analysis.type_of("svc").expect("svc binding");
    assert!(has_field(svc, "GetDataStore"), "{svc:?}");
}

#[test]
fn colon_call_with_explicit_self_narrows_generics() {
    let src = format!(
        "{TYPES}\nconst game = {{}}\n\nfunction game.GetService<service>(self:any, Service:keyof<Services>):ValueOf<Services, service>\n\nend\n\nlocal svc = game:GetService(\"DataStoreService\")\n"
    );
    let (root, p) = temp_project(&[("main.luar", src.as_str())]);
    let info = p.file(&root.join("main.luar")).unwrap();
    let svc = info.analysis.type_of("svc").expect("svc binding");
    assert!(has_field(svc, "GetDataStore"), "{svc:?}");
}

#[test]
fn type_functions_resolve_from_luard_files() {
    let luard = format!(
        "{TYPES}\nconst game = {{}}\n\nfunction game.GetService<service>(self:any, Service:keyof<Services>):ValueOf<Services, service>\n\nend\n"
    );
    let main = "local svc = game:GetService(\"DataStoreService\")\n";
    let (root, p) = temp_project(&[("lib/game.luard", luard.as_str()), ("main.luar", main)]);
    let info = p.file(&root.join("main.luar")).unwrap();
    let svc = info.analysis.type_of("svc").expect("svc binding");
    assert!(has_field(svc, "GetDataStore"), "{svc:?}");
}

#[test]
fn luard_files_share_aliases_with_each_other() {
    let game_luard = "const game = {}\n\nfunction game.GetService<service>(self:any, Service:keyof<Services>):ValueOf<Services, service>\n\nend\n";
    let main = "local svc = game:GetService(\"DataStoreService\")\n";
    let (root, p) = temp_project(&[
        ("lib/a_types.luard", TYPES),
        ("lib/b_game.luard", game_luard),
        ("main.luar", main),
    ]);
    let info = p.file(&root.join("main.luar")).unwrap();
    let svc = info.analysis.type_of("svc").expect("svc binding");
    assert!(has_field(svc, "GetDataStore"), "{svc:?}");
}

#[test]
fn colon_call_arguments_offer_keyof_literals_past_explicit_self() {
    let src = "export type Services = {\n    DataStoreService: { Kind:string },\n    HttpService: { Kind:string },\n}\n\nconst game = {}\n\nfunction game.GetService<service>(self:any, Service:keyof<Services>):ValueOf<Services, service>\n\nend\n\nlocal svc = game:GetService(\"\n";
    let (root, p) = temp_project(&[("main.luar", src)]);
    let main = root.join("main.luar");
    let view = FileView::from_project(&p, &main).unwrap();
    let line0 = 11;
    let col = "local svc = game:GetService(\"".chars().count();
    let labels: Vec<String> = completion::complete(&view, src, line0, col)
        .into_iter()
        .map(|i| i.label)
        .collect();
    assert!(labels.contains(&"DataStoreService".to_string()), "{labels:?}");
    assert!(labels.contains(&"HttpService".to_string()), "{labels:?}");
}

#[test]
fn dot_call_arguments_still_offer_literals_at_position_zero() {
    let src = "export type Services = {\n    DataStoreService: { Kind:string },\n    HttpService: { Kind:string },\n}\n\nconst game = {}\n\nfunction game.GetService<service>(Service:keyof<Services>):ValueOf<Services, service>\n\nend\n\nlocal svc = game.GetService(\"\n";
    let (root, p) = temp_project(&[("main.luar", src)]);
    let main = root.join("main.luar");
    let view = FileView::from_project(&p, &main).unwrap();
    let line0 = 11;
    let col = "local svc = game.GetService(\"".chars().count();
    let labels: Vec<String> = completion::complete(&view, src, line0, col)
        .into_iter()
        .map(|i| i.label)
        .collect();
    assert!(labels.contains(&"DataStoreService".to_string()), "{labels:?}");
    assert!(labels.contains(&"HttpService".to_string()), "{labels:?}");
}

#[test]
fn generic_function_type_alias_narrows_at_call_sites() {
    let src = "export type Services = {\n    DataStoreService: { Kind:string },\n    HttpService: { Kind:string },\n}\n\ntype get = <_, service>(self:any, Service:keyof<Services>) -> ValueOf<Services, service>\n\nlocal test: get = nil\n\nlocal var = test(nil, \"DataStoreService\")\n";
    let (root, p) = temp_project(&[("main.luar", src)]);
    let info = p.file(&root.join("main.luar")).unwrap();
    let var = info.analysis.type_of("var").expect("var binding");
    assert!(has_field(var, "Kind"), "{var:?}");
}

#[test]
fn else_completion_realigns_to_parent_if_indent() {
    let src = "function f(x)\n    if x then\n        local y = 1\n        els\nend\n";
    let (root, p) = temp_project(&[("main.luar", src)]);
    let main = root.join("main.luar");
    let view = FileView::from_project(&p, &main).unwrap();
    let line0 = 3;
    let col = "        els".chars().count();
    let items = completion::complete(&view, src, line0, col);
    let else_item = items
        .iter()
        .find(|i| i.label == "else")
        .expect("else keyword offered");
    let edit = else_item.extra_edit.as_ref().expect("indent edit attached");
    assert_eq!(edit.line0, 3);
    assert_eq!(edit.start_col, 0);
    assert_eq!(edit.end_col, 8);
    assert_eq!(edit.new_text, "    ");
}

#[test]
fn else_completion_keeps_correct_indent_untouched() {
    let src = "function f(x)\n    if x then\n        local y = 1\n    els\nend\n";
    let (root, p) = temp_project(&[("main.luar", src)]);
    let main = root.join("main.luar");
    let view = FileView::from_project(&p, &main).unwrap();
    let line0 = 3;
    let col = "    els".chars().count();
    let items = completion::complete(&view, src, line0, col);
    let else_item = items
        .iter()
        .find(|i| i.label == "else")
        .expect("else keyword offered");
    assert!(else_item.extra_edit.is_none());
}

#[test]
fn generic_type_packs_expand_through_aliases() {
    let src = "export type Connection = { Connected: boolean }\n\nexport type Signal<T...> = {\n    Connect:(self:Signal<T...>, callback:(T...) -> () ) -> Connection,\n    Wait:(self:Signal<T...>) -> T...,\n    Fire:(self:Signal<T...>, T...) -> (),\n}\n\nlocal sig: Signal<number, string> = nil\n\nlocal conn = sig:Connect(function(a, b)\nend)\n\nlocal x, y = sig:Wait()\n";
    let (root, p) = temp_project(&[("main.luar", src)]);
    let info = p.file(&root.join("main.luar")).unwrap();
    let conn = info.analysis.type_of("conn").expect("conn binding");
    assert!(has_field(conn, "Connected"), "{conn:?}");
    assert_eq!(info.analysis.type_of("x"), Some(&Type::Number));
    assert_eq!(info.analysis.type_of("y"), Some(&Type::String));
}

#[test]
fn callback_parameters_get_contextual_types() {
    let src = "export type Connection = { Connected: boolean }\n\nexport type Signal<T...> = {\n    Connect:(self:Signal<T...>, callback:(T...) -> () ) -> Connection,\n}\n\nlocal sig: Signal<number, string> = nil\n\nsig:Connect(function(var, name)\n    pub local got_v = var\n    pub local got_n = name\nend)\n";
    let (root, p) = temp_project(&[("main.luar", src)]);
    let info = p.file(&root.join("main.luar")).unwrap();
    assert_eq!(info.analysis.type_of("got_v"), Some(&Type::Number));
    assert_eq!(info.analysis.type_of("got_n"), Some(&Type::String));
}

#[test]
fn pack_generics_accept_any_arity() {
    let src = "export type Signal<T...> = {\n    Wait:(self:Signal<T...>) -> T...,\n}\n\nlocal a: Signal<string, number> = nil\nlocal b: Signal<number> = nil\nlocal c: Signal<> = nil\n";
    let (root, p) = temp_project(&[("main.luar", src)]);
    let info = p.file(&root.join("main.luar")).unwrap();
    assert!(
        !info
            .diagnostics
            .iter()
            .any(|d| d.message.contains("generic argument")),
        "{:?}",
        info.diagnostics
    );
}

#[test]
fn fixed_generic_arity_still_enforced() {
    let src = "export type Box<T> = { value: T }\n\nlocal a: Box<number, string> = nil\n";
    let (root, p) = temp_project(&[("main.luar", src)]);
    let info = p.file(&root.join("main.luar")).unwrap();
    assert!(
        info.diagnostics
            .iter()
            .any(|d| d.message.contains("expects 1 generic argument(s), got 2")),
        "{:?}",
        info.diagnostics
    );
}

#[test]
fn contextual_callback_params_produce_inlay_hints() {
    let src = "export type Connection = { Connected: boolean }\n\nexport type Signal<T...> = {\n    Connect:(self:Signal<T...>, callback:(T...) -> () ) -> Connection,\n}\n\nlocal sig: Signal<number, string> = nil\n\nsig:Connect(function(var, name)\nend)\n";
    let (root, p) = temp_project(&[("main.luar", src)]);
    let info = p.file(&root.join("main.luar")).unwrap();
    let hints = &info.analysis.param_hints;
    assert!(
        hints
            .iter()
            .any(|(l, n, t)| *l == 9 && n == "var" && *t == Type::Number),
        "{hints:?}"
    );
    assert!(
        hints
            .iter()
            .any(|(l, n, t)| *l == 9 && n == "name" && *t == Type::String),
        "{hints:?}"
    );
}

#[test]
fn callback_param_members_complete_inside_lambda() {
    let src = "export type Player = { Name:string, UserId:number }\n\nexport type Signal<T...> = {\n    Connect:(self:Signal<T...>, callback:(T...) -> () ) -> any,\n}\n\nlocal PlayerAdded: Signal<Player> = nil\n\nPlayerAdded:Connect(function(Player:Player)\n    Player.\nend)\n";
    let (root, p) = temp_project(&[("main.luar", src)]);
    let main = root.join("main.luar");
    let view = FileView::from_project(&p, &main).unwrap();
    let line0 = 9;
    let col = "    Player.".chars().count();
    let labels: Vec<String> = completion::complete(&view, src, line0, col)
        .into_iter()
        .map(|i| i.label)
        .collect();
    assert!(labels.contains(&"UserId".to_string()), "{labels:?}");
    assert!(labels.contains(&"Name".to_string()), "{labels:?}");
}

#[test]
fn annotated_lambda_params_complete_without_context() {
    let src = "export type Player = { Name:string, UserId:number }\n\nlocal cb = nil\ncb = function(p: Player)\n    p.\nend\n";
    let (root, p) = temp_project(&[("main.luar", src)]);
    let main = root.join("main.luar");
    let view = FileView::from_project(&p, &main).unwrap();
    let line0 = 4;
    let col = "    p.".chars().count();
    let labels: Vec<String> = completion::complete(&view, src, line0, col)
        .into_iter()
        .map(|i| i.label)
        .collect();
    assert!(labels.contains(&"UserId".to_string()), "{labels:?}");
}

#[test]
fn luard_alias_sharing_is_order_independent() {
    let game_luard = "const game = {}\n\nfunction game.GetService<service>(self:any, Service:keyof<Services>):ValueOf<Services, service>\n\nend\n";
    let main = "local svc = game:GetService(\"DataStoreService\")\n";
    let (root, p) = temp_project(&[
        ("lib/a_game.luard", game_luard),
        ("lib/z_types.luard", TYPES),
        ("main.luar", main),
    ]);
    let info = p.file(&root.join("main.luar")).unwrap();
    let svc = info.analysis.type_of("svc").expect("svc binding");
    assert!(has_field(svc, "GetDataStore"), "{svc:?}");
}
