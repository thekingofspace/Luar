
use crate::ast::*;
use crate::bytecode::{Chunk, Instruction as I, OpCode, Program, Value};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompileError(pub String);

impl std::fmt::Display for CompileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "compile error: {}", self.0)
    }
}

impl std::error::Error for CompileError {}

type Result<T> = std::result::Result<T, CompileError>;

pub fn compile(program: &[Stmt]) -> Result<Program> {
    let mut c = Compiler { program: Program::new() };

    c.program.protos.push(Chunk::default());

    let mut main = FuncCtx::new("main", true);
    c.compile_block(&mut main, program)?;
    main.chunk.emit(I::simple(OpCode::Halt));
    main.finalize();
    c.program.protos[0] = main.chunk;
    Ok(c.program)
}

struct Compiler {
    program: Program,
}

struct Local {
    name: String,
    depth: u32,
}

struct FuncCtx {
    chunk: Chunk,
    is_main: bool,
    locals: Vec<Local>,
    scope_depth: u32,
    max_slots: u16,

    loop_breaks: Vec<Vec<usize>>,
}

impl FuncCtx {
    fn new(name: &str, is_main: bool) -> Self {
        FuncCtx {
            chunk: Chunk::new(name),
            is_main,
            locals: Vec::new(),
            scope_depth: 0,
            max_slots: 0,
            loop_breaks: Vec::new(),
        }
    }

    fn add_local(&mut self, name: &str) -> u32 {
        let slot = self.locals.len() as u32;
        self.locals.push(Local { name: name.to_string(), depth: self.scope_depth });
        if self.locals.len() as u16 > self.max_slots {
            self.max_slots = self.locals.len() as u16;
        }
        slot
    }

    fn resolve(&self, name: &str) -> Option<u32> {
        self.locals.iter().rposition(|l| l.name == name).map(|i| i as u32)
    }

    fn begin_scope(&mut self) {
        self.scope_depth += 1;
    }

    fn end_scope(&mut self) {
        self.scope_depth -= 1;
        while self.locals.last().is_some_and(|l| l.depth > self.scope_depth) {
            self.locals.pop();
        }
    }

    fn finalize(&mut self) {
        self.chunk.num_locals = self.max_slots;
    }

    fn emit(&mut self, instr: I) -> usize {
        self.chunk.emit(instr)
    }

    fn patch_to_here(&mut self, index: usize) {
        let target = self.chunk.code.len() as u32;
        self.chunk.code[index].operand = target;
    }

    fn constant(&mut self, value: Value) -> u32 {
        self.chunk.add_constant(value)
    }
}

impl Compiler {
    fn compile_block(&mut self, ctx: &mut FuncCtx, stmts: &[Stmt]) -> Result<()> {
        for stmt in stmts {
            self.compile_stmt(ctx, stmt)?;
        }
        Ok(())
    }

    fn compile_stmt(&mut self, ctx: &mut FuncCtx, stmt: &Stmt) -> Result<()> {
        match stmt {
            Stmt::TypeAlias { .. } => {}
            Stmt::Buff { .. } | Stmt::FreeBuff { .. } => {
                return Err(CompileError(
                    "buff/freebuff are only supported by the tree-walking interpreter, not the bytecode VM".into(),
                ));
            }
            Stmt::Declare { visibility, names, inits, .. } => {
                if names.len() != 1 || inits.len() != 1 {
                    return Err(CompileError("multiple declaration not supported in VM yet".into()));
                }
                self.compile_expr(ctx, &inits[0])?;
                self.bind(ctx, &names[0], *visibility);
            }
            Stmt::Assign { targets, op, values, .. } => {
                if targets.len() != 1 || values.len() != 1 {
                    return Err(CompileError("multiple assignment not supported in VM yet".into()));
                }
                let LValue::Name(name) = &targets[0] else {
                    return Err(CompileError("table assignment not supported in VM yet".into()));
                };
                if *op == AssignOp::Assign {
                    self.compile_expr(ctx, &values[0])?;
                } else {
                    self.load_name(ctx, name);
                    self.compile_expr(ctx, &values[0])?;
                    ctx.emit(I::simple(assign_binop(*op)?));
                }
                self.store_name(ctx, name);
            }
            Stmt::Do(body) => {
                ctx.begin_scope();
                self.compile_block(ctx, body)?;
                ctx.end_scope();
            }
            Stmt::If { branches, else_block, .. } => self.compile_if(ctx, branches, else_block)?,
            Stmt::While { cond, body, .. } => self.compile_while(ctx, cond, body)?,
            Stmt::Break { .. } => {
                let jump = ctx.emit(I::new(OpCode::Jump, 0));
                ctx.loop_breaks
                    .last_mut()
                    .ok_or_else(|| CompileError("`break` outside of a loop".into()))?
                    .push(jump);
            }
            Stmt::Return { values, .. } => {
                if values.is_empty() {
                    ctx.emit(I::simple(OpCode::PushNil));
                } else {
                    self.compile_expr(ctx, &values[0])?;
                }
                ctx.emit(I::simple(OpCode::Return));
            }
            Stmt::ForNumeric { .. } | Stmt::ForIn { .. } => {
                return Err(CompileError("`for` loops not supported in VM yet".into()));
            }
            Stmt::Class { .. } => {
                return Err(CompileError("`class` not supported in VM yet".into()));
            }
            Stmt::Interface { .. } => {
                return Err(CompileError("`interface` not supported in VM yet".into()));
            }
            Stmt::Enum { .. } => {
                return Err(CompileError("`enum` not supported in VM yet".into()));
            }
            Stmt::Expr(expr, _) => {
                self.compile_expr(ctx, expr)?;
                ctx.emit(I::simple(OpCode::Pop));
            }
        }
        Ok(())
    }

    fn compile_if(
        &mut self,
        ctx: &mut FuncCtx,
        branches: &[(Expr, Vec<Stmt>)],
        else_block: &Option<Vec<Stmt>>,
    ) -> Result<()> {
        let mut end_jumps = Vec::new();
        for (cond, body) in branches {
            self.compile_expr(ctx, cond)?;
            let skip = ctx.emit(I::new(OpCode::JumpIfFalse, 0));
            ctx.begin_scope();
            self.compile_block(ctx, body)?;
            ctx.end_scope();
            end_jumps.push(ctx.emit(I::new(OpCode::Jump, 0)));
            ctx.patch_to_here(skip);
        }
        if let Some(body) = else_block {
            ctx.begin_scope();
            self.compile_block(ctx, body)?;
            ctx.end_scope();
        }
        for j in end_jumps {
            ctx.patch_to_here(j);
        }
        Ok(())
    }

    fn compile_while(&mut self, ctx: &mut FuncCtx, cond: &Expr, body: &[Stmt]) -> Result<()> {
        let loop_start = ctx.chunk.code.len() as u32;
        self.compile_expr(ctx, cond)?;
        let exit = ctx.emit(I::new(OpCode::JumpIfFalse, 0));
        ctx.loop_breaks.push(Vec::new());
        ctx.begin_scope();
        self.compile_block(ctx, body)?;
        ctx.end_scope();
        ctx.emit(I::new(OpCode::Jump, loop_start));
        ctx.patch_to_here(exit);
        for j in ctx.loop_breaks.pop().unwrap() {
            ctx.patch_to_here(j);
        }
        Ok(())
    }

    fn compile_expr(&mut self, ctx: &mut FuncCtx, expr: &Expr) -> Result<()> {
        match expr {
            Expr::Nil => {
                ctx.emit(I::simple(OpCode::PushNil));
            }
            Expr::Bool(true) => {
                ctx.emit(I::simple(OpCode::PushTrue));
            }
            Expr::Bool(false) => {
                ctx.emit(I::simple(OpCode::PushFalse));
            }
            Expr::Int(i) => {
                let k = ctx.constant(Value::Int(*i));
                ctx.emit(I::new(OpCode::PushConst, k));
            }
            Expr::Float(x) => {
                let k = ctx.constant(Value::Float(*x));
                ctx.emit(I::new(OpCode::PushConst, k));
            }
            Expr::Str(s) => {
                let k = ctx.constant(Value::Str(s.clone()));
                ctx.emit(I::new(OpCode::PushConst, k));
            }
            Expr::Name(name) => self.load_name(ctx, name),
            Expr::Function { name, params, is_vararg, body } => {
                if *is_vararg {
                    return Err(CompileError("varargs (`...`) not supported in VM yet".into()));
                }
                let proto = self.compile_function(name, params, body)?;
                ctx.emit(I::new(OpCode::MakeClosure, proto));
            }
            Expr::Unary { op, expr } => {
                self.compile_expr(ctx, expr)?;
                let opcode = match op {
                    UnaryOp::Neg => OpCode::Neg,
                    UnaryOp::Not => OpCode::Not,
                    UnaryOp::Len => return Err(CompileError("`#` not supported in VM yet".into())),
                };
                ctx.emit(I::simple(opcode));
            }
            Expr::Binary { op, lhs, rhs } => {
                self.compile_expr(ctx, lhs)?;
                self.compile_expr(ctx, rhs)?;
                ctx.emit(I::simple(binop(*op)?));
            }
            Expr::Logical { op, lhs, rhs } => self.compile_logical(ctx, *op, lhs, rhs)?,
            Expr::Call { callee, args } => self.compile_call(ctx, callee, args)?,
            Expr::Index { .. } => {
                return Err(CompileError("tables/indexing not supported in VM yet".into()));
            }
            Expr::Table(_) => {
                return Err(CompileError("tables not supported in VM yet".into()));
            }
            Expr::Switch { .. } => {
                return Err(CompileError("`switch` not supported in VM yet".into()));
            }
            Expr::MethodCall { .. } => {
                return Err(CompileError("method calls not supported in VM yet".into()));
            }
            Expr::Vararg => {
                return Err(CompileError("varargs (`...`) not supported in VM yet".into()));
            }
        }
        Ok(())
    }

    fn compile_logical(&mut self, ctx: &mut FuncCtx, op: LogicalOp, lhs: &Expr, rhs: &Expr) -> Result<()> {
        self.compile_expr(ctx, lhs)?;
        ctx.emit(I::simple(OpCode::Dup));
        let short = match op {

            LogicalOp::And => ctx.emit(I::new(OpCode::JumpIfFalse, 0)),

            LogicalOp::Or => {
                ctx.emit(I::simple(OpCode::Not));
                ctx.emit(I::new(OpCode::JumpIfFalse, 0))
            }
        };
        ctx.emit(I::simple(OpCode::Pop));
        self.compile_expr(ctx, rhs)?;
        ctx.patch_to_here(short);
        Ok(())
    }

    fn compile_call(&mut self, ctx: &mut FuncCtx, callee: &Expr, args: &[Expr]) -> Result<()> {

        if let Expr::Name(name) = callee {
            if name == "print" {
                for a in args {
                    self.compile_expr(ctx, a)?;
                }
                ctx.emit(I::new(OpCode::Print, args.len() as u32));
                ctx.emit(I::simple(OpCode::PushNil));
                return Ok(());
            }
        }

        if let Expr::Index { base, key } = callee {
            if let (Expr::Name(ns), Expr::Str(method)) = (base.as_ref(), key.as_ref()) {
                if ns == "coroutine" {
                    return self.compile_coroutine(ctx, method, args);
                }
            }
        }

        self.compile_expr(ctx, callee)?;
        for a in args {
            self.compile_expr(ctx, a)?;
        }
        ctx.emit(I::new(OpCode::Call, args.len() as u32));
        Ok(())
    }

    fn compile_coroutine(&mut self, ctx: &mut FuncCtx, method: &str, args: &[Expr]) -> Result<()> {
        match method {
            "create" => {
                self.expect_args("coroutine.create", args, 1)?;
                self.compile_expr(ctx, &args[0])?;
                ctx.emit(I::simple(OpCode::NewCoroutine));
            }
            "resume" => {
                if args.is_empty() {
                    return Err(CompileError("coroutine.resume expects a coroutine".into()));
                }
                self.compile_expr(ctx, &args[0])?;
                for a in &args[1..] {
                    self.compile_expr(ctx, a)?;
                }
                ctx.emit(I::new(OpCode::Resume, (args.len() - 1) as u32));
            }
            "yield" => {
                if args.is_empty() {
                    ctx.emit(I::simple(OpCode::PushNil));
                } else {
                    self.compile_expr(ctx, &args[0])?;
                }
                ctx.emit(I::simple(OpCode::Yield));
            }
            "status" => {
                self.expect_args("coroutine.status", args, 1)?;
                self.compile_expr(ctx, &args[0])?;
                ctx.emit(I::simple(OpCode::CoStatus));
            }
            "close" => {
                self.expect_args("coroutine.close", args, 1)?;
                self.compile_expr(ctx, &args[0])?;
                ctx.emit(I::simple(OpCode::CoClose));
            }
            "running" => {
                self.expect_args("coroutine.running", args, 0)?;
                ctx.emit(I::simple(OpCode::CoRunning));
            }
            other => return Err(CompileError(format!("unknown coroutine method '{other}'"))),
        }
        Ok(())
    }

    fn expect_args(&self, who: &str, args: &[Expr], n: usize) -> Result<()> {
        if args.len() != n {
            return Err(CompileError(format!("{who} expects {n} argument(s), got {}", args.len())));
        }
        Ok(())
    }

    fn compile_function(&mut self, name: &str, params: &[String], body: &[Stmt]) -> Result<u32> {
        let fname = if name.is_empty() { "anonymous" } else { name };
        let mut ctx = FuncCtx::new(fname, false);
        for p in params {
            ctx.add_local(p);
        }
        self.compile_block(&mut ctx, body)?;

        ctx.emit(I::simple(OpCode::PushNil));
        ctx.emit(I::simple(OpCode::Return));
        ctx.finalize();
        self.program.protos.push(ctx.chunk);
        Ok((self.program.protos.len() - 1) as u32)
    }

    fn bind(&mut self, ctx: &mut FuncCtx, name: &str, visibility: Visibility) {
        let global = ctx.is_main || visibility == Visibility::Pub;
        if global {
            let k = ctx.constant(Value::Str(name.to_string()));
            ctx.emit(I::new(OpCode::SetGlobal, k));
        } else {
            let slot = ctx.add_local(name);
            ctx.emit(I::new(OpCode::StoreLocal, slot));
        }
    }

    fn load_name(&mut self, ctx: &mut FuncCtx, name: &str) {
        if let Some(slot) = ctx.resolve(name) {
            ctx.emit(I::new(OpCode::LoadLocal, slot));
        } else {
            let k = ctx.constant(Value::Str(name.to_string()));
            ctx.emit(I::new(OpCode::GetGlobal, k));
        }
    }

    fn store_name(&mut self, ctx: &mut FuncCtx, name: &str) {
        if let Some(slot) = ctx.resolve(name) {
            ctx.emit(I::new(OpCode::StoreLocal, slot));
        } else {
            let k = ctx.constant(Value::Str(name.to_string()));
            ctx.emit(I::new(OpCode::SetGlobal, k));
        }
    }
}

fn binop(op: BinOp) -> Result<OpCode> {
    Ok(match op {
        BinOp::Add => OpCode::Add,
        BinOp::Sub => OpCode::Sub,
        BinOp::Mul => OpCode::Mul,
        BinOp::Div => OpCode::Div,
        BinOp::Mod => OpCode::Mod,
        BinOp::Eq => OpCode::Eq,
        BinOp::Ne => OpCode::Ne,
        BinOp::Lt => OpCode::Lt,
        BinOp::Le => OpCode::Le,
        BinOp::Gt => OpCode::Gt,
        BinOp::Ge => OpCode::Ge,
        BinOp::Pow => return Err(CompileError("`^` not supported in VM yet".into())),
        BinOp::Concat => return Err(CompileError("`..` not supported in VM yet".into())),
    })
}

fn assign_binop(op: AssignOp) -> Result<OpCode> {
    Ok(match op {
        AssignOp::Add => OpCode::Add,
        AssignOp::Sub => OpCode::Sub,
        AssignOp::Mul => OpCode::Mul,
        AssignOp::Div => OpCode::Div,
        AssignOp::Mod => OpCode::Mod,
        AssignOp::Concat => return Err(CompileError("`..=` not supported in VM yet".into())),
        AssignOp::Assign => unreachable!("plain assignment has no binary op"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::tokenize;
    use crate::parser::parse;
    use crate::vm::Vm;

    fn run(src: &str) -> Vm {
        let program = compile(&parse(tokenize(src).unwrap()).unwrap()).unwrap();
        let mut vm = Vm::new(program).unwrap();
        vm.run().unwrap();
        vm
    }

    #[test]
    fn compiles_arithmetic_and_globals() {
        let vm = run("pub local x = (1 + 2) * 3\nprint(x)");
        assert_eq!(vm.output, vec!["9"]);
        assert_eq!(vm.global("x"), Some(Value::Int(9)));
    }

    #[test]
    fn compiles_if_while_and_break() {
        let vm = run(
            r#"local n = 0
local total = 0
while true do
  n = n + 1
  if n > 5 then break end
  total = total + n
end
print(total)"#,
        );
        assert_eq!(vm.output, vec!["15"]);
    }

    #[test]
    fn compiles_function_call_and_recursion() {
        let vm = run(
            r#"local function fact(n)
  if n <= 1 then return 1 end
  return n * fact(n - 1)
end
print(fact(5))"#,
        );
        assert_eq!(vm.output, vec!["120"]);
    }

    #[test]
    fn runs_a_coroutine_from_source() {
        let vm = run(
            r#"local function gen(start)
  local i = start
  coroutine.yield(i)
  coroutine.yield(i + 1)
  return i + 2
end
local co = coroutine.create(gen)
print(coroutine.resume(co, 10))
print(coroutine.resume(co))
print(coroutine.resume(co))
print(coroutine.status(co))"#,
        );
        assert_eq!(vm.output, vec!["10", "11", "12", "dead"]);
    }

    #[test]
    fn coroutine_shares_outer_globals() {

        let vm = run(
            r#"pub local counter = 0
local function bump()
  counter = counter + 1
  coroutine.yield(counter)
  counter = counter + 1
  return counter
end
local co = coroutine.create(bump)
print(coroutine.resume(co))
print(coroutine.resume(co))
print(counter)"#,
        );
        assert_eq!(vm.output, vec!["1", "2", "2"]);
    }
}
