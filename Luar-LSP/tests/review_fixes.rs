use luar_lsp::infer::Analysis;
use luar_lsp::type_syntax::parse_type;
use luar_lsp::types::Type;

fn analyze(src: &str) -> Analysis {
    luar_lsp::analyze_source(src).expect("source should parse")
}

fn ty(analysis: &Analysis, name: &str) -> Type {
    analysis
        .type_of(name)
        .unwrap_or_else(|| panic!("no binding named {name}"))
        .clone()
}

#[test]
fn unary_and_binary_minus_resolved_by_arity() {
    let a = analyze(
        "class V {\n  operator -() return 42 end\n  operator -(o) return \"subbed\" end\n}\nlocal a = V()\nlocal x = a - a\nlocal y = -a",
    );
    assert_eq!(ty(&a, "x"), Type::String);
    assert_eq!(ty(&a, "y"), Type::Number);
}

#[test]
fn unary_minus_without_unary_overload_falls_back() {
    let a = analyze(
        "class V {\n  operator -(o) return \"subbed\" end\n}\nlocal a = V()\nlocal y = -a",
    );
    assert_eq!(ty(&a, "y"), Type::Number);
}

#[test]
fn comparison_overloads_return_raw_result() {
    let a = analyze(
        "class C {\n  operator ==(o) return \"yes\" end\n  operator <(o) return 123 end\n}\nlocal a = C()\nlocal b = C()\nlocal e = a == b\nlocal ne = a ~= b\nlocal l = a < b\nlocal g = a > b",
    );
    assert_eq!(ty(&a, "e"), Type::String);
    assert_eq!(ty(&a, "ne"), Type::Boolean);
    assert_eq!(ty(&a, "l"), Type::Number);
    assert_eq!(ty(&a, "g"), Type::Number);
}

#[test]
fn parent_getter_wins_over_own_field() {
    let a = analyze(
        "class P {\n  get x() return \"from getter\" end\n}\nclass C extends P {\n  public x: number = 5\n}\nlocal c = C()\nlocal v = c.x",
    );
    assert_eq!(ty(&a, "v"), Type::String);
}

#[test]
fn parent_field_wins_over_mixin_field() {
    let a = analyze(
        "class P {\n  public x: string = \"p\"\n}\nclass M {\n  public x: number = 1\n}\nclass C extends P mixin M {\n}\nlocal c = C()\nlocal v = c.x",
    );
    assert_eq!(ty(&a, "v"), Type::String);
}

#[test]
fn mixin_only_field_still_found() {
    let a = analyze(
        "class M {\n  public x: number = 1\n}\nclass C mixin M {\n}\nlocal c = C()\nlocal v = c.x",
    );
    assert_eq!(ty(&a, "v"), Type::Number);
}

#[test]
fn mixin_getters_and_statics_not_inherited() {
    let a = analyze(
        "class M {\n  static s: number = 9\n  get g() return 7 end\n}\nclass C mixin M {\n}\nlocal c = C()\nlocal gv = c.g\nlocal sv = C.s",
    );
    assert_eq!(ty(&a, "gv"), Type::Unknown);
    assert_eq!(ty(&a, "sv"), Type::Unknown);
}

#[test]
fn methods_not_reachable_by_dot() {
    let a = analyze(
        "class C {\n  function m() return 1 end\n}\nlocal c = C()\nlocal f = c.m\nlocal g = C.m",
    );
    assert_eq!(ty(&a, "f"), Type::Unknown);
    assert_eq!(ty(&a, "g"), Type::Unknown);
}

#[test]
fn table_constructor_expands_trailing_call() {
    let a = analyze(
        "local function two() return 1, \"s\" end\nlocal t = { two() }\nlocal e = t[2]",
    );
    let expected = Type::union_of(vec![Type::Number, Type::String]);
    assert_eq!(ty(&a, "e"), expected);
}

#[test]
fn enum_reopen_overwrites_variant() {
    let a = analyze("enum E { A }\nenum E { A = \"hi\" }");
    let info = &a.enums["E"];
    assert_eq!(info.variants[0], ("A".to_string(), Type::String));
}

#[test]
fn assert_returns_all_args() {
    let a = analyze("local x, y = assert(1, \"msg\")");
    assert_eq!(ty(&a, "x"), Type::Number);
    assert_eq!(ty(&a, "y"), Type::String);
}

#[test]
fn nil_and_rhs_is_just_nil() {
    let a = analyze("local n = nil\nlocal v = n and 5");
    assert_eq!(ty(&a, "v"), Type::Nil);
}

#[test]
fn conditional_assignment_unions_at_join() {
    let a = analyze(
        "local x = 1\nif os.time() > 0 then\n  x = \"hi\"\nend\nlocal after = x",
    );
    assert_eq!(
        ty(&a, "after"),
        Type::union_of(vec![Type::Number, Type::String])
    );
}

#[test]
fn else_covering_all_branches_drops_fallthrough() {
    let a = analyze(
        "local x = 1\nif os.time() > 0 then\n  x = \"hi\"\nelse\n  x = \"lo\"\nend\nlocal after = x",
    );
    assert_eq!(ty(&a, "after"), Type::String);
}

#[test]
fn while_body_assignment_unions() {
    let a = analyze("local x = 1\nwhile os.time() > 0 do\n  x = \"s\"\nend\nlocal after = x");
    assert_eq!(
        ty(&a, "after"),
        Type::union_of(vec![Type::Number, Type::String])
    );
}

#[test]
fn switch_case_assignment_unions() {
    let a = analyze(
        "local x = 1\nlocal n = 2\nlocal r = switch(n)\n  case 1\n    x = \"s\"\n  end\nend\nlocal after = x",
    );
    assert_eq!(
        ty(&a, "after"),
        Type::union_of(vec![Type::Number, Type::String])
    );
}

#[test]
fn union_in_intersection_displays_with_parens() {
    let t = parse_type("(A | B) & C").unwrap();
    assert_eq!(t.to_string(), "(A | B) & C");
    assert_eq!(parse_type(&t.to_string()).unwrap(), t);
}

#[test]
fn string_literal_display_escapes() {
    let t = parse_type("'a\"b'").unwrap();
    assert_eq!(parse_type(&t.to_string()).unwrap(), t);
    let t = parse_type("\"a\\\\b\"").unwrap();
    assert_eq!(parse_type(&t.to_string()).unwrap(), t);
}

#[test]
fn union_with_function_member_parenthesized() {
    let u = Type::union_of(vec![
        Type::Function(Some(Box::new(luar_lsp::types::FunctionType {
            params: vec![],
            is_vararg: false,
            returns: vec![Type::Number],
            returns_param: None,
            generic_sig: None,
        }))),
        Type::Nil,
    ]);
    assert_eq!(u.to_string(), "(() -> number) | nil");
}

#[test]
fn function_returning_union_parenthesized() {
    let f = Type::Function(Some(Box::new(luar_lsp::types::FunctionType {
        params: vec![],
        is_vararg: false,
        returns: vec![Type::union_of(vec![Type::Number, Type::Nil])],
        returns_param: None,
        generic_sig: None,
    })));
    assert_eq!(f.to_string(), "() -> (number | nil)");
}

#[test]
fn guarded_entry_points_survive_deep_nesting() {
    let mut src = String::from("local x = ");
    for _ in 0..2000 {
        src.push('{');
    }
    src.push('1');
    for _ in 0..2000 {
        src.push('}');
    }
    let program = luar_lsp::parse_source_safe(&src).unwrap();
    let analysis = luar_lsp::identify_program(&program);
    assert!(analysis.type_of("x").is_some());
    let env_src = format!("type T = {}number{}", "{ x: ".repeat(1000), " }".repeat(1000));
    let env = luar_lsp::resolve::TypeEnv::from_source(&env_src).unwrap();
    assert!(env.aliases.contains_key("T"));
}

#[test]
fn class_cycle_does_not_hang() {
    let a = analyze(
        "class A extends B {\n}\nclass B extends A {\n}\nlocal x = A()\nlocal v = x.missing",
    );
    assert_eq!(ty(&a, "v"), Type::Unknown);
}
