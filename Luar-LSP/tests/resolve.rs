use luar_lsp::resolve::{Resolved, TypeEnv};
use luar_lsp::type_syntax::{parse_alias, parse_type};
use luar_lsp::types::{TableType, Type};

fn env(src: &str) -> TypeEnv {
    TypeEnv::from_source(src).expect("source should parse")
}

#[test]
fn alias_to_basic() {
    let e = env("type Flag = boolean");
    assert_eq!(e.resolve_name("Flag"), Resolved::Basic(Type::Boolean));
}

#[test]
fn alias_to_enum() {
    let e = env("enum Color { Red Green Blue }\ntype test = Color");
    assert_eq!(e.resolve_name("test"), Resolved::Enum("Color".to_string()));
}

#[test]
fn alias_to_class() {
    let e = env("class Widget {\n}\ntype W = Widget");
    assert_eq!(e.resolve_name("W"), Resolved::Class("Widget".to_string()));
}

#[test]
fn alias_chain() {
    let e = env("type A = B\ntype B = C\ntype C = number");
    assert_eq!(e.resolve_name("A"), Resolved::Basic(Type::Number));
}

#[test]
fn alias_cycle_is_guarded() {
    let e = env("type A = B\ntype B = A");
    assert_eq!(e.resolve_name("A"), Resolved::Unresolved("A".to_string()));
}

#[test]
fn unknown_name() {
    let e = env("local x = 1");
    assert_eq!(
        e.resolve_name("Mystery"),
        Resolved::Unresolved("Mystery".to_string())
    );
}

#[test]
fn basic_names_resolve() {
    let e = TypeEnv::default();
    assert_eq!(e.resolve_name("number"), Resolved::Basic(Type::Number));
    assert_eq!(e.resolve_name("thread"), Resolved::Basic(Type::Thread));
    assert_eq!(e.resolve_name("nil"), Resolved::Basic(Type::Nil));
    assert_eq!(
        e.resolve_name("table"),
        Resolved::Basic(Type::Table(TableType {
            name: Some("table".to_string()),
            ..TableType::default()
        }))
    );
}

#[test]
fn user_names_shadow_basics() {
    let e = env("class string {\n}");
    assert_eq!(
        e.resolve_name("string"),
        Resolved::Class("string".to_string())
    );
}

#[test]
fn structural_alias() {
    let e = env("type Shape = { width: number, height: number }");
    match e.resolve_name("Shape") {
        Resolved::Structural(_) => {}
        other => panic!("expected structural, got {other:?}"),
    }
    let alias = &e.aliases["Shape"];
    match e.value_type(&alias.ty) {
        Type::Table(tt) => {
            assert_eq!(
                tt.fields,
                vec![
                    ("width".to_string(), Type::Number),
                    ("height".to_string(), Type::Number),
                ]
            );
        }
        other => panic!("expected table, got {other}"),
    }
}

#[test]
fn string_literal_alias() {
    let e = env("type Mode = \"on\" | \"off\"");
    let alias = &e.aliases["Mode"];
    assert_eq!(
        e.value_type(&alias.ty),
        Type::union_of(vec![
            Type::StringLit("on".to_string()),
            Type::StringLit("off".to_string()),
        ])
    );
}

#[test]
fn number_literal_in_luar_type_position() {
    let e = env("type X = 5");
    assert_eq!(
        e.resolve_name("X"),
        Resolved::NumberLiteral("5".to_string())
    );
}

#[test]
fn optional_becomes_union_with_nil() {
    let e = env("type N = number?");
    let alias = &e.aliases["N"];
    assert_eq!(
        e.value_type(&alias.ty),
        Type::union_of(vec![Type::Number, Type::Nil])
    );
}

#[test]
fn generic_alias_substitution() {
    let mut e = TypeEnv::default();
    e.define_alias(parse_alias("export type Box<T> = { value: T }").unwrap());
    let usage = parse_type("Box<number>").unwrap();
    match e.value_type(&usage) {
        Type::Table(tt) => {
            assert_eq!(tt.fields, vec![("value".to_string(), Type::Number)]);
        }
        other => panic!("expected table, got {other}"),
    }
}

#[test]
fn generic_alias_resolving_to_class() {
    let mut e = TypeEnv::default();
    e.define_class("Widget");
    e.define_alias(parse_alias("type Of<T> = T").unwrap());
    let usage = parse_type("Of<Widget>").unwrap();
    assert_eq!(e.value_type(&usage), Type::Instance("Widget".to_string()));
}

#[test]
fn annotation_value_types() {
    let e = env("class Dog {\n}\nenum Color { Red }");
    assert_eq!(
        e.value_type(&parse_type("Dog").unwrap()),
        Type::Instance("Dog".to_string())
    );
    assert_eq!(
        e.value_type(&parse_type("Color").unwrap()),
        Type::EnumValue("Color".to_string())
    );
    assert_eq!(
        e.value_type(&parse_type("Dog | nil").unwrap()),
        Type::union_of(vec![Type::Instance("Dog".to_string()), Type::Nil])
    );
    assert_eq!(
        e.value_type(&parse_type("{ Dog }").unwrap()),
        Type::Table(TableType {
            fields: vec![],
            array: Some(Box::new(Type::Instance("Dog".to_string()))),
            name: None,
        })
    );
}

#[test]
fn function_annotation_value_type() {
    let e = TypeEnv::default();
    match e.value_type(&parse_type("(x: number, y: number) -> number").unwrap()) {
        Type::Function(Some(sig)) => {
            let names: Vec<&str> = sig.params.iter().map(|p| p.name.as_str()).collect();
            assert_eq!(names, vec!["x", "y"]);
            assert!(sig.params.iter().all(|p| p.ty == Type::Number));
            assert_eq!(sig.returns, vec![Type::Number]);
        }
        other => panic!("expected function, got {other}"),
    }
    match e.value_type(&parse_type("() -> ()").unwrap()) {
        Type::Function(Some(sig)) => assert!(sig.returns.is_empty()),
        other => panic!("expected function, got {other}"),
    }
}

#[test]
fn declarations_found_in_nested_scopes() {
    let e = env(
        "local function setup()\n  enum Hidden { A }\n  class Inner {\n  }\nend\nif true then\n  type Deep = number\nend",
    );
    assert!(e.enums.contains("Hidden"));
    assert!(e.classes.contains("Inner"));
    assert!(e.aliases.contains_key("Deep"));
}

#[test]
fn export_type_collected() {
    let e = env("export type Wrapper = { value: number }");
    assert!(e.aliases.contains_key("Wrapper"));
}

#[test]
fn documentation_number_types_resolve() {
    let e = TypeEnv::default();
    assert_eq!(e.resolve_name("integer"), Resolved::Basic(Type::Number));
    assert_eq!(e.resolve_name("double"), Resolved::Basic(Type::Number));
    assert_eq!(e.resolve_name("float"), Resolved::Basic(Type::Number));
    assert_eq!(e.value_type(&parse_type("integer | float").unwrap()), Type::Number);
}
