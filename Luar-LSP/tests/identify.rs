use luar_lsp::infer::{Analysis, BindingKind};
use luar_lsp::types::{TableType, Type};

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
fn literals() {
    let a = analyze(
        "local n = 1\nlocal f = 3.14\nlocal s = \"hi\"\nlocal b = true\nlocal z = nil",
    );
    assert_eq!(ty(&a, "n"), Type::Number);
    assert_eq!(ty(&a, "f"), Type::Number);
    assert_eq!(ty(&a, "s"), Type::String);
    assert_eq!(ty(&a, "b"), Type::Boolean);
    assert_eq!(ty(&a, "z"), Type::Nil);
}

#[test]
fn missing_inits_are_nil() {
    let a = analyze("local a, b = 1");
    assert_eq!(ty(&a, "a"), Type::Number);
    assert_eq!(ty(&a, "b"), Type::Nil);
}

#[test]
fn multi_return_distributes() {
    let a = analyze("local function f() return 1, \"a\" end\nlocal x, y = f()");
    assert_eq!(ty(&a, "x"), Type::Number);
    assert_eq!(ty(&a, "y"), Type::String);
}

#[test]
fn tables() {
    let a = analyze(
        "local array = { 10, 20, 30 }\nlocal map = { name = \"luar\", version = 1 }\nlocal mixed = { 1, 2, [10] = \"ten\", label = \"x\" }",
    );
    assert_eq!(
        ty(&a, "array"),
        Type::Table(TableType {
            fields: vec![],
            array: Some(Box::new(Type::Number)),
            name: None,
        })
    );
    match ty(&a, "map") {
        Type::Table(tt) => {
            assert_eq!(
                tt.fields,
                vec![
                    ("name".to_string(), Type::String),
                    ("version".to_string(), Type::Number),
                ]
            );
            assert!(tt.array.is_none());
        }
        other => panic!("expected table, got {other}"),
    }
    match ty(&a, "mixed") {
        Type::Table(tt) => {
            assert_eq!(tt.fields, vec![("label".to_string(), Type::String)]);
            assert_eq!(
                tt.array,
                Some(Box::new(Type::union_of(vec![Type::Number, Type::String])))
            );
        }
        other => panic!("expected table, got {other}"),
    }
}

#[test]
fn table_field_access() {
    let a = analyze("local map = { name = \"luar\", version = 1 }\nlocal v = map.version\nlocal e = map.missing");
    assert_eq!(ty(&a, "v"), Type::Number);
    assert_eq!(ty(&a, "e"), Type::Unknown);
}

#[test]
fn class_binding_and_instantiation() {
    let a = analyze(
        "class Point {\n  public x: number = 0\n  public y: number = 0\n  constructor(x, y)\n    self.x = x\n    self.y = y\n  end\n  function length()\n    return math.sqrt(self.x * self.x + self.y * self.y)\n  end\n}\nconst p = Point(3, 4)\nlocal len = p:length()\nlocal px = p.x",
    );
    assert_eq!(ty(&a, "Point"), Type::Class("Point".to_string()));
    assert_eq!(ty(&a, "p"), Type::Instance("Point".to_string()));
    assert_eq!(ty(&a, "p").to_string(), "Point");
    assert_eq!(ty(&a, "len"), Type::Number);
    assert_eq!(ty(&a, "px"), Type::Number);
}

#[test]
fn getter_resolves_to_return_type() {
    let a = analyze(
        "class Box {\n  private v: number = 0\n  get value() return self.v end\n  set value(x) self.v = x end\n}\nconst b = Box()\nlocal got = b.value",
    );
    assert_eq!(ty(&a, "got"), Type::Number);
}

#[test]
fn static_members() {
    let a = analyze(
        "class Counter {\n  static total: number = 0\n  static function made() return Counter.total end\n}\nlocal t = Counter.total\nlocal m = Counter.made()",
    );
    assert_eq!(ty(&a, "t"), Type::Number);
    assert_eq!(ty(&a, "m"), Type::Number);
}

#[test]
fn inheritance_resolves_parent_members() {
    let a = analyze(
        "class Animal {\n  public name: string = \"\"\n  function speak() return \"...\" end\n}\nclass Dog extends Animal {\n  override function speak() return \"woof\" end\n}\nlocal d = Dog()\nlocal s = d:speak()\nlocal n = d.name",
    );
    assert_eq!(ty(&a, "d"), Type::Instance("Dog".to_string()));
    assert_eq!(ty(&a, "s"), Type::String);
    assert_eq!(ty(&a, "n"), Type::String);
}

#[test]
fn operator_overload_changes_binary_result() {
    let a = analyze(
        "class Vec {\n  public x: number = 0\n  constructor(x) self.x = x end\n  operator +(o) return Vec(self.x + o.x) end\n}\nlocal a = Vec(1)\nlocal b = Vec(2)\nlocal c = a + b\nlocal d = a.x + b.x",
    );
    assert_eq!(ty(&a, "c"), Type::Instance("Vec".to_string()));
    assert_eq!(ty(&a, "d"), Type::Number);
}

#[test]
fn enums() {
    let a = analyze(
        "enum Color { Red Green Blue }\nlocal c = Color.Red\nlocal e = Color\nenum Color { Yellow }\nlocal y = Color.Yellow",
    );
    assert_eq!(ty(&a, "c"), Type::EnumValue("Color".to_string()));
    assert_eq!(ty(&a, "c").to_string(), "Color");
    assert_eq!(ty(&a, "e"), Type::Enum("Color".to_string()));
    assert_eq!(ty(&a, "y"), Type::EnumValue("Color".to_string()));
    let variants = &a.enums["Color"].variants;
    assert_eq!(variants.len(), 4);
}

#[test]
fn enum_declared_inside_function_is_global() {
    let a = analyze("function make()\n  enum Dir { North South }\nend\nmake()\nlocal n = Dir.North");
    assert_eq!(ty(&a, "n"), Type::EnumValue("Dir".to_string()));
}

#[test]
fn switch_expression_unions_case_returns() {
    let a = analyze(
        "local value = 1\nlocal var = switch(value)\n  case \"test\"\n    return true\n  end\n  case 1\n  end\nend",
    );
    assert_eq!(
        ty(&a, "var"),
        Type::union_of(vec![Type::Boolean, Type::Nil])
    );
}

#[test]
fn switch_with_default_and_all_returns() {
    let a = analyze(
        "local n = 2\nlocal label = switch(n)\n  case 1 return \"one\" end\n  case 2 return \"two\" end\n  default return \"other\" end\nend",
    );
    assert_eq!(ty(&a, "label"), Type::String);
}

#[test]
fn logical_operators_yield_operand_unions() {
    let a = analyze(
        "local function maybe() return nil end\nlocal x = maybe() or 5\nlocal y = false or \"s\"\nlocal z = 1 and \"yes\"",
    );
    assert_eq!(ty(&a, "x"), Type::Number);
    assert_eq!(
        ty(&a, "y"),
        Type::union_of(vec![Type::Boolean, Type::String])
    );
    assert_eq!(ty(&a, "z"), Type::String);
}

#[test]
fn function_signature_inferred() {
    let a = analyze("local function add(a, b) return a + b end");
    match ty(&a, "add") {
        Type::Function(Some(sig)) => {
            let names: Vec<&str> = sig.params.iter().map(|p| p.name.as_str()).collect();
            assert_eq!(names, vec!["a", "b"]);
            assert!(!sig.is_vararg);
            assert_eq!(sig.returns, vec![Type::Number]);
        }
        other => panic!("expected function, got {other}"),
    }
}

#[test]
fn function_conditional_returns_union() {
    let a = analyze(
        "local function pick(c)\n  if c then\n    return 1\n  end\n  return \"s\"\nend\nlocal r = pick(true)",
    );
    assert_eq!(
        ty(&a, "r"),
        Type::union_of(vec![Type::Number, Type::String])
    );
}

#[test]
fn recursive_local_function() {
    let a = analyze("local function loop(n) if n then return loop(n) end end");
    assert!(matches!(ty(&a, "loop"), Type::Function(Some(_))));
}

#[test]
fn builtins_seeded() {
    let a = analyze(
        "local s = string.upper(\"x\")\nlocal n = math.sqrt(2)\nlocal t = type(1)\nlocal num = tonumber(\"5\")\nlocal parts = string.split(\"a,b\", \",\")",
    );
    assert_eq!(ty(&a, "s"), Type::String);
    assert_eq!(ty(&a, "n"), Type::Number);
    assert_eq!(ty(&a, "t"), Type::String);
    assert_eq!(ty(&a, "num"), Type::union_of(vec![Type::Number, Type::Nil]));
    match ty(&a, "parts") {
        Type::Table(tt) => assert_eq!(tt.array, Some(Box::new(Type::String))),
        other => panic!("expected table, got {other}"),
    }
}

#[test]
fn coroutines_are_threads() {
    let a = analyze(
        "local co = coroutine.create(function() end)\nlocal ok, v = coroutine.resume(co)\nlocal st = coroutine.status(co)",
    );
    assert_eq!(ty(&a, "co"), Type::Thread);
    assert_eq!(ty(&a, "ok"), Type::Boolean);
    assert_eq!(ty(&a, "st"), Type::String);
}

#[test]
fn pcall_prepends_boolean() {
    let a = analyze("local ok, v = pcall(function() return 5 end)");
    assert_eq!(ty(&a, "ok"), Type::Boolean);
    assert_eq!(ty(&a, "v"), Type::Number);
}

#[test]
fn operators_produce_expected_types() {
    let a = analyze(
        "local c = \"a\" .. 1\nlocal q = 1 == 2\nlocal l = #\"abc\"\nlocal m = -5\nlocal nt = not nil",
    );
    assert_eq!(ty(&a, "c"), Type::String);
    assert_eq!(ty(&a, "q"), Type::Boolean);
    assert_eq!(ty(&a, "l"), Type::Number);
    assert_eq!(ty(&a, "m"), Type::Number);
    assert_eq!(ty(&a, "nt"), Type::Boolean);
}

#[test]
fn bare_assignment_declares_immutable_local() {
    let a = analyze("greeting = \"hi\"");
    let b = a.binding("greeting").unwrap();
    assert_eq!(b.ty, Type::String);
    assert_eq!(b.kind, BindingKind::BareAssign);
}

#[test]
fn compound_assignment() {
    let a = analyze("local n = 1\nn += 2\nlocal s = \"a\"\ns ..= \"b\"");
    assert_eq!(ty(&a, "n"), Type::Number);
    assert_eq!(ty(&a, "s"), Type::String);
}

#[test]
fn loop_variables() {
    let a = analyze(
        "for i = 1, 10 do end\nlocal list = { 1, 2 }\nfor j, v in ipairs(list) do end\nlocal map = { name = \"x\", n = 1 }\nfor k, val in pairs(map) do end",
    );
    let loops: Vec<_> = a
        .bindings
        .iter()
        .filter(|b| b.kind == BindingKind::LoopVar)
        .collect();
    assert_eq!(loops[0].ty, Type::Number);
    assert_eq!(loops[1].ty, Type::Number);
    assert_eq!(loops[2].ty, Type::Number);
    assert_eq!(loops[3].ty, Type::String);
    assert_eq!(
        loops[4].ty,
        Type::union_of(vec![Type::String, Type::Number])
    );
}

#[test]
fn module_pattern() {
    let a = analyze(
        "local M = {}\nfunction M.hello(name) return \"hi \" .. name end\nM.answer = 42\nlocal h = M.hello(\"luar\")\nreturn M",
    );
    assert_eq!(ty(&a, "h"), Type::String);
    match &a.module_returns[0] {
        Type::Table(tt) => {
            assert!(tt.fields.iter().any(|(n, _)| n == "hello"));
            assert!(
                tt.fields
                    .iter()
                    .any(|(n, t)| n == "answer" && *t == Type::Number)
            );
        }
        other => panic!("expected table module return, got {other}"),
    }
}

#[test]
fn method_declared_with_colon_gets_self() {
    let a = analyze(
        "local M = { count = 1 }\nfunction M:bump() return self end\nlocal r = M:bump()",
    );
    assert!(matches!(ty(&a, "r"), Type::Unknown));
}

#[test]
fn buff_binding_survives_scope() {
    let a = analyze("local function setup()\n  buff 8 cache = \"warm\"\nend\nsetup()\nlocal c = cache");
    assert_eq!(ty(&a, "c"), Type::String);
    let b = a.binding("cache").unwrap();
    assert_eq!(b.kind, BindingKind::Buff);
}

#[test]
fn require_is_unknown_for_now() {
    let a = analyze("local shapes = require(\"./shapes\")");
    assert_eq!(ty(&a, "shapes"), Type::Unknown);
}

#[test]
fn interface_binding() {
    let a = analyze("interface Named { name }\nlocal i = Named");
    assert_eq!(ty(&a, "i"), Type::Interface("Named".to_string()));
}

#[test]
fn pub_declares_globally() {
    let a = analyze("local function publish()\n  pub shared = 42\nend\npublish()\nlocal v = shared");
    assert_eq!(ty(&a, "v"), Type::Number);
}

#[test]
fn casts_and_annotations_are_invisible() {
    let a = analyze("local n = 41 :: number\nlocal x: number = 1\nlocal z = n :: { x: number } | nil");
    assert_eq!(ty(&a, "n"), Type::Number);
    assert_eq!(ty(&a, "x"), Type::Number);
    assert_eq!(ty(&a, "z"), Type::Number);
}

#[test]
fn mixin_members_found() {
    let a = analyze(
        "class Loggable {\n  function log() return \"logged\" end\n}\nclass Service mixin Loggable {\n}\nlocal s = Service()\nlocal r = s:log()",
    );
    assert_eq!(ty(&a, "r"), Type::String);
}

#[test]
fn assert_passes_through_first_arg() {
    let a = analyze("local v = assert(5, \"must be five\")");
    assert_eq!(ty(&a, "v"), Type::Number);
}

#[test]
fn vararg_function() {
    let a = analyze("local function count(...) return #({ ... }) end\nlocal n = count(1, 2)");
    assert_eq!(ty(&a, "n"), Type::Number);
    match ty(&a, "count") {
        Type::Function(Some(sig)) => assert!(sig.is_vararg),
        other => panic!("expected function, got {other}"),
    }
}

#[test]
fn constructor_discovered_fields() {
    let a = analyze(
        "class Holder {\n  constructor(v)\n    self.held = \"x\"\n  end\n}\nlocal h = Holder()\nlocal f = h.held",
    );
    assert_eq!(ty(&a, "f"), Type::String);
}

#[test]
fn identify_expr_works() {
    let program = luar_lsp::parse_source_safe("return 1 + 2").unwrap();
    match &program[0] {
        luar::ast::Stmt::Return { values, .. } => {
            assert_eq!(luar_lsp::identify_expr(&values[0]), Type::Number);
        }
        other => panic!("expected return, got {other:?}"),
    }
}

#[test]
fn deeply_nested_source_does_not_overflow() {
    let mut src = String::from("local x = ");
    for _ in 0..2000 {
        src.push('(');
    }
    src.push('1');
    for _ in 0..2000 {
        src.push(')');
    }
    let result = luar_lsp::analyze_source(&src);
    if let Ok(a) = result {
        assert_eq!(ty(&a, "x"), Type::Number);
    }
}
