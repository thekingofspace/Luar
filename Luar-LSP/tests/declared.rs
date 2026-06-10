use luar_lsp::infer::Analysis;
use luar_lsp::types::Type;

fn analyze(src: &str) -> Analysis {
    luar_lsp::analyze_source(src).expect("source should parse")
}

#[test]
fn class_members_catalogued() {
    let a = analyze(
        "abstract class Animal {\n  public name: string = \"a\"\n  private id: number = 0\n  static count: number = 0\n  constructor(n) self.name = n end\n  abstract function speak() end\n  get tired() return false end\n  set tired(v) self.id = 1 end\n  operator +(o) return self end\n  final function tag() return \"x\" end\n}",
    );
    let c = &a.classes["Animal"];
    assert!(c.is_abstract);
    assert!(!c.is_final);
    assert_eq!(c.parent, None);

    let name_field = c.fields.iter().find(|f| f.name == "name").unwrap();
    assert_eq!(name_field.ty, Type::String);
    assert!(!name_field.is_static);

    let count_field = c.fields.iter().find(|f| f.name == "count").unwrap();
    assert!(count_field.is_static);
    assert_eq!(count_field.ty, Type::Number);

    assert_eq!(c.getters[0].name, "tired");
    assert_eq!(c.getters[0].ty, Type::Boolean);
    assert_eq!(c.setters.len(), 1);
    assert_eq!(c.setters[0].0, "tired");

    let tag = c.methods.iter().find(|m| m.name == "tag").unwrap();
    assert_eq!(tag.sig.returns, vec![Type::String]);

    let speak = c.methods.iter().find(|m| m.name == "speak").unwrap();
    assert!(speak.sig.returns.is_empty());

    assert_eq!(c.operators[0].0, "+");
    assert_eq!(
        c.operators[0].1.returns,
        vec![Type::Instance("Animal".to_string())]
    );

    assert!(c.constructor.is_some());
}

#[test]
fn class_header_clauses() {
    let a = analyze(
        "class Base {\n}\nclass Loggable {\n}\ninterface Named { name }\nfinal class Dog extends Base mixin Loggable implements Named {\n  public name: string = \"rex\"\n}",
    );
    let c = &a.classes["Dog"];
    assert!(c.is_final);
    assert_eq!(c.parent, Some("Base".to_string()));
    assert_eq!(c.mixins, vec!["Loggable".to_string()]);
    assert_eq!(c.interfaces, vec!["Named".to_string()]);
}

#[test]
fn enum_variants_with_values() {
    let a = analyze("enum Status { Active = 10 Inactive Banned = 99 }");
    let e = &a.enums["Status"];
    assert_eq!(e.variants.len(), 3);
    assert!(e.variants.iter().all(|(_, t)| *t == Type::Number));
}

#[test]
fn enum_variant_with_string_value() {
    let a = analyze("enum Names { Default = \"anon\" Other }");
    let e = &a.enums["Names"];
    assert_eq!(e.variants[0], ("Default".to_string(), Type::String));
    assert_eq!(e.variants[1], ("Other".to_string(), Type::Number));
}

#[test]
fn interface_members_catalogued() {
    let a = analyze("interface Named { name describe }");
    assert_eq!(
        a.interfaces["Named"],
        vec!["name".to_string(), "describe".to_string()]
    );
}

#[test]
fn method_signature_params_recorded() {
    let a = analyze("class Greeter {\n  function greet(who, punct) return \"hi \" .. who end\n}");
    let m = &a.classes["Greeter"].methods[0];
    let names: Vec<&str> = m.sig.params.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, vec!["who", "punct"]);
    assert_eq!(m.sig.returns, vec![Type::String]);
}
