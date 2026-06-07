
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::thread::JoinHandle;

use super::env::ScopeRef;
use super::interp::Interpreter;
use super::value::Value;

struct Xfer<T>(T);
unsafe impl<T> Send for Xfer<T> {}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Suspended,
    Running,
    Dead,
}

enum Resume {
    Go(Vec<Value>),
    Kill,
}

enum Yielded {
    Yield(Vec<Value>),
    Return(Vec<Value>),
    Fail(String),
}

struct Yielder {
    to_rx: Receiver<Xfer<Resume>>,
    from_tx: Sender<Xfer<Yielded>>,
}

thread_local! {

    static YIELDER: RefCell<Option<Yielder>> = const { RefCell::new(None) };
}

pub struct CoroState {
    to_coro: Option<Sender<Xfer<Resume>>>,
    from_coro: Receiver<Xfer<Yielded>>,
    handle: Option<JoinHandle<()>>,
    status: Status,
}

impl CoroState {
    pub fn status_str(&self) -> &'static str {
        match self.status {
            Status::Suspended => "suspended",
            Status::Running => "running",
            Status::Dead => "dead",
        }
    }
}

impl Drop for CoroState {
    fn drop(&mut self) {

        if let Some(tx) = self.to_coro.take() {
            let _ = tx.send(Xfer(Resume::Kill));
        }
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

type StartPayload = (Value, ScopeRef, Receiver<Xfer<Resume>>, Sender<Xfer<Yielded>>);

pub fn create(func: Value, global: ScopeRef) -> CoroState {
    let (to_tx, to_rx) = channel::<Xfer<Resume>>();
    let (from_tx, from_rx) = channel::<Xfer<Yielded>>();
    let payload = Xfer((func, global, to_rx, from_tx));
    let handle = std::thread::Builder::new()
        .name("luar-coroutine".into())
        .stack_size(16 * 1024 * 1024)
        .spawn(move || coro_main(payload))
        .expect("failed to spawn coroutine thread");
    CoroState { to_coro: Some(to_tx), from_coro: from_rx, handle: Some(handle), status: Status::Suspended }
}

fn coro_main(payload: Xfer<StartPayload>) {
    let Xfer((func, global, to_rx, from_tx)) = payload;

    let args = match to_rx.recv() {
        Ok(Xfer(Resume::Go(a))) => a,
        _ => return,
    };

    let from_tx_final = from_tx.clone();
    YIELDER.with(|cell| *cell.borrow_mut() = Some(Yielder { to_rx, from_tx }));

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut interp = Interpreter::with_shared_global(global);
        interp.call(&func, args)
    }));

    YIELDER.with(|cell| *cell.borrow_mut() = None);

    let message = match result {
        Ok(Ok(values)) => Yielded::Return(values),
        Ok(Err(e)) => Yielded::Fail(e.0),
        Err(_) => Yielded::Fail("coroutine panicked".into()),
    };
    let _ = from_tx_final.send(Xfer(message));
}

pub fn do_yield(values: Vec<Value>) -> Result<Vec<Value>, String> {
    YIELDER.with(|cell| {
        let guard = cell.borrow();
        let yielder = guard
            .as_ref()
            .ok_or_else(|| "attempt to yield from outside a coroutine".to_string())?;
        yielder
            .from_tx
            .send(Xfer(Yielded::Yield(values)))
            .map_err(|_| "coroutine: the resumer is gone".to_string())?;
        match yielder.to_rx.recv() {
            Ok(Xfer(Resume::Go(a))) => Ok(a),

            _ => Err("__luar_coroutine_closed__".to_string()),
        }
    })
}

pub fn resume(state: &Rc<RefCell<CoroState>>, args: Vec<Value>) -> Vec<Value> {
    match state.borrow().status {
        Status::Dead => return vec![Value::Bool(false), Value::str("cannot resume dead coroutine")],
        Status::Running => {
            return vec![Value::Bool(false), Value::str("cannot resume non-suspended coroutine")]
        }
        Status::Suspended => {}
    }

    state.borrow_mut().status = Status::Running;

    let received = {
        let st = state.borrow();
        if let Some(tx) = &st.to_coro {
            if tx.send(Xfer(Resume::Go(args))).is_err() {
                drop(st);
                state.borrow_mut().status = Status::Dead;
                return vec![Value::Bool(false), Value::str("coroutine is gone")];
            }
        }
        st.from_coro.recv()
    };

    match received {
        Ok(Xfer(Yielded::Yield(values))) => {
            state.borrow_mut().status = Status::Suspended;
            prepend_true(values)
        }
        Ok(Xfer(Yielded::Return(values))) => {
            state.borrow_mut().status = Status::Dead;
            prepend_true(values)
        }
        Ok(Xfer(Yielded::Fail(e))) => {
            state.borrow_mut().status = Status::Dead;
            vec![Value::Bool(false), Value::str(e)]
        }
        Err(_) => {
            state.borrow_mut().status = Status::Dead;
            vec![Value::Bool(false), Value::str("coroutine ended unexpectedly")]
        }
    }
}

fn prepend_true(values: Vec<Value>) -> Vec<Value> {
    let mut out = Vec::with_capacity(values.len() + 1);
    out.push(Value::Bool(true));
    out.extend(values);
    out
}

pub fn close(state: &Rc<RefCell<CoroState>>) -> bool {
    let (tx, handle) = {
        let mut st = state.borrow_mut();
        match st.status {
            Status::Running => return false,
            _ => {
                st.status = Status::Dead;
                (st.to_coro.take(), st.handle.take())
            }
        }
    };
    if let Some(tx) = tx {
        let _ = tx.send(Xfer(Resume::Kill));
    }
    if let Some(handle) = handle {
        let _ = handle.join();
    }
    true
}
