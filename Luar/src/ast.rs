
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Visibility {

    Local,

    Pub,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mutability {

    Mutable,

    Const,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Stmt {

    Declare {
        visibility: Visibility,
        mutability: Mutability,
        names: Vec<String>,
        inits: Vec<Expr>,

        line: u32,
    },

    Assign {
        targets: Vec<LValue>,
        op: AssignOp,
        values: Vec<Expr>,

        line: u32,
    },

    Do(Vec<Stmt>),

    If {
        branches: Vec<(Expr, Vec<Stmt>)>,
        else_block: Option<Vec<Stmt>>,

        line: u32,
    },

    While { cond: Expr, body: Vec<Stmt>, line: u32 },

    ForNumeric {
        var: String,
        start: Expr,
        stop: Expr,
        step: Option<Expr>,
        body: Vec<Stmt>,
    },

    ForIn {
        names: Vec<String>,
        iters: Vec<Expr>,
        body: Vec<Stmt>,
    },

    Break { line: u32 },

    Return { values: Vec<Expr>, line: u32 },

    TypeAlias { name: String, ty: Type },

    Buff {
        name: String,
        size: u64,
        init: Expr,
        line: u32,
    },

    FreeBuff { name: String, line: u32 },

    Class {
        visibility: Visibility,
        is_final: bool,
        is_abstract: bool,
        name: String,
        parent: Option<String>,
        mixins: Vec<String>,
        interfaces: Vec<String>,
        members: Vec<ClassMember>,
    },

    Interface {
        visibility: Visibility,
        name: String,
        parents: Vec<String>,
        members: Vec<String>,
    },

    Enum {
        visibility: Visibility,
        name: String,
        variants: Vec<(String, Option<Expr>)>,

        line: u32,
    },

    Expr(Expr, u32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {

    Public,

    Protected,

    Private,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FnBody {
    pub params: Vec<String>,
    pub is_vararg: bool,
    pub body: Vec<Stmt>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ClassMember {

    Field {
        access: Access,
        is_static: bool,
        name: String,
        default: Option<Expr>,
    },

    Method {
        access: Access,
        is_static: bool,
        is_abstract: bool,
        is_final: bool,
        is_override: bool,
        name: String,
        func: FnBody,
    },

    Getter { access: Access, name: String, func: FnBody },

    Setter { access: Access, name: String, func: FnBody },

    Constructor { func: FnBody },

    Destructor { func: FnBody },

    Operator { symbol: String, func: FnBody },
}

#[derive(Debug, Clone, PartialEq)]
pub enum Type {

    Named(String),

    Literal(String),

    Table(Vec<(String, Type)>),

    Array(Box<Type>),

    Optional(Box<Type>),

    Function { params: Vec<Type>, ret: Box<Type> },

    Union(Vec<Type>),

    Intersection(Vec<Type>),
}

#[derive(Debug, Clone, PartialEq)]
pub enum LValue {

    Name(String),

    Index { base: Box<Expr>, key: Box<Expr> },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssignOp {

    Assign,

    Add,

    Sub,

    Mul,

    Div,

    Mod,

    Concat,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Nil,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),

    Name(String),

    Table(Vec<TableEntry>),

    Index { base: Box<Expr>, key: Box<Expr> },

    Call { callee: Box<Expr>, args: Vec<Expr> },

    Function { name: String, params: Vec<String>, is_vararg: bool, body: Vec<Stmt> },

    Vararg,

    MethodCall { receiver: Box<Expr>, method: String, args: Vec<Expr> },

    Switch { subject: Box<Expr>, cases: Vec<SwitchCase>, default: Option<Vec<Stmt>> },

    Unary { op: UnaryOp, expr: Box<Expr> },

    Binary { op: BinOp, lhs: Box<Expr>, rhs: Box<Expr> },

    Logical { op: LogicalOp, lhs: Box<Expr>, rhs: Box<Expr> },
}

#[derive(Debug, Clone, PartialEq)]
pub struct SwitchCase {
    pub pattern: Expr,
    pub body: Vec<Stmt>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TableEntry {

    Positional(Expr),

    Keyed { key: Expr, value: Expr },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {

    Neg,

    Not,

    Len,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Pow,

    Concat,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogicalOp {
    And,
    Or,
}
