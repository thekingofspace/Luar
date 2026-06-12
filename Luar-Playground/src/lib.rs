
use std::alloc::{alloc as raw_alloc, dealloc as raw_dealloc, Layout};

unsafe extern "C" {
    fn js_now_ms() -> f64;
}

fn host_now_ms() -> f64 {
    unsafe { js_now_ms() }
}

#[unsafe(no_mangle)]
pub extern "C" fn lp_alloc(len: usize) -> *mut u8 {
    let layout = Layout::from_size_align(len.max(1), 1).unwrap();
    unsafe { raw_alloc(layout) }
}

#[unsafe(no_mangle)]
pub extern "C" fn lp_dealloc(ptr: *mut u8, len: usize) {
    let layout = Layout::from_size_align(len.max(1), 1).unwrap();
    unsafe { raw_dealloc(ptr, layout) }
}

fn pack_result(status: u8, body: &str) -> *mut u8 {
    let bytes = body.as_bytes();
    let total = 5 + bytes.len();
    let out = lp_alloc(total);
    let len32 = bytes.len() as u32;
    unsafe {
        std::ptr::copy_nonoverlapping(len32.to_le_bytes().as_ptr(), out, 4);
        *out.add(4) = status;
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), out.add(5), bytes.len());
    }
    out
}

#[unsafe(no_mangle)]
pub extern "C" fn lp_run(ptr: *const u8, len: usize) -> *mut u8 {
    let source = unsafe {
        let slice = std::slice::from_raw_parts(ptr, len);
        match std::str::from_utf8(slice) {
            Ok(s) => s.to_string(),
            Err(_) => return pack_result(1, "playground: source is not valid UTF-8"),
        }
    };
    luar::set_time_source(host_now_ms);
    luar::capture_output(true);
    let outcome = std::panic::catch_unwind(|| luar::eval_source(&source));
    let printed = luar::take_captured_output();
    luar::capture_output(false);
    match outcome {
        Ok(Ok(_)) => pack_result(0, &printed),
        Ok(Err(e)) => {
            let mut body = printed;
            if !body.is_empty() && !body.ends_with('\n') {
                body.push('\n');
            }
            body.push_str(&format!("error: {e}"));
            pack_result(1, &body)
        }
        Err(_) => {
            let mut body = printed;
            if !body.is_empty() && !body.ends_with('\n') {
                body.push('\n');
            }
            body.push_str("error: the interpreter panicked");
            pack_result(1, &body)
        }
    }
}
