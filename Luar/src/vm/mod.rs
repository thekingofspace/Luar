
use std::collections::HashMap;

use crate::bytecode::{Instruction, OpCode, Program, Value};

#[derive(Debug)]
struct Frame {

    proto: usize,

    ip: usize,

    base: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FiberStatus {

    Suspended,

    Running,

    Normal,

    Dead,
}

#[derive(Debug)]
struct Fiber {
    stack: Vec<Value>,
    frames: Vec<Frame>,
    status: FiberStatus,

    started: bool,

    proto: usize,
}

impl Fiber {

    fn main(main_proto: &crate::bytecode::Chunk) -> Fiber {
        let mut stack = Vec::new();
        stack.resize(main_proto.num_locals as usize, Value::Nil);
        Fiber {
            stack,
            frames: vec![Frame { proto: 0, ip: 0, base: 0 }],
            status: FiberStatus::Running,
            started: true,
            proto: 0,
        }
    }

    fn coroutine(proto: usize) -> Fiber {
        Fiber {
            stack: Vec::new(),
            frames: Vec::new(),
            status: FiberStatus::Suspended,
            started: false,
            proto,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeError(pub String);

impl std::fmt::Display for RuntimeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "runtime error: {}", self.0)
    }
}

impl std::error::Error for RuntimeError {}

type Result<T> = std::result::Result<T, RuntimeError>;

pub struct Vm {
    program: Program,

    fibers: Vec<Fiber>,

    current: usize,

    resume_chain: Vec<usize>,

    globals: HashMap<String, Value>,

    pub output: Vec<String>,
}

impl Vm {

    pub fn new(program: Program) -> Result<Vm> {
        let main_proto = program
            .main()
            .ok_or_else(|| RuntimeError("program has no entry-point chunk".into()))?;
        let main = Fiber::main(main_proto);
        Ok(Vm {
            program,
            fibers: vec![main],
            current: 0,
            resume_chain: Vec::new(),
            globals: HashMap::new(),
            output: Vec::new(),
        })
    }

    pub fn set_global(&mut self, name: impl Into<String>, value: Value) {
        self.globals.insert(name.into(), value);
    }

    pub fn global(&self, name: &str) -> Option<Value> {
        self.globals.get(name).cloned()
    }

    pub fn run(&mut self) -> Result<Option<Value>> {
        loop {
            if let Some(result) = self.step()? {
                return Ok(result);
            }
        }
    }

    fn step(&mut self) -> Result<Option<Option<Value>>> {
        let instr = self.fetch()?;
        match instr.op {
            OpCode::Nop => {}
            OpCode::PushConst => {
                let v = self.constant(instr.operand)?;
                self.push(v);
            }
            OpCode::PushNil => self.push(Value::Nil),
            OpCode::PushTrue => self.push(Value::Bool(true)),
            OpCode::PushFalse => self.push(Value::Bool(false)),
            OpCode::Pop => {
                self.pop()?;
            }
            OpCode::Dup => {
                let v = self.peek()?.clone();
                self.push(v);
            }
            OpCode::LoadLocal => {
                let v = self.load_local(instr.operand)?;
                self.push(v);
            }
            OpCode::StoreLocal => {
                let v = self.pop()?;
                self.store_local(instr.operand, v)?;
            }
            OpCode::Add | OpCode::Sub | OpCode::Mul | OpCode::Div | OpCode::Mod => {
                self.binary_arith(instr.op)?;
            }
            OpCode::Neg => {
                let v = self.pop()?;
                let r = match v {
                    Value::Int(i) => Value::Int(-i),
                    Value::Float(x) => Value::Float(-x),
                    other => return Err(self.type_err("negate", &other)),
                };
                self.push(r);
            }
            OpCode::Eq => {
                let (a, b) = self.pop2()?;
                self.push(Value::Bool(values_equal(&a, &b)));
            }
            OpCode::Ne => {
                let (a, b) = self.pop2()?;
                self.push(Value::Bool(!values_equal(&a, &b)));
            }
            OpCode::Lt | OpCode::Le | OpCode::Gt | OpCode::Ge => {
                self.binary_compare(instr.op)?;
            }
            OpCode::Not => {
                let v = self.pop()?;
                self.push(Value::Bool(!v.is_truthy()));
            }
            OpCode::Jump => {
                self.frame_mut().ip = instr.operand as usize;
            }
            OpCode::JumpIfFalse => {
                let cond = self.pop()?;
                if !cond.is_truthy() {
                    self.frame_mut().ip = instr.operand as usize;
                }
            }
            OpCode::GetGlobal => {
                let name = self.global_name(instr.operand)?;
                let v = self.globals.get(&name).cloned().unwrap_or(Value::Nil);
                self.push(v);
            }
            OpCode::SetGlobal => {
                let name = self.global_name(instr.operand)?;
                let v = self.pop()?;
                self.globals.insert(name, v);
            }
            OpCode::MakeClosure => {
                self.push(Value::Function(instr.operand));
            }
            OpCode::Call => {
                self.do_call(instr.operand as usize)?;
            }
            OpCode::Return => {
                if let Some(result) = self.do_return()? {
                    return Ok(Some(result));
                }
            }
            OpCode::NewCoroutine => {
                let v = self.pop()?;
                let proto = match v {
                    Value::Function(p) => p as usize,
                    other => return Err(self.type_err("make a coroutine from", &other)),
                };
                if self.program.protos.get(proto).is_none() {
                    return Err(RuntimeError(format!("coroutine over unknown proto {proto}")));
                }
                self.fibers.push(Fiber::coroutine(proto));
                let handle = self.fibers.len() - 1;
                self.push(Value::Coroutine(handle));
            }
            OpCode::Resume => {
                self.do_resume(instr.operand as usize)?;
            }
            OpCode::Yield => {
                self.do_yield()?;
            }
            OpCode::CoStatus => {
                let id = self.pop_coroutine("status")?;
                let s = match self.fibers[id].status {
                    FiberStatus::Suspended => "suspended",
                    FiberStatus::Running => "running",
                    FiberStatus::Normal => "normal",
                    FiberStatus::Dead => "dead",
                };
                self.push(Value::Str(s.to_string()));
            }
            OpCode::CoClose => {
                let id = self.pop_coroutine("close")?;
                let ok = match self.fibers[id].status {
                    FiberStatus::Suspended | FiberStatus::Dead => {
                        let fiber = &mut self.fibers[id];
                        fiber.status = FiberStatus::Dead;
                        fiber.stack.clear();
                        fiber.frames.clear();
                        true
                    }

                    _ => false,
                };
                self.push(Value::Bool(ok));
            }
            OpCode::CoRunning => {
                self.push(Value::Coroutine(self.current));
            }
            OpCode::Print => {

                let n = instr.operand as usize;
                let mut vals = Vec::with_capacity(n);
                for _ in 0..n {
                    vals.push(self.pop()?);
                }
                vals.reverse();
                let line = vals.iter().map(|v| v.to_string()).collect::<Vec<_>>().join("\t");
                println!("{line}");
                self.output.push(line);
            }
            OpCode::Halt => {

                let top = self.fibers[self.current].stack.last().cloned();
                return Ok(Some(top));
            }
        }
        Ok(None)
    }

    fn fetch(&mut self) -> Result<Instruction> {
        let frame = self
            .fibers[self.current]
            .frames
            .last()
            .ok_or_else(|| RuntimeError("no active frame".into()))?;
        let chunk = &self.program.protos[frame.proto];
        let instr = *chunk
            .code
            .get(frame.ip)
            .ok_or_else(|| RuntimeError("instruction pointer out of bounds".into()))?;
        self.fibers[self.current].frames.last_mut().unwrap().ip += 1;
        Ok(instr)
    }

    fn frame_mut(&mut self) -> &mut Frame {
        self.fibers[self.current].frames.last_mut().expect("active frame")
    }

    fn constant(&self, index: u32) -> Result<Value> {
        let frame = self.fibers[self.current].frames.last().unwrap();
        self.program.protos[frame.proto]
            .constants
            .get(index as usize)
            .cloned()
            .ok_or_else(|| RuntimeError(format!("constant index {index} out of range")))
    }

    fn push(&mut self, v: Value) {
        self.fibers[self.current].stack.push(v);
    }

    fn pop(&mut self) -> Result<Value> {
        self.fibers[self.current]
            .stack
            .pop()
            .ok_or_else(|| RuntimeError("stack underflow".into()))
    }

    fn pop2(&mut self) -> Result<(Value, Value)> {
        let b = self.pop()?;
        let a = self.pop()?;
        Ok((a, b))
    }

    fn peek(&self) -> Result<&Value> {
        self.fibers[self.current]
            .stack
            .last()
            .ok_or_else(|| RuntimeError("stack underflow".into()))
    }

    fn load_local(&self, slot: u32) -> Result<Value> {
        let fiber = &self.fibers[self.current];
        let base = fiber.frames.last().unwrap().base;
        fiber
            .stack
            .get(base + slot as usize)
            .cloned()
            .ok_or_else(|| RuntimeError(format!("local slot {slot} out of range")))
    }

    fn store_local(&mut self, slot: u32, value: Value) -> Result<()> {
        let fiber = &mut self.fibers[self.current];
        let base = fiber.frames.last().unwrap().base;
        let cell = fiber
            .stack
            .get_mut(base + slot as usize)
            .ok_or_else(|| RuntimeError(format!("local slot {slot} out of range")))?;
        *cell = value;
        Ok(())
    }

    fn binary_arith(&mut self, op: OpCode) -> Result<()> {
        let (a, b) = self.pop2()?;
        let r = match (&a, &b) {
            (Value::Int(x), Value::Int(y)) => match op {
                OpCode::Add => Value::Int(x.wrapping_add(*y)),
                OpCode::Sub => Value::Int(x.wrapping_sub(*y)),
                OpCode::Mul => Value::Int(x.wrapping_mul(*y)),
                OpCode::Div => {
                    if *y == 0 {
                        return Err(RuntimeError("integer division by zero".into()));
                    }
                    Value::Int(x / y)
                }
                OpCode::Mod => {
                    if *y == 0 {
                        return Err(RuntimeError("integer modulo by zero".into()));
                    }
                    Value::Int(x % y)
                }
                _ => unreachable!(),
            },
            _ => {
                let (x, y) = (as_float(&a), as_float(&b));
                match (x, y) {
                    (Some(x), Some(y)) => {
                        let v = match op {
                            OpCode::Add => x + y,
                            OpCode::Sub => x - y,
                            OpCode::Mul => x * y,
                            OpCode::Div => x / y,
                            OpCode::Mod => x % y,
                            _ => unreachable!(),
                        };
                        Value::Float(v)
                    }
                    _ => {
                        return Err(RuntimeError(format!(
                            "cannot apply arithmetic to {} and {}",
                            a.type_name(),
                            b.type_name()
                        )));
                    }
                }
            }
        };
        self.push(r);
        Ok(())
    }

    fn binary_compare(&mut self, op: OpCode) -> Result<()> {
        let (a, b) = self.pop2()?;
        let ordering = compare(&a, &b)
            .ok_or_else(|| RuntimeError(format!(
                "cannot compare {} and {}",
                a.type_name(),
                b.type_name()
            )))?;
        use std::cmp::Ordering::*;
        let result = match op {
            OpCode::Lt => ordering == Less,
            OpCode::Le => ordering != Greater,
            OpCode::Gt => ordering == Greater,
            OpCode::Ge => ordering != Less,
            _ => unreachable!(),
        };
        self.push(Value::Bool(result));
        Ok(())
    }

    fn type_err(&self, what: &str, v: &Value) -> RuntimeError {
        RuntimeError(format!("cannot {what} a {}", v.type_name()))
    }

    fn global_name(&self, index: u32) -> Result<String> {
        match self.constant(index)? {
            Value::Str(s) => Ok(s),
            other => Err(RuntimeError(format!("global name must be a string, got {}", other.type_name()))),
        }
    }

    fn pop_coroutine(&mut self, what: &str) -> Result<usize> {
        match self.pop()? {
            Value::Coroutine(id) => Ok(id),
            other => Err(RuntimeError(format!("cannot {what} a {}", other.type_name()))),
        }
    }

    fn do_call(&mut self, argc: usize) -> Result<()> {
        let stack_len = self.fibers[self.current].stack.len();
        if stack_len < argc + 1 {
            return Err(RuntimeError("call: stack underflow".into()));
        }
        let callee_idx = stack_len - argc - 1;
        let proto = match &self.fibers[self.current].stack[callee_idx] {
            Value::Function(p) => *p as usize,
            other => return Err(self.type_err("call", &other.clone())),
        };
        let need = self
            .program
            .protos
            .get(proto)
            .ok_or_else(|| RuntimeError(format!("call to unknown proto {proto}")))?
            .num_locals as usize;

        let base = callee_idx + 1;
        let fiber = &mut self.fibers[self.current];

        if argc < need {
            for _ in argc..need {
                fiber.stack.push(Value::Nil);
            }
        } else if argc > need {
            fiber.stack.truncate(base + need);
        }
        fiber.frames.push(Frame { proto, ip: 0, base });
        Ok(())
    }

    fn do_return(&mut self) -> Result<Option<Option<Value>>> {
        let ret = self.pop().unwrap_or(Value::Nil);
        let frame = self.fibers[self.current].frames.pop().expect("active frame");

        if !self.fibers[self.current].frames.is_empty() {

            let fiber = &mut self.fibers[self.current];
            fiber.stack.truncate(frame.base - 1);
            fiber.stack.push(ret);
            return Ok(None);
        }

        self.fibers[self.current].stack.clear();
        if self.current == 0 {
            return Ok(Some(Some(ret)));
        }

        self.fibers[self.current].status = FiberStatus::Dead;
        self.return_to_resumer(ret);
        Ok(None)
    }

    fn do_resume(&mut self, argc: usize) -> Result<()> {
        let mut args = Vec::with_capacity(argc);
        for _ in 0..argc {
            args.push(self.pop()?);
        }
        args.reverse();
        let target = self.pop_coroutine("resume")?;

        match self.fibers[target].status {
            FiberStatus::Suspended => {}
            FiberStatus::Dead => return Err(RuntimeError("cannot resume a dead coroutine".into())),
            _ => return Err(RuntimeError("cannot resume a non-suspended coroutine".into())),
        }

        self.fibers[self.current].status = FiberStatus::Normal;
        self.resume_chain.push(self.current);
        self.fibers[target].status = FiberStatus::Running;
        self.current = target;

        if !self.fibers[target].started {

            self.fibers[target].started = true;
            let proto = self.fibers[target].proto;
            let need = self.program.protos[proto].num_locals as usize;
            let fiber = &mut self.fibers[target];
            fiber.stack.clear();
            for a in args.into_iter().take(need) {
                fiber.stack.push(a);
            }
            while fiber.stack.len() < need {
                fiber.stack.push(Value::Nil);
            }
            fiber.frames.push(Frame { proto, ip: 0, base: 0 });
        } else {

            let resume_val = args.into_iter().next().unwrap_or(Value::Nil);
            self.fibers[target].stack.push(resume_val);
        }
        Ok(())
    }

    fn do_yield(&mut self) -> Result<()> {
        if self.current == 0 {
            return Err(RuntimeError("attempt to yield from outside a coroutine".into()));
        }
        let value = self.pop()?;
        self.fibers[self.current].status = FiberStatus::Suspended;
        self.return_to_resumer(value);
        Ok(())
    }

    fn return_to_resumer(&mut self, value: Value) {
        let resumer = self
            .resume_chain
            .pop()
            .expect("a resumed fiber always has a resumer");
        self.fibers[resumer].status = FiberStatus::Running;
        self.fibers[resumer].stack.push(value);
        self.current = resumer;
    }
}

fn as_float(v: &Value) -> Option<f64> {
    match v {
        Value::Int(i) => Some(*i as f64),
        Value::Float(x) => Some(*x),
        _ => None,
    }
}

fn values_equal(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Int(x), Value::Float(y)) => (*x as f64) == *y,
        (Value::Float(x), Value::Int(y)) => *x == (*y as f64),
        _ => a == b,
    }
}

fn compare(a: &Value, b: &Value) -> Option<std::cmp::Ordering> {
    match (a, b) {
        (Value::Int(x), Value::Int(y)) => Some(x.cmp(y)),
        (Value::Str(x), Value::Str(y)) => Some(x.cmp(y)),
        _ => {
            let (x, y) = (as_float(a)?, as_float(b)?);
            x.partial_cmp(&y)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bytecode::{Chunk, Instruction as I, OpCode::*};

    #[test]
    fn arithmetic_and_return() {

        let mut main = Chunk::new("main");
        let two = main.add_constant(Value::Int(2));
        let three = main.add_constant(Value::Int(3));
        let four = main.add_constant(Value::Int(4));
        main.emit(I::new(PushConst, three));
        main.emit(I::new(PushConst, four));
        main.emit(I::simple(Mul));
        main.emit(I::new(PushConst, two));

        main.emit(I::simple(Add));
        main.emit(I::simple(Return));

        let mut prog = Program::new();
        prog.add_proto(main);
        let mut vm = Vm::new(prog).unwrap();
        assert_eq!(vm.run().unwrap(), Some(Value::Int(14)));
    }

    #[test]
    fn coroutine_yields_then_returns() {

        let mut co = Chunk::new("co");
        let c10 = co.add_constant(Value::Int(10));
        let c20 = co.add_constant(Value::Int(20));
        let c30 = co.add_constant(Value::Int(30));
        co.emit(I::new(PushConst, c10));
        co.emit(I::simple(Yield));
        co.emit(I::simple(Pop));
        co.emit(I::new(PushConst, c20));
        co.emit(I::simple(Yield));
        co.emit(I::simple(Pop));
        co.emit(I::new(PushConst, c30));
        co.emit(I::simple(Return));

        let mut main = Chunk::new("main");
        main.num_locals = 1;
        main.emit(I::new(MakeClosure, 1));
        main.emit(I::simple(NewCoroutine));
        main.emit(I::new(StoreLocal, 0));
        for _ in 0..3 {
            main.emit(I::new(LoadLocal, 0));
            main.emit(I::new(Resume, 0));
            main.emit(I::new(Print, 1));
        }
        main.emit(I::simple(Halt));

        let mut prog = Program::new();
        prog.add_proto(main);
        prog.add_proto(co);
        let mut vm = Vm::new(prog).unwrap();
        vm.run().unwrap();
        assert_eq!(vm.output, vec!["10", "20", "30"]);
    }

    #[test]
    fn function_call_returns_value() {

        let mut f = Chunk::new("f");
        let c = f.add_constant(Value::Int(99));
        f.emit(I::new(PushConst, c));
        f.emit(I::simple(Return));

        let mut main = Chunk::new("main");
        main.emit(I::new(MakeClosure, 1));
        main.emit(I::new(Call, 0));
        main.emit(I::new(Print, 1));
        main.emit(I::simple(Halt));

        let mut prog = Program::new();
        prog.add_proto(main);
        prog.add_proto(f);
        let mut vm = Vm::new(prog).unwrap();
        vm.run().unwrap();
        assert_eq!(vm.output, vec!["99"]);
    }

    #[test]
    fn coroutine_with_args_and_status() {

        let mut co = Chunk::new("co");
        co.num_locals = 1;
        let one = co.add_constant(Value::Int(1));
        let two = co.add_constant(Value::Int(2));
        co.emit(I::new(LoadLocal, 0));
        co.emit(I::simple(Yield));
        co.emit(I::simple(Pop));
        co.emit(I::new(LoadLocal, 0));
        co.emit(I::new(PushConst, one));
        co.emit(I::simple(Add));
        co.emit(I::simple(Yield));
        co.emit(I::simple(Pop));
        co.emit(I::new(LoadLocal, 0));
        co.emit(I::new(PushConst, two));
        co.emit(I::simple(Add));
        co.emit(I::simple(Return));

        let mut main = Chunk::new("main");
        main.num_locals = 1;
        let ten = main.add_constant(Value::Int(10));
        main.emit(I::new(MakeClosure, 1));
        main.emit(I::simple(NewCoroutine));
        main.emit(I::new(StoreLocal, 0));

        main.emit(I::new(LoadLocal, 0));
        main.emit(I::new(PushConst, ten));
        main.emit(I::new(Resume, 1));
        main.emit(I::new(Print, 1));
        main.emit(I::new(LoadLocal, 0));
        main.emit(I::new(Resume, 0));
        main.emit(I::new(Print, 1));
        main.emit(I::new(LoadLocal, 0));
        main.emit(I::new(Resume, 0));
        main.emit(I::new(Print, 1));

        main.emit(I::new(LoadLocal, 0));
        main.emit(I::simple(CoStatus));
        main.emit(I::new(Print, 1));
        main.emit(I::simple(Halt));

        let mut prog = Program::new();
        prog.add_proto(main);
        prog.add_proto(co);
        let mut vm = Vm::new(prog).unwrap();
        vm.run().unwrap();
        assert_eq!(vm.output, vec!["10", "11", "12", "dead"]);
    }
}
