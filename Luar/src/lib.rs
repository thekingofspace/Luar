
pub mod ast;
pub mod bytecode;
pub mod compiler;
pub mod ferrite;
pub mod lexer;
pub mod parser;
pub mod precompile;
pub mod runtime;
pub mod vm;

pub use ast::{AssignOp, BinOp, Expr, LValue, Stmt, TableEntry, Type, UnaryOp};
pub use compiler::{compile, CompileError};
pub use lexer::{LexError, Lexer, Span, Token, TokenKind};
pub use parser::{parse, ParseError};
pub use runtime::{
    blocking, pump_ready, run_pending, Context, Environment, EvalError, Interpreter, Mutability,
    NativeClassBuilder, Table, Value, VarError, Variable, Visibility,
};
pub use runtime::do_yield as yield_values;
pub use runtime::running as current_coroutine;
pub use runtime::gc::script_source;
pub use vm::{RuntimeError, Vm};

pub use bytecode::{Chunk, CodecError, Instruction, OpCode, Program};

#[derive(Debug)]
pub enum Error {
    Lex(LexError),
    Parse(ParseError),
    Eval(EvalError),
    Compile(CompileError),
    Runtime(RuntimeError),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Lex(e) => write!(f, "{e}"),
            Error::Parse(e) => write!(f, "{e}"),
            Error::Eval(e) => write!(f, "{e}"),
            Error::Compile(e) => write!(f, "{e}"),
            Error::Runtime(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<LexError> for Error {
    fn from(e: LexError) -> Self {
        Error::Lex(e)
    }
}
impl From<ParseError> for Error {
    fn from(e: ParseError) -> Self {
        Error::Parse(e)
    }
}
impl From<EvalError> for Error {
    fn from(e: EvalError) -> Self {
        Error::Eval(e)
    }
}
impl From<CompileError> for Error {
    fn from(e: CompileError) -> Self {
        Error::Compile(e)
    }
}
impl From<RuntimeError> for Error {
    fn from(e: RuntimeError) -> Self {
        Error::Runtime(e)
    }
}

pub fn tokenize(source: &str) -> Result<Vec<Token>, LexError> {
    lexer::tokenize(source)
}

pub fn parse_source(source: &str) -> Result<Vec<Stmt>, Error> {
    Ok(parser::parse(lexer::tokenize(source)?)?)
}

pub fn eval_source(source: &str) -> Result<Interpreter, Error> {
    let program = parse_source(source)?;
    let mut interp = Interpreter::new();
    interp.run(&program)?;
    Ok(interp)
}

pub fn execute(program: Program) -> Result<Option<bytecode::Value>, RuntimeError> {
    Vm::new(program)?.run()
}

pub fn precompile_source(source: &str) -> Result<Vec<u8>, Error> {
    let program = parse_source(source)?;
    Ok(precompile::pack(&program))
}

pub fn run_precompiled(bytes: &[u8]) -> Result<Interpreter, Error> {
    let (interp, _) = run_precompiled_returns(bytes)?;
    Ok(interp)
}

pub fn run_precompiled_returns(bytes: &[u8]) -> Result<(Interpreter, Vec<Value>), Error> {
    let program =
        precompile::unpack(bytes).map_err(|e| Error::Compile(CompileError(e)))?;
    let mut interp = Interpreter::new();
    let returned = interp.run(&program)?;
    Ok((interp, returned))
}

pub fn load_precompiled_module(bytes: &[u8]) -> Result<Value, Error> {
    let (_, returned) = run_precompiled_returns(bytes)?;
    Ok(returned.into_iter().next().unwrap_or(Value::Nil))
}

pub fn compile_source(source: &str) -> Result<Program, Error> {
    Ok(compiler::compile(&parse_source(source)?)?)
}

pub fn run_on_vm(source: &str) -> Result<Vm, Error> {
    let program = compile_source(source)?;
    let mut vm = Vm::new(program)?;
    vm.run()?;
    Ok(vm)
}

impl Interpreter {

    pub fn run_source(&mut self, source: &str) -> Result<Vec<Value>, Error> {
        let program = parse_source(source)?;
        Ok(self.run(&program)?)
    }

    pub fn run_precompiled(&mut self, bytes: &[u8]) -> Result<Vec<Value>, Error> {
        let program =
            precompile::unpack(bytes).map_err(|e| Error::Compile(CompileError(e)))?;
        Ok(self.run(&program)?)
    }

    pub fn load_module(source: &str) -> Result<Value, Error> {
        let mut module = Interpreter::new();
        let returned = module.run_source(source)?;
        Ok(returned.into_iter().next().unwrap_or(Value::Nil))
    }

    pub fn get_global(&self, name: &str) -> Option<Value> {
        let _fam = runtime::gil::FamilyScope::enter(&self.family);
        self.env.get(name)
    }

    pub fn set_global(&mut self, name: &str, value: Value) {
        let _fam = runtime::gil::FamilyScope::enter(&self.family);
        self.env.declare(name.to_string(), value, Mutability::Mutable, Visibility::Pub);
    }

    pub fn set_global_const(&mut self, name: &str, value: Value) {
        let _fam = runtime::gil::FamilyScope::enter(&self.family);
        self.env.declare(name.to_string(), value, Mutability::Const, Visibility::Pub);
    }

    pub fn force_set(&mut self, name: &str, value: Value) -> bool {
        let _fam = runtime::gil::FamilyScope::enter(&self.family);
        self.env.force_set(name, value)
    }

    pub fn nil_value(&mut self, name: &str) -> bool {
        let _fam = runtime::gil::FamilyScope::enter(&self.family);
        self.env.force_set(name, Value::nil())
    }

    pub fn remove_global(&mut self, name: &str) -> bool {
        let _fam = runtime::gil::FamilyScope::enter(&self.family);
        self.env.remove(name).is_some()
    }

    pub fn create_table(&self) -> Value {
        Value::table()
    }

    pub fn set_global_fn(&mut self, name: &'static str, func: runtime::NativeFn) {
        self.set_global_const(name, Value::native(name, func));
    }

    pub fn define_class(&mut self, builder: NativeClassBuilder) -> Value {
        let class = builder.build();
        if let Value::Class(c) = &class {
            self.set_global_const(&c.name, class.clone());
        }
        class
    }

    pub fn call_value(&mut self, callee: &Value, args: Vec<Value>) -> Result<Vec<Value>, EvalError> {
        let _fam = runtime::gil::FamilyScope::enter(&self.family);
        self.call(callee, args)
    }

    pub fn launch(&mut self, callee: &Value, args: Vec<Value>) -> Result<Value, EvalError> {
        let _fam = runtime::gil::FamilyScope::enter(&self.family);
        runtime::launch_callable(self, callee.clone(), args).map_err(EvalError)
    }

    pub fn launch_method(
        &mut self,
        receiver: &Value,
        method: &str,
        args: Vec<Value>,
    ) -> Result<Value, EvalError> {
        let _fam = runtime::gil::FamilyScope::enter(&self.family);
        runtime::launch_method_value(self, receiver.clone(), method, args).map_err(EvalError)
    }

    pub fn resume_coroutine(&mut self, co: &Value, args: Vec<Value>) -> Result<Vec<Value>, EvalError> {
        let _fam = runtime::gil::FamilyScope::enter(&self.family);
        match co {
            Value::Coroutine(rc) => Ok(runtime::coroutine_resume(rc, args)),
            other => Err(EvalError(format!(
                "resume_coroutine: expected a thread, got {}",
                other.type_name()
            ))),
        }
    }
}
