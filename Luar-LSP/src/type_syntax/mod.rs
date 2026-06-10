mod ast;
mod lexer;
mod parser;

pub use ast::{BASIC_TYPES, NameSeg, Param, TableField, TableTypeExpr, TypeAlias, TypeExpr};
pub use lexer::TypeSyntaxError;
pub use parser::{parse_alias, parse_aliases, parse_type, parse_type_prefix};
