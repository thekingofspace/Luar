

const BENCHES: &[(&str, &str)] = &[
    ("fib_recursive", "local function fib(n)\n  if n < 2 then\n    return n\n  end\n  return fib(n - 1) + fib(n - 2)\nend\npub local out = fib(24)"),
    ("loop_arith", "local acc = 0\nfor i = 1, 2000000 do\n  acc = acc + i * 2 - 1\nend\npub local out = acc"),
    ("table_fill_read", "local t = {}\nfor i = 1, 200000 do\n  t[i] = i * 2\nend\nlocal s = 0\nfor i = 1, 200000 do\n  s = s + t[i]\nend\npub local out = s"),
    ("var_lookup", "local a = 1\nlocal b = 2\nlocal c = 3\nlocal d = 4\nlocal acc = 0\nfor i = 1, 1000000 do\n  acc = acc + a + b + c + d\nend\npub local out = acc"),
];

fn main() {
    let filter = std::env::args().nth(1);
    for (name, src) in BENCHES {
        if let Some(f) = &filter {
            if f != name {
                continue;
            }
        }
        match luar::compile_source(src) {
            Ok(program) => {
                let mut best = None;
                for _ in 0..3 {
                    let start = std::time::Instant::now();
                    match luar::execute(program.clone()) {
                        Ok(_) => {}
                        Err(e) => {
                            println!("{name:18} vm runtime error: {e}");
                            return;
                        }
                    }
                    let elapsed = start.elapsed();
                    if best.map(|b| elapsed < b).unwrap_or(true) {
                        best = Some(elapsed);
                    }
                }
                println!("{name:18} {:>9.2?}", best.unwrap());
            }
            Err(e) => println!("{name:18} compile error: {e}"),
        }
    }
}

