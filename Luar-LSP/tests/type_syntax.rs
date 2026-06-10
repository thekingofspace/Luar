use luar_lsp::type_syntax::{
    Param, TableTypeExpr, TypeExpr, parse_alias, parse_aliases, parse_type,
};

fn named(n: &str) -> TypeExpr {
    TypeExpr::named(n)
}

#[test]
fn primitive_names() {
    for n in [
        "boolean", "number", "string", "table", "thread", "nil", "any", "unknown", "never",
        "void", "function",
    ] {
        assert_eq!(parse_type(n).unwrap(), named(n));
    }
}

#[test]
fn dotted_names_and_generics() {
    let t = parse_type("module.Thing<T>").unwrap();
    match t {
        TypeExpr::Named(segs) => {
            assert_eq!(segs.len(), 2);
            assert_eq!(segs[0].name, "module");
            assert!(segs[0].args.is_none());
            assert_eq!(segs[1].name, "Thing");
            assert_eq!(segs[1].args.as_ref().unwrap().len(), 1);
        }
        other => panic!("expected named, got {other:?}"),
    }
}

#[test]
fn nested_generic_slots_balance() {
    let t = parse_type("Map<string, Array<number>>").unwrap();
    assert_eq!(t.to_string(), "Map<string, Array<number>>");
}

#[test]
fn literal_types() {
    assert_eq!(
        parse_type("\"on\" | \"off\"").unwrap(),
        TypeExpr::Union(vec![
            TypeExpr::StringLit("on".to_string()),
            TypeExpr::StringLit("off".to_string()),
        ])
    );
    assert_eq!(
        parse_type("0 | 1").unwrap(),
        TypeExpr::Union(vec![
            TypeExpr::NumberLit("0".to_string()),
            TypeExpr::NumberLit("1".to_string()),
        ])
    );
    assert_eq!(
        parse_type("-1").unwrap(),
        TypeExpr::NumberLit("-1".to_string())
    );
}

#[test]
fn binding_order_union_intersection_optional() {
    let t = parse_type("A | B & C?").unwrap();
    assert_eq!(
        t,
        TypeExpr::Union(vec![
            named("A"),
            TypeExpr::Intersection(vec![
                named("B"),
                TypeExpr::Optional(Box::new(named("C"))),
            ]),
        ])
    );
}

#[test]
fn arrow_is_loosest() {
    let t = parse_type("A | B -> C").unwrap();
    match t {
        TypeExpr::Function { params, ret } => {
            assert_eq!(params.len(), 1);
            assert_eq!(
                params[0],
                Param::Positional {
                    name: None,
                    ty: TypeExpr::Union(vec![named("A"), named("B")]),
                }
            );
            assert_eq!(*ret, named("C"));
        }
        other => panic!("expected function, got {other:?}"),
    }
}

#[test]
fn arrow_right_associative() {
    let t = parse_type("A -> B -> C").unwrap();
    match t {
        TypeExpr::Function { ret, .. } => match *ret {
            TypeExpr::Function { .. } => {}
            other => panic!("expected nested function, got {other:?}"),
        },
        other => panic!("expected function, got {other:?}"),
    }
}

#[test]
fn optional_repeats() {
    let t = parse_type("T??").unwrap();
    assert_eq!(
        t,
        TypeExpr::Optional(Box::new(TypeExpr::Optional(Box::new(named("T")))))
    );
}

#[test]
fn table_empty() {
    assert_eq!(
        parse_type("{}").unwrap(),
        TypeExpr::Table(TableTypeExpr::Empty)
    );
}

#[test]
fn table_record() {
    let t = parse_type("{ x: number, y: number }").unwrap();
    match t {
        TypeExpr::Table(TableTypeExpr::Record(fields)) => {
            assert_eq!(fields.len(), 2);
            assert_eq!(fields[0].name, "x");
            assert!(!fields[0].optional);
        }
        other => panic!("expected record, got {other:?}"),
    }
}

#[test]
fn table_record_optional_field_and_equals_and_semicolons() {
    let t = parse_type("{ name?: string; level = 1 }").unwrap();
    match t {
        TypeExpr::Table(TableTypeExpr::Record(fields)) => {
            assert!(fields[0].optional);
            assert_eq!(fields[1].name, "level");
            assert_eq!(fields[1].ty, TypeExpr::NumberLit("1".to_string()));
        }
        other => panic!("expected record, got {other:?}"),
    }
}

#[test]
fn table_index_signature() {
    let t = parse_type("{ [string]: number }").unwrap();
    match t {
        TypeExpr::Table(TableTypeExpr::Indexer { key, value }) => {
            assert_eq!(*key, named("string"));
            assert_eq!(*value, named("number"));
        }
        other => panic!("expected indexer, got {other:?}"),
    }
}

#[test]
fn table_array_shorthand() {
    let t = parse_type("{ number }").unwrap();
    assert_eq!(
        t,
        TypeExpr::Table(TableTypeExpr::Array(Box::new(named("number"))))
    );
}

#[test]
fn table_array_shorthand_multiline() {
    let t = parse_type("{\n  C\n}").unwrap();
    assert_eq!(t, TypeExpr::Table(TableTypeExpr::Array(Box::new(named("C")))));
}

#[test]
fn table_array_of_optional() {
    let t = parse_type("{ Foo? }").unwrap();
    assert_eq!(
        t,
        TypeExpr::Table(TableTypeExpr::Array(Box::new(TypeExpr::Optional(
            Box::new(named("Foo"))
        ))))
    );
}

#[test]
fn record_of_methods() {
    let t = parse_type("{\n  test: (self: test) -> ()\n}").unwrap();
    match t {
        TypeExpr::Table(TableTypeExpr::Record(fields)) => match &fields[0].ty {
            TypeExpr::Function { params, ret } => {
                assert_eq!(
                    params[0],
                    Param::Positional {
                        name: Some("self".to_string()),
                        ty: named("test"),
                    }
                );
                assert_eq!(**ret, TypeExpr::Tuple(vec![]));
            }
            other => panic!("expected function field, got {other:?}"),
        },
        other => panic!("expected record, got {other:?}"),
    }
}

#[test]
fn function_types() {
    assert_eq!(parse_type("() -> nil").unwrap().to_string(), "() -> nil");
    assert_eq!(
        parse_type("(number, string) -> boolean").unwrap().to_string(),
        "(number, string) -> boolean"
    );
    assert_eq!(
        parse_type("(x: number, y: number) -> number")
            .unwrap()
            .to_string(),
        "(x: number, y: number) -> number"
    );
}

#[test]
fn function_type_with_vararg() {
    let t = parse_type("(string, ...number) -> nil").unwrap();
    match t {
        TypeExpr::Function { params, .. } => {
            assert_eq!(params.len(), 2);
            assert_eq!(
                params[1],
                Param::Vararg {
                    ty: Some(named("number")),
                }
            );
        }
        other => panic!("expected function, got {other:?}"),
    }
    let t = parse_type("(...) -> nil").unwrap();
    match t {
        TypeExpr::Function { params, .. } => {
            assert_eq!(params[0], Param::Vararg { ty: None });
        }
        other => panic!("expected function, got {other:?}"),
    }
}

#[test]
fn parens_group_single_type() {
    assert_eq!(parse_type("(number)").unwrap(), named("number"));
}

#[test]
fn postfix_binds_tighter_than_arrow_in_return() {
    let t = parse_type("(number) -> boolean?").unwrap();
    match t {
        TypeExpr::Function { ret, .. } => {
            assert_eq!(*ret, TypeExpr::Optional(Box::new(named("boolean"))));
        }
        other => panic!("expected function, got {other:?}"),
    }
}

#[test]
fn alias_simple() {
    let a = parse_alias("type Id = number").unwrap();
    assert_eq!(a.name, "Id");
    assert!(!a.exported);
    assert!(a.generics.is_empty());
    assert_eq!(a.ty, named("number"));
}

#[test]
fn alias_exported_generic() {
    let a = parse_alias("export type Box<T> = { value: T }").unwrap();
    assert!(a.exported);
    assert_eq!(a.generics, vec!["T".to_string()]);
}

#[test]
fn aliases_multiple() {
    let all = parse_aliases("type A = number type B = \"x\" | \"y\"\nexport type C<T, U> = (T) -> U").unwrap();
    assert_eq!(all.len(), 3);
    assert_eq!(all[2].generics.len(), 2);
}

#[test]
fn display_round_trips() {
    for src in [
        "number",
        "Array<number>",
        "Map<string, Array<number>>",
        "module.Thing<T>",
        "\"on\" | \"off\"",
        "{ x: number, y: number }",
        "{ name?: string }",
        "{ [string]: number }",
        "{ number }",
        "() -> nil",
        "(number, string) -> boolean",
        "(x: number, y: number) -> number",
        "(string, ...number) -> nil",
        "number?",
        "A | B & C?",
    ] {
        let once = parse_type(src).unwrap();
        let again = parse_type(&once.to_string()).unwrap();
        assert_eq!(once, again, "display round-trip failed for {src}");
    }
}

#[test]
fn deep_nesting_is_guarded_not_crashing() {
    let mut src = String::new();
    for _ in 0..500 {
        src.push('(');
    }
    src.push_str("number");
    for _ in 0..500 {
        src.push(')');
    }
    assert!(parse_type(&src).is_err());
}

#[test]
fn errors() {
    assert!(parse_type("").is_err());
    assert!(parse_type("\"unterminated").is_err());
    assert!(parse_type("{ x: }").is_err());
    assert!(parse_type("number |").is_err());
    assert!(parse_type("number extra").is_err());
    assert!(parse_type("Foo<number").is_err());
}
