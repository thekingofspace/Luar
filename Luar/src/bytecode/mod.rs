
use std::fmt;

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Nil,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),

    Function(u32),

    Coroutine(usize),
}

impl Value {

    pub fn is_truthy(&self) -> bool {
        !matches!(self, Value::Nil | Value::Bool(false))
    }

    pub fn type_name(&self) -> &'static str {
        match self {
            Value::Nil => "nil",
            Value::Bool(_) => "bool",
            Value::Int(_) => "int",
            Value::Float(_) => "float",
            Value::Str(_) => "string",
            Value::Function(_) => "function",
            Value::Coroutine(_) => "coroutine",
        }
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Nil => write!(f, "nil"),
            Value::Bool(b) => write!(f, "{b}"),
            Value::Int(i) => write!(f, "{i}"),
            Value::Float(x) => write!(f, "{x}"),
            Value::Str(s) => write!(f, "{s}"),
            Value::Function(p) => write!(f, "function#{p}"),
            Value::Coroutine(id) => write!(f, "coroutine#{id}"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum OpCode {

    Nop = 0,

    PushConst = 1,

    PushNil = 2,

    PushTrue = 3,

    PushFalse = 4,

    Pop = 5,

    Dup = 6,

    LoadLocal = 7,

    StoreLocal = 8,
    GetGlobal = 9,
    SetGlobal = 10,
    MakeClosure = 11,

    Add = 20,
    Sub = 21,
    Mul = 22,
    Div = 23,
    Mod = 24,
    Neg = 25,
    Eq = 26,
    Ne = 27,
    Lt = 28,
    Le = 29,
    Gt = 30,
    Ge = 31,
    Not = 32,

    Jump = 40,
    JumpIfFalse = 41,

    Call = 50,
    Return = 51,
    NewCoroutine = 60,

    Resume = 61,

    Yield = 62,

    CoStatus = 63,

    CoClose = 64,

    Print = 70,

    Halt = 255,
}

impl OpCode {

    pub fn from_u8(byte: u8) -> Option<OpCode> {
        use OpCode::*;
        Some(match byte {
            0 => Nop,
            1 => PushConst,
            2 => PushNil,
            3 => PushTrue,
            4 => PushFalse,
            5 => Pop,
            6 => Dup,
            7 => LoadLocal,
            8 => StoreLocal,
            9 => GetGlobal,
            10 => SetGlobal,
            11 => MakeClosure,
            20 => Add,
            21 => Sub,
            22 => Mul,
            23 => Div,
            24 => Mod,
            25 => Neg,
            26 => Eq,
            27 => Ne,
            28 => Lt,
            29 => Le,
            30 => Gt,
            31 => Ge,
            32 => Not,
            40 => Jump,
            41 => JumpIfFalse,
            50 => Call,
            51 => Return,
            60 => NewCoroutine,
            61 => Resume,
            62 => Yield,
            63 => CoStatus,
            64 => CoClose,
            70 => Print,
            255 => Halt,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Instruction {
    pub op: OpCode,
    pub operand: u32,
}

impl Instruction {
    pub fn new(op: OpCode, operand: u32) -> Self {
        Instruction { op, operand }
    }

    pub fn simple(op: OpCode) -> Self {
        Instruction { op, operand: 0 }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Chunk {

    pub name: String,

    pub num_locals: u16,

    pub constants: Vec<Value>,

    pub code: Vec<Instruction>,
}

impl Chunk {
    pub fn new(name: impl Into<String>) -> Self {
        Chunk { name: name.into(), ..Default::default() }
    }

    pub fn emit(&mut self, instr: Instruction) -> usize {
        self.code.push(instr);
        self.code.len() - 1
    }

    pub fn add_constant(&mut self, value: Value) -> u32 {
        self.constants.push(value);
        (self.constants.len() - 1) as u32
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Program {
    pub protos: Vec<Chunk>,
}

const MAGIC: [u8; 4] = *b"LUAR";

const VERSION: u16 = 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodecError(pub String);

impl fmt::Display for CodecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "bytecode codec error: {}", self.0)
    }
}

impl std::error::Error for CodecError {}

impl Program {
    pub fn new() -> Self {
        Program::default()
    }

    pub fn add_proto(&mut self, chunk: Chunk) -> u32 {
        self.protos.push(chunk);
        (self.protos.len() - 1) as u32
    }

    pub fn main(&self) -> Option<&Chunk> {
        self.protos.first()
    }

    pub fn to_bytes(&self) -> Result<Vec<u8>, CodecError> {
        let mut out = Vec::new();
        out.extend_from_slice(&MAGIC);
        out.extend_from_slice(&VERSION.to_le_bytes());
        write_u32(&mut out, self.protos.len() as u32);
        for chunk in &self.protos {
            write_chunk(&mut out, chunk)?;
        }
        Ok(out)
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Program, CodecError> {
        let mut r = Reader::new(bytes);
        let magic = r.take(4)?;
        if magic != MAGIC {
            return Err(CodecError("bad magic header (not a LUAR program)".into()));
        }
        let version = r.read_u16()?;
        if version != VERSION {
            return Err(CodecError(format!(
                "unsupported bytecode version {version} (expected {VERSION})"
            )));
        }
        let count = r.read_u32()? as usize;
        let mut protos = Vec::with_capacity(count);
        for _ in 0..count {
            protos.push(read_chunk(&mut r)?);
        }
        if !r.at_end() {
            return Err(CodecError("trailing bytes after program".into()));
        }
        Ok(Program { protos })
    }

    pub fn save(&self, path: impl AsRef<std::path::Path>) -> std::io::Result<()> {
        let bytes = self
            .to_bytes()
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.0))?;
        std::fs::write(path, bytes)
    }

    pub fn load(path: impl AsRef<std::path::Path>) -> std::io::Result<Program> {
        let bytes = std::fs::read(path)?;
        Program::from_bytes(&bytes)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.0))
    }
}

fn write_u16(out: &mut Vec<u8>, v: u16) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn write_u32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn write_string(out: &mut Vec<u8>, s: &str) {
    write_u32(out, s.len() as u32);
    out.extend_from_slice(s.as_bytes());
}

fn write_value(out: &mut Vec<u8>, value: &Value) -> Result<(), CodecError> {
    match value {
        Value::Nil => out.push(0),
        Value::Bool(b) => {
            out.push(1);
            out.push(*b as u8);
        }
        Value::Int(i) => {
            out.push(2);
            out.extend_from_slice(&i.to_le_bytes());
        }
        Value::Float(x) => {
            out.push(3);
            out.extend_from_slice(&x.to_bits().to_le_bytes());
        }
        Value::Str(s) => {
            out.push(4);
            write_string(out, s);
        }
        Value::Function(_) => {
            return Err(CodecError("cannot serialize a function value".into()));
        }
        Value::Coroutine(_) => {
            return Err(CodecError("cannot serialize a coroutine handle".into()));
        }
    }
    Ok(())
}

fn write_chunk(out: &mut Vec<u8>, chunk: &Chunk) -> Result<(), CodecError> {
    write_string(out, &chunk.name);
    write_u16(out, chunk.num_locals);
    write_u32(out, chunk.constants.len() as u32);
    for c in &chunk.constants {
        write_value(out, c)?;
    }
    write_u32(out, chunk.code.len() as u32);
    for instr in &chunk.code {
        out.push(instr.op as u8);
        write_u32(out, instr.operand);
    }
    Ok(())
}

struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Reader { bytes, pos: 0 }
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], CodecError> {
        let end = self.pos.checked_add(n).ok_or_else(|| CodecError("length overflow".into()))?;
        let slice = self
            .bytes
            .get(self.pos..end)
            .ok_or_else(|| CodecError("unexpected end of input".into()))?;
        self.pos = end;
        Ok(slice)
    }

    fn read_u8(&mut self) -> Result<u8, CodecError> {
        Ok(self.take(1)?[0])
    }

    fn read_u16(&mut self) -> Result<u16, CodecError> {
        let b = self.take(2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }

    fn read_u32(&mut self) -> Result<u32, CodecError> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn read_i64(&mut self) -> Result<i64, CodecError> {
        let b = self.take(8)?;
        Ok(i64::from_le_bytes(b.try_into().unwrap()))
    }

    fn read_u64(&mut self) -> Result<u64, CodecError> {
        let b = self.take(8)?;
        Ok(u64::from_le_bytes(b.try_into().unwrap()))
    }

    fn read_string(&mut self) -> Result<String, CodecError> {
        let len = self.read_u32()? as usize;
        let bytes = self.take(len)?;
        String::from_utf8(bytes.to_vec()).map_err(|_| CodecError("invalid UTF-8 in string".into()))
    }

    fn at_end(&self) -> bool {
        self.pos >= self.bytes.len()
    }
}

fn read_value(r: &mut Reader) -> Result<Value, CodecError> {
    let tag = r.read_u8()?;
    Ok(match tag {
        0 => Value::Nil,
        1 => Value::Bool(r.read_u8()? != 0),
        2 => Value::Int(r.read_i64()?),
        3 => Value::Float(f64::from_bits(r.read_u64()?)),
        4 => Value::Str(r.read_string()?),
        other => return Err(CodecError(format!("unknown value tag {other}"))),
    })
}

fn read_chunk(r: &mut Reader) -> Result<Chunk, CodecError> {
    let name = r.read_string()?;
    let num_locals = r.read_u16()?;
    let const_count = r.read_u32()? as usize;
    let mut constants = Vec::with_capacity(const_count);
    for _ in 0..const_count {
        constants.push(read_value(r)?);
    }
    let code_count = r.read_u32()? as usize;
    let mut code = Vec::with_capacity(code_count);
    for _ in 0..code_count {
        let op = OpCode::from_u8(r.read_u8()?)
            .ok_or_else(|| CodecError("unknown opcode".into()))?;
        let operand = r.read_u32()?;
        code.push(Instruction { op, operand });
    }
    Ok(Chunk { name, num_locals, constants, code })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_program() -> Program {
        let mut main = Chunk::new("main");
        main.num_locals = 2;
        let c = main.add_constant(Value::Int(7));
        let s = main.add_constant(Value::Str("hi".into()));
        main.emit(Instruction::new(OpCode::PushConst, c));
        main.emit(Instruction::new(OpCode::PushConst, s));
        main.emit(Instruction::simple(OpCode::Print));
        main.emit(Instruction::simple(OpCode::Halt));

        let mut prog = Program::new();
        prog.add_proto(main);
        prog
    }

    #[test]
    fn roundtrips_through_bytes() {
        let prog = sample_program();
        let bytes = prog.to_bytes().unwrap();
        let back = Program::from_bytes(&bytes).unwrap();
        assert_eq!(prog, back);
    }

    #[test]
    fn rejects_bad_magic() {
        let err = Program::from_bytes(b"NOPE........").unwrap_err();
        assert!(err.0.contains("magic"));
    }

    #[test]
    fn refuses_to_serialize_coroutine_handle() {
        let mut chunk = Chunk::new("bad");
        chunk.add_constant(Value::Coroutine(0));
        let mut prog = Program::new();
        prog.add_proto(chunk);
        assert!(prog.to_bytes().is_err());
    }
}
