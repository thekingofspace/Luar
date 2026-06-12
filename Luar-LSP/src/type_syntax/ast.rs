use std::fmt;

#[derive(Debug, Clone, PartialEq)]
pub enum TypeExpr {
    Named(Vec<NameSeg>),
    StringLit(String),
    NumberLit(String),
    Optional(Box<TypeExpr>),
    Union(Vec<TypeExpr>),
    Intersection(Vec<TypeExpr>),
    Table(TableTypeExpr),
    Function {
        generics: Vec<String>,
        params: Vec<Param>,
        ret: Box<TypeExpr>,
    },
    Tuple(Vec<TypeExpr>),
    Pack(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct NameSeg {
    pub name: String,
    pub args: Option<Vec<TypeExpr>>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Param {
    Positional {
        name: Option<String>,
        ty: TypeExpr,
    },
    Vararg {
        ty: Option<TypeExpr>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum TableTypeExpr {
    Empty,
    Record(Vec<TableField>),
    Indexer {
        key: Box<TypeExpr>,
        value: Box<TypeExpr>,
    },
    Array(Box<TypeExpr>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct TableField {
    pub name: String,
    pub optional: bool,
    pub ty: TypeExpr,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TypeAlias {
    pub exported: bool,
    pub name: String,
    pub generics: Vec<String>,
    pub ty: TypeExpr,
}

impl TypeExpr {
    pub fn named(name: &str) -> TypeExpr {
        TypeExpr::Named(vec![NameSeg {
            name: name.to_string(),
            args: None,
        }])
    }

    pub fn simple_name(&self) -> Option<&str> {
        match self {
            TypeExpr::Named(segs) if segs.len() == 1 && segs[0].args.is_none() => {
                Some(&segs[0].name)
            }
            _ => None,
        }
    }
}

pub const BASIC_TYPES: [&str; 9] = [
    "thread", "boolean", "string", "class", "enum", "number", "nil", "table", "function",
];

fn needs_parens_in_union(t: &TypeExpr) -> bool {
    matches!(t, TypeExpr::Function { .. })
}

fn needs_parens_in_intersection(t: &TypeExpr) -> bool {
    matches!(t, TypeExpr::Function { .. } | TypeExpr::Union(_))
}

fn write_string_lit(f: &mut fmt::Formatter<'_>, s: &str) -> fmt::Result {
    write!(f, "\"")?;
    for c in s.chars() {
        match c {
            '"' => write!(f, "\\\"")?,
            '\\' => write!(f, "\\\\")?,
            '\n' => write!(f, "\\n")?,
            '\t' => write!(f, "\\t")?,
            '\r' => write!(f, "\\r")?,
            c => write!(f, "{c}")?,
        }
    }
    write!(f, "\"")
}

impl fmt::Display for TypeExpr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TypeExpr::Named(segs) => {
                for (i, seg) in segs.iter().enumerate() {
                    if i > 0 {
                        write!(f, ".")?;
                    }
                    write!(f, "{}", seg.name)?;
                    if let Some(args) = &seg.args {
                        write!(f, "<")?;
                        for (j, a) in args.iter().enumerate() {
                            if j > 0 {
                                write!(f, ", ")?;
                            }
                            write!(f, "{a}")?;
                        }
                        write!(f, ">")?;
                    }
                }
                Ok(())
            }
            TypeExpr::StringLit(s) => write_string_lit(f, s),
            TypeExpr::NumberLit(n) => write!(f, "{n}"),
            TypeExpr::Optional(inner) => match inner.as_ref() {
                TypeExpr::Union(_) | TypeExpr::Intersection(_) | TypeExpr::Function { .. } => {
                    write!(f, "({inner})?")
                }
                _ => write!(f, "{inner}?"),
            },
            TypeExpr::Union(parts) => {
                for (i, p) in parts.iter().enumerate() {
                    if i > 0 {
                        write!(f, " | ")?;
                    }
                    if needs_parens_in_union(p) {
                        write!(f, "({p})")?;
                    } else {
                        write!(f, "{p}")?;
                    }
                }
                Ok(())
            }
            TypeExpr::Intersection(parts) => {
                for (i, p) in parts.iter().enumerate() {
                    if i > 0 {
                        write!(f, " & ")?;
                    }
                    if needs_parens_in_intersection(p) {
                        write!(f, "({p})")?;
                    } else {
                        write!(f, "{p}")?;
                    }
                }
                Ok(())
            }
            TypeExpr::Table(t) => write!(f, "{t}"),
            TypeExpr::Function { generics, params, ret } => {
                if !generics.is_empty() {
                    write!(f, "<")?;
                    for (i, g) in generics.iter().enumerate() {
                        if i > 0 {
                            write!(f, ", ")?;
                        }
                        write!(f, "{g}")?;
                    }
                    write!(f, ">")?;
                } else if params.len() == 1
                    && matches!(
                        &params[0],
                        Param::Positional { name: None, ty }
                            if !matches!(ty, TypeExpr::Function { .. } | TypeExpr::Tuple(_))
                    )
                {
                    if let Param::Positional { ty, .. } = &params[0] {
                        write!(f, "{ty} -> {ret}")?;
                    }
                    return Ok(());
                }
                write!(f, "(")?;
                for (i, p) in params.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{p}")?;
                }
                write!(f, ") -> {ret}")
            }
            TypeExpr::Tuple(parts) => {
                write!(f, "(")?;
                for (i, p) in parts.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{p}")?;
                }
                write!(f, ")")
            }
            TypeExpr::Pack(name) => write!(f, "{name}..."),
        }
    }
}

impl fmt::Display for Param {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Param::Positional { name: Some(n), ty } => write!(f, "{n}: {ty}"),
            Param::Positional { name: None, ty } => write!(f, "{ty}"),
            Param::Vararg { ty: Some(t) } => write!(f, "...{t}"),
            Param::Vararg { ty: None } => write!(f, "..."),
        }
    }
}

impl fmt::Display for TableTypeExpr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TableTypeExpr::Empty => write!(f, "{{}}"),
            TableTypeExpr::Record(fields) => {
                write!(f, "{{ ")?;
                for (i, field) in fields.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", field.name)?;
                    if field.optional {
                        write!(f, "?")?;
                    }
                    write!(f, ": {}", field.ty)?;
                }
                write!(f, " }}")
            }
            TableTypeExpr::Indexer { key, value } => write!(f, "{{ [{key}]: {value} }}"),
            TableTypeExpr::Array(elem) => write!(f, "{{ {elem} }}"),
        }
    }
}

impl fmt::Display for TypeAlias {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.exported {
            write!(f, "export ")?;
        }
        write!(f, "type {}", self.name)?;
        if !self.generics.is_empty() {
            write!(f, "<")?;
            for (i, g) in self.generics.iter().enumerate() {
                if i > 0 {
                    write!(f, ", ")?;
                }
                write!(f, "{g}")?;
            }
            write!(f, ">")?;
        }
        write!(f, " = {}", self.ty)
    }
}
