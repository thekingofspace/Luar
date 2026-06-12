
use crate::ast::{
    AssignOp, ClassMember, Expr, LValue, Mutability, Stmt, TableEntry, UnaryOp, Visibility,
};
use crate::runtime::block_creates_functions;

const MAX_TRIPS: i128 = 16;
const MAX_EXPANDED_STMTS: usize = 64;

pub fn optimize_program(stmts: &mut Vec<Stmt>) {
    optimize_block(stmts);
}

fn optimize_block(stmts: &mut Vec<Stmt>) {
    for stmt in stmts.iter_mut() {
        optimize_stmt(stmt);
    }
    for stmt in stmts.iter_mut() {
        if let Some(unrolled) = try_unroll(stmt) {
            *stmt = unrolled;
        }
    }
}

fn optimize_stmt(stmt: &mut Stmt) {
    match stmt {
        Stmt::Do(body) => optimize_block(body),
        Stmt::If { branches, else_block, .. } => {
            for (cond, body) in branches.iter_mut() {
                optimize_expr(cond);
                optimize_block(body);
            }
            if let Some(body) = else_block {
                optimize_block(body);
            }
        }
        Stmt::While { cond, body, .. } => {
            optimize_expr(cond);
            optimize_block(body);
        }
        Stmt::ForNumeric { start, stop, step, body, .. } => {
            optimize_expr(start);
            optimize_expr(stop);
            if let Some(e) = step {
                optimize_expr(e);
            }
            optimize_block(body);
        }
        Stmt::ForIn { iters, body, .. } => {
            for e in iters.iter_mut() {
                optimize_expr(e);
            }
            optimize_block(body);
        }
        Stmt::Declare { inits, .. } => {
            for e in inits.iter_mut() {
                optimize_expr(e);
            }
        }
        Stmt::Assign { targets, values, .. } => {
            for t in targets.iter_mut() {
                if let LValue::Index { base, key } = t {
                    optimize_expr(base);
                    optimize_expr(key);
                }
            }
            for e in values.iter_mut() {
                optimize_expr(e);
            }
        }
        Stmt::Return { values, .. } => {
            for e in values.iter_mut() {
                optimize_expr(e);
            }
        }
        Stmt::Buff { init, .. } => optimize_expr(init),
        Stmt::Class { members, .. } => {
            for m in members.iter_mut() {
                match m {
                    ClassMember::Field { default: Some(e), .. } => optimize_expr(e),
                    ClassMember::Field { .. } => {}
                    ClassMember::Method { func, .. }
                    | ClassMember::Getter { func, .. }
                    | ClassMember::Setter { func, .. }
                    | ClassMember::Constructor { func }
                    | ClassMember::Destructor { func }
                    | ClassMember::Operator { func, .. } => optimize_block(&mut func.body),
                }
            }
        }
        Stmt::Enum { variants, .. } => {
            for (_, v) in variants.iter_mut() {
                if let Some(e) = v {
                    optimize_expr(e);
                }
            }
        }
        Stmt::Expr(e, _) => optimize_expr(e),
        Stmt::Break { .. }
        | Stmt::FreeBuff { .. }
        | Stmt::TypeAlias { .. }
        | Stmt::Interface { .. } => {}
    }
}

fn optimize_expr(expr: &mut Expr) {
    match expr {
        Expr::Function { body, .. } => optimize_block(body),
        Expr::Call { callee, args } => {
            optimize_expr(callee);
            for a in args.iter_mut() {
                optimize_expr(a);
            }
        }
        Expr::MethodCall { receiver, args, .. } => {
            optimize_expr(receiver);
            for a in args.iter_mut() {
                optimize_expr(a);
            }
        }
        Expr::Index { base, key } => {
            optimize_expr(base);
            optimize_expr(key);
        }
        Expr::Table(entries) => {
            for e in entries.iter_mut() {
                match e {
                    TableEntry::Positional(v) => optimize_expr(v),
                    TableEntry::Keyed { key, value } => {
                        optimize_expr(key);
                        optimize_expr(value);
                    }
                }
            }
        }
        Expr::Switch { subject, cases, default } => {
            optimize_expr(subject);
            for c in cases.iter_mut() {
                optimize_expr(&mut c.pattern);
                optimize_block(&mut c.body);
            }
            if let Some(b) = default {
                optimize_block(b);
            }
        }
        Expr::Unary { expr, .. } => optimize_expr(expr),
        Expr::Binary { lhs, rhs, .. } | Expr::Logical { lhs, rhs, .. } => {
            optimize_expr(lhs);
            optimize_expr(rhs);
        }
        Expr::Nil
        | Expr::Bool(_)
        | Expr::Int(_)
        | Expr::Float(_)
        | Expr::Str(_)
        | Expr::Name(_)
        | Expr::Vararg => {}
    }
}

fn const_int(e: &Expr) -> Option<i64> {
    match e {
        Expr::Int(n) => Some(*n),
        Expr::Unary { op: UnaryOp::Neg, expr } => match expr.as_ref() {
            Expr::Int(n) => n.checked_neg(),
            _ => None,
        },
        _ => None,
    }
}

fn block_has_break(stmts: &[Stmt]) -> bool {
    stmts.iter().any(stmt_has_break)
}

fn stmt_has_break(stmt: &Stmt) -> bool {
    match stmt {
        Stmt::Break { .. } => true,
        Stmt::Do(body) => block_has_break(body),
        Stmt::If { branches, else_block, .. } => {
            branches.iter().any(|(_, b)| block_has_break(b))
                || else_block.as_ref().is_some_and(|b| block_has_break(b))
        }
        _ => false,
    }
}

fn trip_count(start: i64, stop: i64, step: i64) -> i128 {
    let (start, stop, step) = (start as i128, stop as i128, step as i128);
    if step > 0 {
        if start > stop {
            0
        } else {
            (stop - start) / step + 1
        }
    } else if start < stop {
        0
    } else {
        (start - stop) / (-step) + 1
    }
}

fn try_unroll(stmt: &Stmt) -> Option<Stmt> {
    let Stmt::ForNumeric { var, start, stop, step, body, line } = stmt else {
        return None;
    };
    let start_v = const_int(start)?;
    let stop_v = const_int(stop)?;
    let step_v = match step {
        Some(e) => const_int(e)?,
        None => 1,
    };
    if step_v == 0 {
        return None;
    }
    let trips = trip_count(start_v, stop_v, step_v);
    if trips == 0 {
        return Some(Stmt::Do(Vec::new()));
    }
    if trips > MAX_TRIPS {
        return None;
    }
    let expanded = (trips as usize) * (body.len() + 1);
    if expanded > MAX_EXPANDED_STMTS {
        return None;
    }
    if block_has_break(body) || block_creates_functions(body) {
        return None;
    }
    let mut out: Vec<Stmt> = Vec::with_capacity(expanded + 1);
    out.push(Stmt::Declare {
        visibility: Visibility::Local,
        mutability: Mutability::Mutable,
        names: vec![var.clone()],
        inits: vec![Expr::Int(start_v)],
        line: *line,
    });
    out.push(Stmt::Do(body.clone()));
    let mut next = start_v as i128 + step_v as i128;
    for _ in 1..trips {
        out.push(Stmt::Assign {
            targets: vec![LValue::Name(var.clone())],
            op: AssignOp::Assign,
            values: vec![Expr::Int(next as i64)],
            line: *line,
        });
        out.push(Stmt::Do(body.clone()));
        next += step_v as i128;
    }
    Some(Stmt::Do(out))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::{Interpreter, Value};

    fn run_optimized(src: &str) -> Interpreter {
        let mut program = crate::parse_source(src).unwrap();
        optimize_program(&mut program);
        let mut interp = Interpreter::new();
        interp.run(&program).unwrap();
        interp
    }

    fn optimized(src: &str) -> Vec<Stmt> {
        let mut program = crate::parse_source(src).unwrap();
        optimize_program(&mut program);
        program
    }

    #[test]
    fn unrolls_small_constant_loops() {
        let program = optimized("pub local x = 0\nfor i = 1, 4 do\n    x += i\nend");
        assert!(matches!(program[1], Stmt::Do(_)), "{:?}", program[1]);
        let i = run_optimized("pub local x = 0\nfor i = 1, 4 do\n    x += i\nend");
        assert_eq!(i.env.get("x"), Some(Value::Int(10)));
    }

    #[test]
    fn unrolls_negative_step_loops() {
        let i = run_optimized("pub local x = 0\nfor i = 10, 8, -1 do\n    x += i\nend");
        assert_eq!(i.env.get("x"), Some(Value::Int(27)));
    }

    #[test]
    fn zero_trip_loops_become_empty() {
        let program = optimized("pub local x = 0\nfor i = 1, 0 do\n    x += 1\nend");
        assert!(matches!(&program[1], Stmt::Do(b) if b.is_empty()));
        let i = run_optimized("pub local x = 0\nfor i = 1, 0 do\n    x += 1\nend");
        assert_eq!(i.env.get("x"), Some(Value::Int(0)));
    }

    #[test]
    fn large_loops_stay_loops() {
        let program = optimized("pub local x = 0\nfor i = 1, 1000 do\n    x += i\nend");
        assert!(matches!(program[1], Stmt::ForNumeric { .. }));
    }

    #[test]
    fn float_bounds_stay_loops() {
        let program = optimized("pub local x = 0\nfor i = 1.0, 4.0 do\n    x += i\nend");
        assert!(matches!(program[1], Stmt::ForNumeric { .. }));
    }

    #[test]
    fn loops_with_break_stay_loops() {
        let src = "pub local x = 0\nfor i = 1, 4 do\n    if i == 2 then\n        break\n    end\n    x += i\nend";
        let program = optimized(src);
        assert!(matches!(program[1], Stmt::ForNumeric { .. }));
        let i = run_optimized(src);
        assert_eq!(i.env.get("x"), Some(Value::Int(1)));
    }

    #[test]
    fn closure_capturing_loops_stay_loops() {
        let src = "pub local fns = {}\nfor i = 1, 3 do\n    fns[i] = function()\n        return i\n    end\nend\npub local v = fns[1]() + fns[2]() + fns[3]()";
        let program = optimized(src);
        assert!(matches!(program[1], Stmt::ForNumeric { .. }));
        let i = run_optimized(src);
        assert_eq!(i.env.get("v"), Some(Value::Int(6)));
    }

    #[test]
    fn return_inside_unrolled_loop_works() {
        let src = "local function f()\n    for i = 1, 3 do\n        if i == 2 then\n            return i\n        end\n    end\n    return 0\nend\npub local v = f()";
        let i = run_optimized(src);
        assert_eq!(i.env.get("v"), Some(Value::Int(2)));
    }

    #[test]
    fn shadowing_inside_unrolled_body_is_preserved() {
        let src = "pub local x = 0\nfor i = 1, 3 do\n    local i = i * 10\n    x += i\nend";
        let i = run_optimized(src);
        assert_eq!(i.env.get("x"), Some(Value::Int(60)));
    }

    #[test]
    fn nested_constant_loops_unroll() {
        let src = "pub local x = 0\nfor i = 1, 3 do\n    for j = 1, 2 do\n        x += i * j\n    end\nend";
        let i = run_optimized(src);
        assert_eq!(i.env.get("x"), Some(Value::Int(18)));
    }
}
