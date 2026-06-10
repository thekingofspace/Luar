
use crate::ast::*;

const HEADER: &[u8; 7] = b"LUARC\x00\x01";

pub fn pack(stmts: &[Stmt]) -> Vec<u8> {
    let mut w = Writer { buf: HEADER.to_vec() };
    w.vec(stmts, Writer::stmt);
    w.buf
}

pub fn unpack(bytes: &[u8]) -> Result<Vec<Stmt>, String> {
    if bytes.len() < HEADER.len() || &bytes[..HEADER.len()] != HEADER {
        return Err("not a LUAR precompiled file (bad header)".into());
    }
    let mut r = Reader { buf: bytes, pos: HEADER.len() };
    let stmts = r.vec(Reader::stmt)?;
    Ok(stmts)
}

struct Writer {
    buf: Vec<u8>,
}

impl Writer {
    fn u8(&mut self, v: u8) {
        self.buf.push(v);
    }
    fn u32(&mut self, v: u32) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
    fn i64(&mut self, v: i64) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
    fn u64(&mut self, v: u64) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
    fn f64(&mut self, v: f64) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
    fn bool(&mut self, v: bool) {
        self.u8(v as u8);
    }
    fn str(&mut self, s: &str) {
        self.u32(s.len() as u32);
        self.buf.extend_from_slice(s.as_bytes());
    }
    fn vec<T>(&mut self, items: &[T], mut f: impl FnMut(&mut Self, &T)) {
        self.u32(items.len() as u32);
        for it in items {
            f(self, it);
        }
    }
    fn opt<T>(&mut self, v: &Option<T>, mut f: impl FnMut(&mut Self, &T)) {
        match v {
            Some(x) => {
                self.u8(1);
                f(self, x);
            }
            None => self.u8(0),
        }
    }

    fn visibility(&mut self, v: &Visibility) {
        self.u8(match v {
            Visibility::Local => 0,
            Visibility::Pub => 1,
        });
    }
    fn mutability(&mut self, m: &Mutability) {
        self.u8(match m {
            Mutability::Mutable => 0,
            Mutability::Const => 1,
        });
    }
    fn access(&mut self, a: &Access) {
        self.u8(match a {
            Access::Public => 0,
            Access::Protected => 1,
            Access::Private => 2,
        });
    }

    fn stmt(&mut self, s: &Stmt) {
        match s {
            Stmt::Declare { visibility, mutability, names, inits, line } => {
                self.u8(0);
                self.visibility(visibility);
                self.mutability(mutability);
                self.vec(names, |w, n| w.str(n));
                self.vec(inits, Writer::expr);
                self.u32(*line);
            }
            Stmt::Assign { targets, op, values, line } => {
                self.u8(1);
                self.vec(targets, Writer::lvalue);
                self.assign_op(op);
                self.vec(values, Writer::expr);
                self.u32(*line);
            }
            Stmt::Do(body) => {
                self.u8(2);
                self.vec(body, Writer::stmt);
            }
            Stmt::If { branches, else_block, line } => {
                self.u8(3);
                self.vec(branches, |w, (cond, body)| {
                    w.expr(cond);
                    w.vec(body, Writer::stmt);
                });
                self.opt(else_block, |w, b| w.vec(b, Writer::stmt));
                self.u32(*line);
            }
            Stmt::While { cond, body, line } => {
                self.u8(4);
                self.expr(cond);
                self.vec(body, Writer::stmt);
                self.u32(*line);
            }
            Stmt::ForNumeric { var, start, stop, step, body } => {
                self.u8(5);
                self.str(var);
                self.expr(start);
                self.expr(stop);
                self.opt(step, Writer::expr);
                self.vec(body, Writer::stmt);
            }
            Stmt::ForIn { names, iters, body } => {
                self.u8(6);
                self.vec(names, |w, n| w.str(n));
                self.vec(iters, Writer::expr);
                self.vec(body, Writer::stmt);
            }
            Stmt::Break { line } => {
                self.u8(7);
                self.u32(*line);
            }
            Stmt::Return { values, line } => {
                self.u8(8);
                self.vec(values, Writer::expr);
                self.u32(*line);
            }
            Stmt::TypeAlias { name, ty } => {
                self.u8(9);
                self.str(name);
                self.ty(ty);
            }
            Stmt::Class { visibility, is_final, is_abstract, name, parent, mixins, interfaces, members } => {
                self.u8(10);
                self.visibility(visibility);
                self.bool(*is_final);
                self.bool(*is_abstract);
                self.str(name);
                self.opt(parent, |w, p| w.str(p));
                self.vec(mixins, |w, m| w.str(m));
                self.vec(interfaces, |w, i| w.str(i));
                self.vec(members, Writer::member);
            }
            Stmt::Interface { visibility, name, parents, members } => {
                self.u8(11);
                self.visibility(visibility);
                self.str(name);
                self.vec(parents, |w, p| w.str(p));
                self.vec(members, |w, m| w.str(m));
            }
            Stmt::Enum { visibility, name, variants, line } => {
                self.u8(13);
                self.visibility(visibility);
                self.str(name);
                self.vec(variants, |w, (n, v)| {
                    w.str(n);
                    w.opt(v, Writer::expr);
                });
                self.u32(*line);
            }
            Stmt::Expr(e, line) => {
                self.u8(12);
                self.expr(e);
                self.u32(*line);
            }
            Stmt::Buff { name, size, init, line } => {
                self.u8(14);
                self.str(name);
                self.u64(*size);
                self.expr(init);
                self.u32(*line);
            }
            Stmt::FreeBuff { name, line } => {
                self.u8(15);
                self.str(name);
                self.u32(*line);
            }
        }
    }

    fn assign_op(&mut self, op: &AssignOp) {
        self.u8(match op {
            AssignOp::Assign => 0,
            AssignOp::Add => 1,
            AssignOp::Sub => 2,
            AssignOp::Mul => 3,
            AssignOp::Div => 4,
            AssignOp::Mod => 5,
            AssignOp::Concat => 6,
        });
    }

    fn lvalue(&mut self, lv: &LValue) {
        match lv {
            LValue::Name(n) => {
                self.u8(0);
                self.str(n);
            }
            LValue::Index { base, key } => {
                self.u8(1);
                self.expr(base);
                self.expr(key);
            }
        }
    }

    fn fnbody(&mut self, f: &FnBody) {
        self.vec(&f.params, |w, p| w.str(p));
        self.bool(f.is_vararg);
        self.vec(&f.body, Writer::stmt);
    }

    fn member(&mut self, m: &ClassMember) {
        match m {
            ClassMember::Field { access, is_static, name, default } => {
                self.u8(0);
                self.access(access);
                self.bool(*is_static);
                self.str(name);
                self.opt(default, Writer::expr);
            }
            ClassMember::Method { access, is_static, is_abstract, is_final, is_override, name, func } => {
                self.u8(1);
                self.access(access);
                self.bool(*is_static);
                self.bool(*is_abstract);
                self.bool(*is_final);
                self.bool(*is_override);
                self.str(name);
                self.fnbody(func);
            }
            ClassMember::Getter { access, name, func } => {
                self.u8(2);
                self.access(access);
                self.str(name);
                self.fnbody(func);
            }
            ClassMember::Setter { access, name, func } => {
                self.u8(3);
                self.access(access);
                self.str(name);
                self.fnbody(func);
            }
            ClassMember::Constructor { func } => {
                self.u8(4);
                self.fnbody(func);
            }
            ClassMember::Operator { symbol, func } => {
                self.u8(5);
                self.str(symbol);
                self.fnbody(func);
            }
            ClassMember::Destructor { func } => {
                self.u8(6);
                self.fnbody(func);
            }
        }
    }

    fn ty(&mut self, t: &Type) {
        match t {
            Type::Named(n) => {
                self.u8(0);
                self.str(n);
            }
            Type::Literal(s) => {
                self.u8(1);
                self.str(s);
            }
            Type::Table(fields) => {
                self.u8(2);
                self.vec(fields, |w, (k, v)| {
                    w.str(k);
                    w.ty(v);
                });
            }
            Type::Array(inner) => {
                self.u8(3);
                self.ty(inner);
            }
            Type::Optional(inner) => {
                self.u8(4);
                self.ty(inner);
            }
            Type::Function { params, ret } => {
                self.u8(5);
                self.vec(params, Writer::ty);
                self.ty(ret);
            }
            Type::Union(parts) => {
                self.u8(6);
                self.vec(parts, Writer::ty);
            }
            Type::Intersection(parts) => {
                self.u8(7);
                self.vec(parts, Writer::ty);
            }
        }
    }

    fn table_entry(&mut self, e: &TableEntry) {
        match e {
            TableEntry::Positional(v) => {
                self.u8(0);
                self.expr(v);
            }
            TableEntry::Keyed { key, value } => {
                self.u8(1);
                self.expr(key);
                self.expr(value);
            }
        }
    }

    fn switch_case(&mut self, c: &SwitchCase) {
        self.expr(&c.pattern);
        self.vec(&c.body, Writer::stmt);
    }

    fn expr(&mut self, e: &Expr) {
        match e {
            Expr::Nil => self.u8(0),
            Expr::Bool(b) => {
                self.u8(1);
                self.bool(*b);
            }
            Expr::Int(n) => {
                self.u8(2);
                self.i64(*n);
            }
            Expr::Float(f) => {
                self.u8(3);
                self.f64(*f);
            }
            Expr::Str(s) => {
                self.u8(4);
                self.str(s);
            }
            Expr::Name(n) => {
                self.u8(5);
                self.str(n);
            }
            Expr::Table(entries) => {
                self.u8(6);
                self.vec(entries, Writer::table_entry);
            }
            Expr::Index { base, key } => {
                self.u8(7);
                self.expr(base);
                self.expr(key);
            }
            Expr::Call { callee, args } => {
                self.u8(8);
                self.expr(callee);
                self.vec(args, Writer::expr);
            }
            Expr::Function { name, params, is_vararg, body } => {
                self.u8(9);
                self.str(name);
                self.vec(params, |w, p| w.str(p));
                self.bool(*is_vararg);
                self.vec(body, Writer::stmt);
            }
            Expr::Vararg => self.u8(10),
            Expr::MethodCall { receiver, method, args } => {
                self.u8(11);
                self.expr(receiver);
                self.str(method);
                self.vec(args, Writer::expr);
            }
            Expr::Switch { subject, cases, default } => {
                self.u8(12);
                self.expr(subject);
                self.vec(cases, Writer::switch_case);
                self.opt(default, |w, b| w.vec(b, Writer::stmt));
            }
            Expr::Unary { op, expr } => {
                self.u8(13);
                self.u8(match op {
                    UnaryOp::Neg => 0,
                    UnaryOp::Not => 1,
                    UnaryOp::Len => 2,
                });
                self.expr(expr);
            }
            Expr::Binary { op, lhs, rhs } => {
                self.u8(14);
                self.bin_op(op);
                self.expr(lhs);
                self.expr(rhs);
            }
            Expr::Logical { op, lhs, rhs } => {
                self.u8(15);
                self.u8(match op {
                    LogicalOp::And => 0,
                    LogicalOp::Or => 1,
                });
                self.expr(lhs);
                self.expr(rhs);
            }
        }
    }

    fn bin_op(&mut self, op: &BinOp) {
        self.u8(match op {
            BinOp::Add => 0,
            BinOp::Sub => 1,
            BinOp::Mul => 2,
            BinOp::Div => 3,
            BinOp::Mod => 4,
            BinOp::Pow => 5,
            BinOp::Concat => 6,
            BinOp::Eq => 7,
            BinOp::Ne => 8,
            BinOp::Lt => 9,
            BinOp::Le => 10,
            BinOp::Gt => 11,
            BinOp::Ge => 12,
        });
    }
}

struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        if self.pos + n > self.buf.len() {
            return Err("unexpected end of precompiled data".into());
        }
        let s = &self.buf[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }
    fn u8(&mut self) -> Result<u8, String> {
        Ok(self.take(1)?[0])
    }
    fn u32(&mut self) -> Result<u32, String> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn i64(&mut self) -> Result<i64, String> {
        Ok(i64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
    fn u64(&mut self) -> Result<u64, String> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
    fn f64(&mut self) -> Result<f64, String> {
        Ok(f64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
    fn bool(&mut self) -> Result<bool, String> {
        Ok(self.u8()? != 0)
    }
    fn str(&mut self) -> Result<String, String> {
        let n = self.u32()? as usize;
        let bytes = self.take(n)?;
        String::from_utf8(bytes.to_vec()).map_err(|_| "invalid UTF-8 in precompiled string".into())
    }
    fn vec<T>(&mut self, mut f: impl FnMut(&mut Self) -> Result<T, String>) -> Result<Vec<T>, String> {
        let n = self.u32()? as usize;
        let mut out = Vec::with_capacity(n);
        for _ in 0..n {
            out.push(f(self)?);
        }
        Ok(out)
    }
    fn opt<T>(&mut self, mut f: impl FnMut(&mut Self) -> Result<T, String>) -> Result<Option<T>, String> {
        if self.u8()? == 1 {
            Ok(Some(f(self)?))
        } else {
            Ok(None)
        }
    }

    fn visibility(&mut self) -> Result<Visibility, String> {
        Ok(match self.u8()? {
            0 => Visibility::Local,
            1 => Visibility::Pub,
            t => return Err(format!("bad visibility tag {t}")),
        })
    }
    fn mutability(&mut self) -> Result<Mutability, String> {
        Ok(match self.u8()? {
            0 => Mutability::Mutable,
            1 => Mutability::Const,
            t => return Err(format!("bad mutability tag {t}")),
        })
    }
    fn access(&mut self) -> Result<Access, String> {
        Ok(match self.u8()? {
            0 => Access::Public,
            1 => Access::Protected,
            2 => Access::Private,
            t => return Err(format!("bad access tag {t}")),
        })
    }

    fn stmt(&mut self) -> Result<Stmt, String> {
        Ok(match self.u8()? {
            0 => Stmt::Declare {
                visibility: self.visibility()?,
                mutability: self.mutability()?,
                names: self.vec(Reader::str)?,
                inits: self.vec(Reader::expr)?,
                line: self.u32()?,
            },
            1 => Stmt::Assign {
                targets: self.vec(Reader::lvalue)?,
                op: self.assign_op()?,
                values: self.vec(Reader::expr)?,
                line: self.u32()?,
            },
            2 => Stmt::Do(self.vec(Reader::stmt)?),
            3 => Stmt::If {
                branches: self.vec(|r| Ok((r.expr()?, r.vec(Reader::stmt)?)))?,
                else_block: self.opt(|r| r.vec(Reader::stmt))?,
                line: self.u32()?,
            },
            4 => Stmt::While {
                cond: self.expr()?,
                body: self.vec(Reader::stmt)?,
                line: self.u32()?,
            },
            5 => Stmt::ForNumeric {
                var: self.str()?,
                start: self.expr()?,
                stop: self.expr()?,
                step: self.opt(Reader::expr)?,
                body: self.vec(Reader::stmt)?,
            },
            6 => Stmt::ForIn {
                names: self.vec(Reader::str)?,
                iters: self.vec(Reader::expr)?,
                body: self.vec(Reader::stmt)?,
            },
            7 => Stmt::Break { line: self.u32()? },
            8 => Stmt::Return {
                values: self.vec(Reader::expr)?,
                line: self.u32()?,
            },
            9 => Stmt::TypeAlias {
                name: self.str()?,
                ty: self.ty()?,
            },
            10 => Stmt::Class {
                visibility: self.visibility()?,
                is_final: self.bool()?,
                is_abstract: self.bool()?,
                name: self.str()?,
                parent: self.opt(Reader::str)?,
                mixins: self.vec(Reader::str)?,
                interfaces: self.vec(Reader::str)?,
                members: self.vec(Reader::member)?,
            },
            11 => Stmt::Interface {
                visibility: self.visibility()?,
                name: self.str()?,
                parents: self.vec(Reader::str)?,
                members: self.vec(Reader::str)?,
            },
            12 => Stmt::Expr(self.expr()?, self.u32()?),
            13 => Stmt::Enum {
                visibility: self.visibility()?,
                name: self.str()?,
                variants: self.vec(|r| Ok((r.str()?, r.opt(Reader::expr)?)))?,
                line: self.u32()?,
            },
            14 => Stmt::Buff {
                name: self.str()?,
                size: self.u64()?,
                init: self.expr()?,
                line: self.u32()?,
            },
            15 => Stmt::FreeBuff {
                name: self.str()?,
                line: self.u32()?,
            },
            t => return Err(format!("bad statement tag {t}")),
        })
    }

    fn assign_op(&mut self) -> Result<AssignOp, String> {
        Ok(match self.u8()? {
            0 => AssignOp::Assign,
            1 => AssignOp::Add,
            2 => AssignOp::Sub,
            3 => AssignOp::Mul,
            4 => AssignOp::Div,
            5 => AssignOp::Mod,
            6 => AssignOp::Concat,
            t => return Err(format!("bad assign-op tag {t}")),
        })
    }

    fn lvalue(&mut self) -> Result<LValue, String> {
        Ok(match self.u8()? {
            0 => LValue::Name(self.str()?),
            1 => LValue::Index { base: Box::new(self.expr()?), key: Box::new(self.expr()?) },
            t => return Err(format!("bad lvalue tag {t}")),
        })
    }

    fn fnbody(&mut self) -> Result<FnBody, String> {
        Ok(FnBody {
            params: self.vec(Reader::str)?,
            is_vararg: self.bool()?,
            body: self.vec(Reader::stmt)?,
        })
    }

    fn member(&mut self) -> Result<ClassMember, String> {
        Ok(match self.u8()? {
            0 => ClassMember::Field {
                access: self.access()?,
                is_static: self.bool()?,
                name: self.str()?,
                default: self.opt(Reader::expr)?,
            },
            1 => ClassMember::Method {
                access: self.access()?,
                is_static: self.bool()?,
                is_abstract: self.bool()?,
                is_final: self.bool()?,
                is_override: self.bool()?,
                name: self.str()?,
                func: self.fnbody()?,
            },
            2 => ClassMember::Getter { access: self.access()?, name: self.str()?, func: self.fnbody()? },
            3 => ClassMember::Setter { access: self.access()?, name: self.str()?, func: self.fnbody()? },
            4 => ClassMember::Constructor { func: self.fnbody()? },
            5 => ClassMember::Operator { symbol: self.str()?, func: self.fnbody()? },
            6 => ClassMember::Destructor { func: self.fnbody()? },
            t => return Err(format!("bad member tag {t}")),
        })
    }

    fn ty(&mut self) -> Result<Type, String> {
        Ok(match self.u8()? {
            0 => Type::Named(self.str()?),
            1 => Type::Literal(self.str()?),
            2 => Type::Table(self.vec(|r| Ok((r.str()?, r.ty()?)))?),
            3 => Type::Array(Box::new(self.ty()?)),
            4 => Type::Optional(Box::new(self.ty()?)),
            5 => Type::Function { params: self.vec(Reader::ty)?, ret: Box::new(self.ty()?) },
            6 => Type::Union(self.vec(Reader::ty)?),
            7 => Type::Intersection(self.vec(Reader::ty)?),
            t => return Err(format!("bad type tag {t}")),
        })
    }

    fn table_entry(&mut self) -> Result<TableEntry, String> {
        Ok(match self.u8()? {
            0 => TableEntry::Positional(self.expr()?),
            1 => TableEntry::Keyed { key: self.expr()?, value: self.expr()? },
            t => return Err(format!("bad table-entry tag {t}")),
        })
    }

    fn switch_case(&mut self) -> Result<SwitchCase, String> {
        Ok(SwitchCase { pattern: self.expr()?, body: self.vec(Reader::stmt)? })
    }

    fn expr(&mut self) -> Result<Expr, String> {
        Ok(match self.u8()? {
            0 => Expr::Nil,
            1 => Expr::Bool(self.bool()?),
            2 => Expr::Int(self.i64()?),
            3 => Expr::Float(self.f64()?),
            4 => Expr::Str(self.str()?),
            5 => Expr::Name(self.str()?),
            6 => Expr::Table(self.vec(Reader::table_entry)?),
            7 => Expr::Index { base: Box::new(self.expr()?), key: Box::new(self.expr()?) },
            8 => Expr::Call { callee: Box::new(self.expr()?), args: self.vec(Reader::expr)? },
            9 => Expr::Function {
                name: self.str()?,
                params: self.vec(Reader::str)?,
                is_vararg: self.bool()?,
                body: self.vec(Reader::stmt)?,
            },
            10 => Expr::Vararg,
            11 => Expr::MethodCall {
                receiver: Box::new(self.expr()?),
                method: self.str()?,
                args: self.vec(Reader::expr)?,
            },
            12 => Expr::Switch {
                subject: Box::new(self.expr()?),
                cases: self.vec(Reader::switch_case)?,
                default: self.opt(|r| r.vec(Reader::stmt))?,
            },
            13 => Expr::Unary {
                op: match self.u8()? {
                    0 => UnaryOp::Neg,
                    1 => UnaryOp::Not,
                    2 => UnaryOp::Len,
                    t => return Err(format!("bad unary-op tag {t}")),
                },
                expr: Box::new(self.expr()?),
            },
            14 => Expr::Binary {
                op: self.bin_op()?,
                lhs: Box::new(self.expr()?),
                rhs: Box::new(self.expr()?),
            },
            15 => Expr::Logical {
                op: match self.u8()? {
                    0 => LogicalOp::And,
                    1 => LogicalOp::Or,
                    t => return Err(format!("bad logical-op tag {t}")),
                },
                lhs: Box::new(self.expr()?),
                rhs: Box::new(self.expr()?),
            },
            t => return Err(format!("bad expression tag {t}")),
        })
    }

    fn bin_op(&mut self) -> Result<BinOp, String> {
        Ok(match self.u8()? {
            0 => BinOp::Add,
            1 => BinOp::Sub,
            2 => BinOp::Mul,
            3 => BinOp::Div,
            4 => BinOp::Mod,
            5 => BinOp::Pow,
            6 => BinOp::Concat,
            7 => BinOp::Eq,
            8 => BinOp::Ne,
            9 => BinOp::Lt,
            10 => BinOp::Le,
            11 => BinOp::Gt,
            12 => BinOp::Ge,
            t => return Err(format!("bad binary-op tag {t}")),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse;
    use crate::lexer::tokenize;

    fn roundtrip(src: &str) {
        let stmts = parse(tokenize(src).unwrap()).unwrap();
        let bytes = pack(&stmts);
        let back = unpack(&bytes).unwrap();
        for (i, (a, b)) in stmts.iter().zip(back.iter()).enumerate() {
            assert_eq!(a, b, "round-trip mismatch at stmt {i}:\n  in={a:?}\n out={b:?}");
        }
        assert_eq!(stmts.len(), back.len(), "stmt count mismatch");
    }

    #[test]
    fn roundtrips_every_node() {
        roundtrip(
            r#"
            type Box<T> = { value: T }
            pub enum Color { Red Green Blue }
            enum Status { Active = 10 Inactive Banned = 99 }
            abstract class Animal implements Named {
              public name: string = "a"
              private id: number = 0
              static count: number = 0
              constructor(n) self.name = n end
              abstract function speak() end
              get tired() return false end
              set tired(v) self.id = 1 end
              operator +(o) return self end
              final function tag() return "x" end
            }
            final class Dog extends Animal mixin Loggable {
              override function speak() return "woof" end
            }
            class C extends MonoBehaviour {
              function Update(): void
                local t = { 1, 2, [3] = "x", k = true }
                for i = 1, 10, 2 do print(i) end
                for k, v in pairs(t) do print(k, v) end
                local s = switch(self.name)
                  case "a"
                    return 1
                  end
                  default
                    return 0
                  end
                end
                local n = #t + 1
                local m = -n
                local cat = "a" .. "b"
                while n > 0 and not false do n -= 1 end
                if n == 0 then print("done") elseif n > 5 then print("hi") else print("x") end
                return
              end
            }
            local f = function(a, ...) return a end
            print(f "sugar")
            "#,
        );
    }

    #[test]
    fn rejects_bad_header() {
        assert!(unpack(b"nope").is_err());
    }

    #[test]
    fn precompiled_program_runs_with_all_features() {

        let src = r#"
            class Greet { function hi() return "hi" end }
            abstract class Base { abstract function id(): number end }
            class Thing extends Base mixin Greet {
              public n: number = 0
              override function id(): number return self.n end
              operator +(o) local t = Thing() t.n = self.n + o.n return t end
            }
            pub local freed = false
            class Ticker {
              public count: number = 0
              destructor() freed = true end
            }
            do local tk = Ticker() end
            const a = Thing()
            a.n = 2
            const b = Thing()
            b.n = 3
            const c = a + b
            pub local sum = c:id()
            pub local greeting = a:hi()
        "#;
        let bytes = crate::precompile_source(src).expect("precompiles");
        let interp = crate::run_precompiled(&bytes).expect("runs");
        assert_eq!(interp.env.get("sum"), Some(crate::runtime::Value::Int(5)));
        assert_eq!(interp.env.get("greeting"), Some(crate::runtime::Value::str("hi")));
        assert_eq!(interp.env.get("freed"), Some(crate::runtime::Value::Bool(true)));
    }

    #[test]
    fn precompiled_module_returns_flow_to_host_and_other_script() {
        use crate::runtime::Value;

        let module_src = r#"
            pub local name = "mathmod"
            local M = {}
            function M.add(a, b) return a + b end
            function M.mul(a, b) return a * b end
            return M
        "#;
        let bytes = crate::precompile_source(module_src).expect("precompiles");

        let (interp, returns) = crate::run_precompiled_returns(&bytes).expect("runs");
        assert_eq!(interp.env.get("name"), Some(Value::str("mathmod")));
        assert_eq!(returns.len(), 1, "module returned exactly one value");

        let module = crate::load_precompiled_module(&bytes).expect("loads");
        assert!(matches!(module, Value::Table(_)), "module is a table");

        let mut host = crate::Interpreter::new();
        host.set_global("math", module);
        host.run_source("pub local sum = math.add(2, 3)  pub local prod = math.mul(4, 5)")
            .expect("other script runs");
        assert_eq!(host.get_global("sum"), Some(Value::Int(5)));
        assert_eq!(host.get_global("prod"), Some(Value::Int(20)));

        let mut shared = crate::Interpreter::new();
        let returned = shared.run_precompiled(&bytes).expect("runs into shared interp");
        assert_eq!(returned.len(), 1);
        assert_eq!(shared.get_global("name"), Some(Value::str("mathmod")));
    }

    #[test]
    fn precompiled_module_with_no_return_yields_nil() {
        use crate::runtime::Value;
        let bytes = crate::precompile_source("pub local x = 1").expect("precompiles");
        let module = crate::load_precompiled_module(&bytes).expect("loads");
        assert_eq!(module, Value::Nil);
        let (_, returns) = crate::run_precompiled_returns(&bytes).expect("runs");
        assert!(returns.is_empty());
    }

    #[test]
    fn precompiled_enum_switch_interface_loops() {
        let src = r#"
            pub enum Color { Red Green Blue }
            enum Color { Yellow }
            enum Status { Active = 10 Inactive Banned = 99 }
            interface Named { name }
            class Tag implements Named { public name: string = "t" }
            local function classify(n)
              return switch(n)
                case 1 return "one" end
                case 2 return "two" end
                default return "many" end
              end
            end
            local total = 0
            for i = 1, 5 do total += i end
            for _, v in ipairs({ 10, 20, 30 }) do total += v end
            local k = 3
            while k > 0 do total += 1  k -= 1 end
            pub local sum = total
            pub local red = Color.Red
            pub local yellow = Color.Yellow
            pub local inactive = Status.Inactive
            pub local pick = classify(2)
            pub local named = instanceof(Tag(), Named)
        "#;
        let bytes = crate::precompile_source(src).expect("precompiles");
        let i = crate::run_precompiled(&bytes).expect("runs");
        use crate::runtime::Value;
        assert_eq!(i.env.get("sum"), Some(Value::Int(78)));
        assert_eq!(i.env.get("red"), Some(Value::Int(0)));
        assert_eq!(i.env.get("yellow"), Some(Value::Int(3)));
        assert_eq!(i.env.get("inactive"), Some(Value::Int(11)));
        assert_eq!(i.env.get("pick"), Some(Value::str("two")));
        assert_eq!(i.env.get("named"), Some(Value::Bool(true)));
    }
}
