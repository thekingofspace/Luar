use crate::types::{FunctionType, ParamInfo, TableType, Type};
use std::collections::HashMap;

fn func(params: &[&str], is_vararg: bool, returns: Vec<Type>) -> Type {
    Type::Function(Some(Box::new(FunctionType {
        params: params.iter().map(|p| ParamInfo::untyped(*p)).collect(),
        is_vararg,
        returns,
        returns_param: None,
        generic_sig: None,
    })))
}

fn lib(fields: Vec<(&str, Type)>) -> Type {
    Type::Table(TableType {
        fields: fields
            .into_iter()
            .map(|(n, t)| (n.to_string(), t))
            .collect(),
        array: None,
        name: None,
    })
}

fn string_array() -> Type {
    Type::Table(TableType {
        fields: Vec::new(),
        array: Some(Box::new(Type::String)),
        name: None,
    })
}

pub fn global_env() -> HashMap<String, Type> {
    let mut env = HashMap::new();
    let mut set = |name: &str, ty: Type| {
        env.insert(name.to_string(), ty);
    };

    set("print", func(&[], true, vec![]));
    set("warn", func(&[], true, vec![]));
    set("type", func(&["v"], false, vec![Type::String]));
    set("TypeOf", func(&["v"], false, vec![Type::String]));
    set("tostring", func(&["v"], false, vec![Type::String]));
    set(
        "tonumber",
        func(
            &["v", "base"],
            false,
            vec![Type::union_of(vec![Type::Number, Type::Nil])],
        ),
    );
    set("assert", func(&["v", "message"], true, vec![Type::Unknown]));
    set("error", func(&["message", "level"], false, vec![]));
    set(
        "select",
        func(&["n"], true, vec![Type::Unknown]),
    );
    set(
        "pcall",
        func(&["f"], true, vec![Type::Boolean, Type::Unknown]),
    );
    set("rawget", func(&["t", "k"], false, vec![Type::Unknown]));
    set(
        "rawset",
        func(&["t", "k", "v"], false, vec![Type::Table(TableType::default())]),
    );
    set("rawequal", func(&["a", "b"], false, vec![Type::Boolean]));
    set("rawlen", func(&["v"], false, vec![Type::Number]));
    set(
        "ipairs",
        func(
            &["t"],
            false,
            vec![
                Type::Function(None),
                Type::Table(TableType::default()),
                Type::Number,
            ],
        ),
    );
    set(
        "pairs",
        func(
            &["t"],
            false,
            vec![
                Type::Function(None),
                Type::Table(TableType::default()),
                Type::Nil,
            ],
        ),
    );
    set(
        "next",
        func(&["t", "k"], false, vec![Type::Unknown, Type::Unknown]),
    );
    set("collectgarbage", func(&[], false, vec![]));
    set("require", func(&["path"], false, vec![Type::Unknown]));
    set("instanceof", func(&["v", "class"], false, vec![Type::Boolean]));
    set(
        "classname",
        func(
            &["v"],
            false,
            vec![Type::union_of(vec![Type::String, Type::Nil])],
        ),
    );
    set("classof", func(&["v"], false, vec![Type::Unknown]));
    set("superclass", func(&["v"], false, vec![Type::Unknown]));
    set("isabstract", func(&["v"], false, vec![Type::Boolean]));
    set(
        "methodsof",
        func(&["v"], false, vec![string_array()]),
    );
    set(
        "setmetatable",
        func(&["t", "m"], false, vec![Type::Table(TableType::default())]),
    );
    set(
        "getmetatable",
        func(
            &["t"],
            false,
            vec![Type::union_of(vec![
                Type::Table(TableType::default()),
                Type::Nil,
            ])],
        ),
    );

    let num1 = |name: &str| (name.to_string(), func(&["x"], false, vec![Type::Number]));
    let mut math_fields: Vec<(String, Type)> = [
        "abs", "ceil", "floor", "round", "sqrt", "sin", "cos", "tan", "asin", "acos", "atan",
        "exp", "log", "deg", "rad", "sign",
    ]
    .iter()
    .map(|n| num1(n))
    .collect();
    for name in ["pow", "fmod", "max", "min", "clamp", "random"] {
        math_fields.push((name.to_string(), func(&["x", "y"], true, vec![Type::Number])));
    }
    math_fields.push((
        "modf".to_string(),
        func(&["x"], false, vec![Type::Number, Type::Number]),
    ));
    math_fields.push(("randomseed".to_string(), func(&["n"], false, vec![])));
    for c in ["pi", "huge", "maxinteger", "mininteger"] {
        math_fields.push((c.to_string(), Type::Number));
    }
    set(
        "math",
        Type::Table(TableType {
            fields: math_fields,
            array: None,
            name: None,
        }),
    );

    set(
        "string",
        lib(vec![
            ("len", func(&["s"], false, vec![Type::Number])),
            ("sub", func(&["s", "i", "j"], false, vec![Type::String])),
            ("upper", func(&["s"], false, vec![Type::String])),
            ("lower", func(&["s"], false, vec![Type::String])),
            ("rep", func(&["s", "n", "sep"], false, vec![Type::String])),
            ("reverse", func(&["s"], false, vec![Type::String])),
            ("byte", func(&["s", "i", "j"], false, vec![Type::Number])),
            ("char", func(&[], true, vec![Type::String])),
            (
                "find",
                func(
                    &["s", "sub"],
                    false,
                    vec![
                        Type::union_of(vec![Type::Number, Type::Nil]),
                        Type::union_of(vec![Type::Number, Type::Nil]),
                    ],
                ),
            ),
            ("contains", func(&["s", "sub"], false, vec![Type::Boolean])),
            (
                "startswith",
                func(&["s", "prefix"], false, vec![Type::Boolean]),
            ),
            (
                "endswith",
                func(&["s", "suffix"], false, vec![Type::Boolean]),
            ),
            ("trim", func(&["s"], false, vec![Type::String])),
            ("split", func(&["s", "sep"], false, vec![string_array()])),
            ("format", func(&["fmt"], true, vec![Type::String])),
        ]),
    );

    set(
        "table",
        lib(vec![
            ("insert", func(&["t", "pos", "v"], false, vec![])),
            ("remove", func(&["t", "pos"], false, vec![Type::Unknown])),
            (
                "concat",
                func(&["t", "sep", "i", "j"], false, vec![Type::String]),
            ),
            ("unpack", func(&["t", "i", "j"], false, vec![Type::Unknown])),
            (
                "pack",
                func(&[], true, vec![Type::Table(TableType::default())]),
            ),
            ("sort", func(&["t", "comp"], false, vec![])),
            (
                "keys",
                func(&["t"], false, vec![Type::Table(TableType {
                    fields: Vec::new(),
                    array: Some(Box::new(Type::Unknown)),
                    name: None,
                })]),
            ),
            ("clear", func(&["t"], false, vec![])),
        ]),
    );

    set(
        "bit32",
        lib(vec![
            ("band", func(&[], true, vec![Type::Number])),
            ("bor", func(&[], true, vec![Type::Number])),
            ("bxor", func(&[], true, vec![Type::Number])),
            ("bnot", func(&["a"], false, vec![Type::Number])),
            ("lshift", func(&["a", "n"], false, vec![Type::Number])),
            ("rshift", func(&["a", "n"], false, vec![Type::Number])),
            ("arshift", func(&["a", "n"], false, vec![Type::Number])),
        ]),
    );

    set(
        "os",
        lib(vec![
            ("time", func(&[], false, vec![Type::Number])),
            ("clock", func(&[], false, vec![Type::Number])),
        ]),
    );

    set(
        "coroutine",
        lib(vec![
            ("create", func(&["f"], false, vec![Type::Thread])),
            (
                "resume",
                func(&["co"], true, vec![Type::Boolean, Type::Unknown]),
            ),
            ("yield", func(&[], true, vec![Type::Unknown])),
            ("status", func(&["co"], false, vec![Type::String])),
            ("close", func(&["co"], false, vec![Type::Boolean])),
            (
                "running",
                func(&[], false, vec![Type::Thread, Type::Boolean]),
            ),
        ]),
    );

    env
}
