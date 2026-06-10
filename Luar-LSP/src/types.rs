use std::fmt;

#[derive(Debug, Clone, PartialEq)]
pub enum Type {
    Nil,
    Boolean,
    Number,
    String,
    StringLit(String),
    Thread,
    Function(Option<Box<FunctionType>>),
    Table(TableType),
    Class(String),
    Instance(String),
    Enum(String),
    EnumValue(String),
    Interface(String),
    Union(Vec<Type>),
    Unknown,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParamInfo {
    pub name: String,
    pub ty: Type,
}

impl ParamInfo {
    pub fn untyped(name: impl Into<String>) -> ParamInfo {
        ParamInfo {
            name: name.into(),
            ty: Type::Unknown,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct FunctionType {
    pub params: Vec<ParamInfo>,
    pub is_vararg: bool,
    pub returns: Vec<Type>,
    pub returns_param: Option<usize>,
    pub generic_sig: Option<Box<GenericSig>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GenericSig {
    pub generics: Vec<String>,
    pub param_anns: Vec<Option<crate::type_syntax::TypeExpr>>,
    pub ret_ann: crate::type_syntax::TypeExpr,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct TableType {
    pub fields: Vec<(String, Type)>,
    pub array: Option<Box<Type>>,
    pub name: Option<String>,
}

impl Type {
    pub fn basic_name(&self) -> &'static str {
        match self {
            Type::Nil => "nil",
            Type::Boolean => "boolean",
            Type::Number => "number",
            Type::String => "string",
            Type::StringLit(_) => "string",
            Type::Thread => "thread",
            Type::Function(_) => "function",
            Type::Table(_) => "table",
            Type::Class(_) => "class",
            Type::Instance(_) => "table",
            Type::Enum(_) => "enum",
            Type::EnumValue(_) => "number",
            Type::Interface(_) => "interface",
            Type::Union(_) => "union",
            Type::Unknown => "unknown",
        }
    }

    pub fn union_of(types: Vec<Type>) -> Type {
        let mut flat: Vec<Type> = Vec::new();
        for t in types {
            match t {
                Type::Union(inner) => {
                    for i in inner {
                        if !flat.contains(&i) {
                            flat.push(i);
                        }
                    }
                }
                other => {
                    if !flat.contains(&other) {
                        flat.push(other);
                    }
                }
            }
        }
        match flat.len() {
            0 => Type::Unknown,
            1 => flat.pop().unwrap(),
            _ => {
                if flat.contains(&Type::Unknown) {
                    Type::Unknown
                } else {
                    Type::Union(flat)
                }
            }
        }
    }
}

impl fmt::Display for Type {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Type::Nil => write!(f, "nil"),
            Type::Boolean => write!(f, "boolean"),
            Type::Number => write!(f, "number"),
            Type::String => write!(f, "string"),
            Type::StringLit(s) => write!(f, "\"{s}\""),
            Type::Thread => write!(f, "thread"),
            Type::Function(None) => write!(f, "function"),
            Type::Function(Some(ft)) => write!(f, "{ft}"),
            Type::Table(t) => write!(f, "{t}"),
            Type::Class(name) if name.is_empty() => write!(f, "class"),
            Type::Class(name) => write!(f, "class {name}"),
            Type::Instance(name) => write!(f, "{name}"),
            Type::Enum(name) if name.is_empty() => write!(f, "enum"),
            Type::Enum(name) => write!(f, "enum {name}"),
            Type::EnumValue(name) => write!(f, "{name}"),
            Type::Interface(name) => write!(f, "interface {name}"),
            Type::Union(parts) => {
                for (i, p) in parts.iter().enumerate() {
                    if i > 0 {
                        write!(f, " | ")?;
                    }
                    match p {
                        Type::Function(Some(_)) => write!(f, "({p})")?,
                        _ => write!(f, "{p}")?,
                    }
                }
                Ok(())
            }
            Type::Unknown => write!(f, "unknown"),
        }
    }
}

impl fmt::Display for FunctionType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(")?;
        for (i, p) in self.params.iter().enumerate() {
            if i > 0 {
                write!(f, ", ")?;
            }
            if p.ty == Type::Unknown {
                write!(f, "{}", p.name)?;
            } else {
                write!(f, "{}: {}", p.name, p.ty)?;
            }
        }
        if self.is_vararg {
            if !self.params.is_empty() {
                write!(f, ", ")?;
            }
            write!(f, "...")?;
        }
        write!(f, ") -> ")?;
        match self.returns.len() {
            0 => write!(f, "()"),
            1 => match &self.returns[0] {
                u @ Type::Union(_) => write!(f, "({u})"),
                single => write!(f, "{single}"),
            },
            _ => {
                write!(f, "(")?;
                for (i, r) in self.returns.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{r}")?;
                }
                write!(f, ")")
            }
        }
    }
}

impl fmt::Display for TableType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(name) = &self.name {
            return write!(f, "{name}");
        }
        if self.fields.is_empty() && self.array.is_none() {
            return write!(f, "{{}}");
        }
        write!(f, "{{ ")?;
        let mut first = true;
        if let Some(elem) = &self.array {
            write!(f, "{elem}")?;
            first = false;
        }
        for (name, ty) in &self.fields {
            if !first {
                write!(f, ", ")?;
            }
            write!(f, "{name}: {ty}")?;
            first = false;
        }
        write!(f, " }}")
    }
}
