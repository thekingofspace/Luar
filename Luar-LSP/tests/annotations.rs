use luar_lsp::annotations::scan;
use luar_lsp::infer::InferOptions;
use luar_lsp::resolve::TypeEnv;
use luar_lsp::type_syntax::TypeExpr;
use luar_lsp::types::Type;

#[test]
fn local_annotation_recorded() {
    let ann = scan("local x: number = \"oops\"");
    assert_eq!(ann.var("x", 1), Some(&TypeExpr::named("number")));
}

#[test]
fn multi_name_annotations() {
    let ann = scan("local a: number, b: string = f()");
    assert!(ann.var("a", 1).is_some());
    assert_eq!(ann.var("b", 1), Some(&TypeExpr::named("string")));
}

#[test]
fn pub_const_and_bare_annotations() {
    let ann = scan("pub const flag: boolean = true\nspeed: number = 1");
    assert!(ann.var("flag", 1).is_some());
    assert!(ann.var("speed", 2).is_some());
}

#[test]
fn complex_annotation_with_equals_inside_braces() {
    let ann = scan("local cfg: { debug = boolean, mode: \"on\" | \"off\" } = make()");
    let texpr = ann.var("cfg", 1).expect("cfg annotation");
    assert!(matches!(texpr, TypeExpr::Table(_)));
}

#[test]
fn cast_on_init_records_annotation() {
    let ann = scan("local z = v :: { x: number } | nil");
    assert!(ann.var("z", 1).is_some());
    let ann = scan("y = 10 :: Array<string>");
    assert!(ann.var("y", 1).is_some());
}

#[test]
fn explicit_annotation_beats_cast() {
    let ann = scan("local z: number = v :: string");
    assert_eq!(ann.var("z", 1), Some(&TypeExpr::named("number")));
}

#[test]
fn method_call_is_not_an_annotation() {
    let ann = scan("obj:method()\nobj:method \"sugar\"");
    assert!(ann.vars.is_empty());
}

#[test]
fn function_param_and_return_annotations() {
    let ann = scan("local function clamp(x: number, lo: number): number\n  return x\nend");
    let params = ann.fn_params.get(&("clamp".to_string(), 1)).expect("params");
    assert_eq!(params.get("x"), Some(&TypeExpr::named("number")));
    assert_eq!(
        ann.fn_returns.get(&("clamp".to_string(), 1)),
        Some(&TypeExpr::named("number"))
    );
}

#[test]
fn class_field_and_method_annotations() {
    let src = "class Point {\n  public x: number = 0\n  function move(dx: number): boolean\n    return true\n  end\n  get size(): number return 1 end\n}";
    let ann = scan(src);
    assert_eq!(
        ann.class_fields.get(&("Point".to_string(), "x".to_string())),
        Some(&TypeExpr::named("number"))
    );
    let mp = ann
        .method_params
        .get(&("Point".to_string(), "move".to_string()))
        .expect("method params");
    assert_eq!(mp.get("dx"), Some(&TypeExpr::named("number")));
    assert_eq!(
        ann.method_returns
            .get(&("Point".to_string(), "move".to_string())),
        Some(&TypeExpr::named("boolean"))
    );
    assert_eq!(
        ann.getter_returns
            .get(&("Point".to_string(), "size".to_string())),
        Some(&TypeExpr::named("number"))
    );
}

#[test]
fn export_type_marker_and_generics() {
    let ann = scan("export type Wrapper = { value: number }\ntype Box<T, U> = { value: T }");
    assert!(ann.exported_types.contains("Wrapper"));
    assert!(!ann.exported_types.contains("Box"));
    assert_eq!(
        ann.alias_generics.get("Box"),
        Some(&vec!["T".to_string(), "U".to_string()])
    );
}

#[test]
fn manual_annotation_wins_over_inference() {
    let src = "local x: string = 1\nlocal y = x";
    let program = luar_lsp::parse_source_safe(src).unwrap();
    let ann = scan(src);
    let env = TypeEnv::from_program(&program);
    let opts = InferOptions {
        annotations: Some(&ann),
        env: Some(&env),
        ..InferOptions::default()
    };
    let a = luar_lsp::identify_program_with(&program, &opts);
    assert_eq!(a.type_of("x"), Some(&Type::String));
    assert_eq!(a.type_of("y"), Some(&Type::String));
}

#[test]
fn annotated_class_type_resolves_to_instance() {
    let src = "class Dog {\n}\nlocal d: Dog = make()";
    let program = luar_lsp::parse_source_safe(src).unwrap();
    let ann = scan(src);
    let env = TypeEnv::from_program(&program);
    let opts = InferOptions {
        annotations: Some(&ann),
        env: Some(&env),
        ..InferOptions::default()
    };
    let a = luar_lsp::identify_program_with(&program, &opts);
    assert_eq!(a.type_of("d"), Some(&Type::Instance("Dog".to_string())));
}

#[test]
fn annotated_params_flow_into_body() {
    let src = "local function double(x: number)\n  return x\nend";
    let program = luar_lsp::parse_source_safe(src).unwrap();
    let ann = scan(src);
    let env = TypeEnv::from_program(&program);
    let opts = InferOptions {
        annotations: Some(&ann),
        env: Some(&env),
        ..InferOptions::default()
    };
    let a = luar_lsp::identify_program_with(&program, &opts);
    match a.type_of("double") {
        Some(Type::Function(Some(sig))) => {
            assert_eq!(sig.params[0].ty, Type::Number);
            assert_eq!(sig.returns, vec![Type::Number]);
        }
        other => panic!("expected function, got {other:?}"),
    }
}

#[test]
fn return_annotation_overrides_inferred() {
    let src = "local function mystery(): string | nil\n  return whatever()\nend\nlocal r = mystery()";
    let program = luar_lsp::parse_source_safe(src).unwrap();
    let ann = scan(src);
    let env = TypeEnv::from_program(&program);
    let opts = InferOptions {
        annotations: Some(&ann),
        env: Some(&env),
        ..InferOptions::default()
    };
    let a = luar_lsp::identify_program_with(&program, &opts);
    assert_eq!(
        a.type_of("r"),
        Some(&Type::union_of(vec![Type::String, Type::Nil]))
    );
}

#[test]
fn alias_annotation_resolves_through_env() {
    let src = "type Mode = \"on\" | \"off\"\nlocal mode: Mode = \"on\"";
    let program = luar_lsp::parse_source_safe(src).unwrap();
    let ann = scan(src);
    let env = TypeEnv::from_program(&program);
    let opts = InferOptions {
        annotations: Some(&ann),
        env: Some(&env),
        ..InferOptions::default()
    };
    let a = luar_lsp::identify_program_with(&program, &opts);
    assert_eq!(
        a.type_of("mode"),
        Some(&Type::union_of(vec![
            Type::StringLit("on".to_string()),
            Type::StringLit("off".to_string()),
        ]))
    );
}

#[test]
fn scanner_survives_weird_source() {
    let _ = scan("");
    let _ = scan("local");
    let _ = scan("::");
    let _ = scan("local x: = 1");
    let _ = scan("function (a: ) end");
    let _ = scan("class { public x: }");
    let _ = scan("`interp {x} string`\nlocal y: number = 1");
}
