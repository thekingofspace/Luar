
use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use super::env::ScopeRef;
use super::gil::{current_family, Family};
use super::interp::Interpreter;
use super::value::Value;

const GRACE: Duration = Duration::from_millis(10);

struct Xfer<T>(T);

unsafe impl Send for Xfer<Resume> {}
unsafe impl Send for Xfer<Yielded> {}
unsafe impl Send for Xfer<StartPayload> {}
unsafe impl Send for Xfer<(Weak<RefCell<CoroState>>, Arc<Family>)> {}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Suspended,
    Running,
    Background,
    Detached,
    Dead,
}

enum Resume {
    Go(Vec<Value>, Weak<RefCell<CoroState>>),
    Kill,
}

enum Yielded {
    Yield(Vec<Value>),
    Return(Vec<Value>),
    Fail(String),
    Detach,
}

type HubMsg = Xfer<(Weak<RefCell<CoroState>>, Arc<Family>)>;

struct Yielder {
    to_rx: Receiver<Xfer<Resume>>,
    from_tx: Sender<Xfer<Yielded>>,
    hub_tx: Sender<HubMsg>,
    backgrounded: Arc<AtomicBool>,
    family: Arc<Family>,
}

struct OwnerHub {
    tx: Sender<HubMsg>,
    rx: Receiver<HubMsg>,
}

thread_local! {

    static YIELDER: RefCell<Option<Yielder>> = const { RefCell::new(None) };

    static SELF_CORO: RefCell<Option<Weak<RefCell<CoroState>>>> = const { RefCell::new(None) };

    static MAIN_CORO: RefCell<Option<Rc<RefCell<CoroState>>>> = const { RefCell::new(None) };

    static HUB: OwnerHub = {
        let (tx, rx) = channel();
        OwnerHub { tx, rx }
    };

    static KEEP: RefCell<Vec<Rc<RefCell<CoroState>>>> = const { RefCell::new(Vec::new()) };

    static COUNTED: Cell<bool> = const { Cell::new(false) };
}

fn keep_alive(state: &Rc<RefCell<CoroState>>) {
    KEEP.with(|k| {
        let mut held = k.borrow_mut();
        if !held.iter().any(|rc| Rc::ptr_eq(rc, state)) {
            held.push(state.clone());
        }
    });
}

fn release_if_settled(state: &Rc<RefCell<CoroState>>) {
    let status = state.borrow().status;
    if status == Status::Detached || status == Status::Background {
        return;
    }
    KEEP.with(|k| k.borrow_mut().retain(|rc| !Rc::ptr_eq(rc, state)));
}

pub struct CoroState {
    to_coro: Option<Sender<Xfer<Resume>>>,
    from_coro: Arc<Receiver<Xfer<Yielded>>>,
    handle: Option<JoinHandle<()>>,
    status: Status,
    backgrounded: Arc<AtomicBool>,
    finished: Arc<AtomicBool>,
    family: Arc<Family>,
}

impl CoroState {
    pub fn status_str(&self) -> &'static str {
        match self.status {
            Status::Suspended => "suspended",
            Status::Running | Status::Background => "running",
            Status::Detached => "waiting",
            Status::Dead => "dead",
        }
    }

    fn main() -> CoroState {
        let (_tx, from_coro) = channel::<Xfer<Yielded>>();
        CoroState {
            to_coro: None,
            from_coro: Arc::new(from_coro),
            handle: None,
            status: Status::Running,
            backgrounded: Arc::new(AtomicBool::new(false)),
            finished: Arc::new(AtomicBool::new(false)),
            family: Arc::new(Family::new()),
        }
    }
}

impl Drop for CoroState {
    fn drop(&mut self) {

        if let Some(tx) = self.to_coro.take() {
            let _ = tx.send(Xfer(Resume::Kill));
        }
        if let Some(handle) = self.handle.take() {
            if self.status == Status::Background && !self.finished.load(Ordering::SeqCst) {
                drop(handle);
            } else {
                let _ = handle.join();
            }
        }
    }
}

type StartPayload = (
    Value,
    ScopeRef,
    Receiver<Xfer<Resume>>,
    Sender<Xfer<Yielded>>,
    Sender<HubMsg>,
    Arc<AtomicBool>,
    Arc<AtomicBool>,
    Arc<Family>,
);

pub fn create(func: Value, global: ScopeRef, family: Arc<Family>) -> CoroState {
    let (to_tx, to_rx) = channel::<Xfer<Resume>>();
    let (from_tx, from_rx) = channel::<Xfer<Yielded>>();
    let hub_tx = HUB.with(|h| h.tx.clone());
    let backgrounded = Arc::new(AtomicBool::new(false));
    let finished = Arc::new(AtomicBool::new(false));
    let payload = Xfer((
        func,
        global,
        to_rx,
        from_tx,
        hub_tx,
        backgrounded.clone(),
        finished.clone(),
        family.clone(),
    ));
    let handle = std::thread::Builder::new()
        .name("luar-coroutine".into())
        .stack_size(16 * 1024 * 1024)
        .spawn(move || coro_main(payload))
        .expect("failed to spawn coroutine thread");
    CoroState {
        to_coro: Some(to_tx),
        from_coro: Arc::new(from_rx),
        handle: Some(handle),
        status: Status::Suspended,
        backgrounded,
        finished,
        family,
    }
}

fn coro_main(payload: Xfer<StartPayload>) {
    let Xfer((func, global, to_rx, from_tx, hub_tx, backgrounded, finished, family)) = payload;

    let (args, self_weak) = match to_rx.recv() {
        Ok(Xfer(Resume::Go(a, w))) => (a, w),
        _ => return,
    };

    family.threads.fetch_add(1, Ordering::SeqCst);
    COUNTED.set(true);
    family.acquire();

    let from_tx_final = from_tx.clone();
    let hub_tx_final = hub_tx.clone();
    let backgrounded_final = backgrounded.clone();
    let family_final = family.clone();
    YIELDER.with(|cell| {
        *cell.borrow_mut() = Some(Yielder {
            to_rx,
            from_tx,
            hub_tx,
            backgrounded,
            family: family.clone(),
        })
    });
    SELF_CORO.with(|cell| *cell.borrow_mut() = Some(self_weak));

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut interp = Interpreter::with_shared_global(global, family.clone());
        interp.call(&func, args)
    }));

    let weak_for_note = SELF_CORO.with(|cell| cell.borrow().clone());
    YIELDER.with(|cell| *cell.borrow_mut() = None);
    SELF_CORO.with(|cell| *cell.borrow_mut() = None);

    let message = match result {
        Ok(Ok(values)) => Yielded::Return(values),
        Ok(Err(e)) => Yielded::Fail(e.0),
        Err(_) => Yielded::Fail("coroutine panicked".into()),
    };
    let _ = from_tx_final.send(Xfer(message));
    finished.store(true, Ordering::SeqCst);
    if backgrounded_final.load(Ordering::SeqCst) {
        if let Some(w) = weak_for_note {
            let _ = hub_tx_final.send(Xfer((w, family_final.clone())));
        }
    }
    if COUNTED.get() {
        COUNTED.set(false);
        family_final.threads.fetch_sub(1, Ordering::SeqCst);
    }
    if family_final.holds() {
        family_final.unlock_all();
    }
}

pub fn do_yield(values: Vec<Value>) -> Result<Vec<Value>, String> {
    let ctx = YIELDER.with(|cell| {
        cell.borrow()
            .as_ref()
            .map(|y| (y.family.clone(), y.backgrounded.clone()))
    });
    let Some((family, backgrounded)) = ctx else {
        return Err("attempt to yield from outside a coroutine".to_string());
    };
    YIELDER.with(|cell| {
        let guard = cell.borrow();
        guard
            .as_ref()
            .unwrap()
            .from_tx
            .send(Xfer(Yielded::Yield(values)))
            .map_err(|_| "coroutine: the resumer is gone".to_string())
    })?;
    if backgrounded.load(Ordering::SeqCst) {
        let weak = SELF_CORO.with(|cell| cell.borrow().clone());
        if let Some(w) = weak {
            YIELDER.with(|cell| {
                let guard = cell.borrow();
                let _ = guard.as_ref().unwrap().hub_tx.send(Xfer((w, family.clone())));
            });
        }
    }
    COUNTED.set(false);
    family.threads.fetch_sub(1, Ordering::SeqCst);
    let saved = family.unlock_all();
    let got = YIELDER.with(|cell| {
        let guard = cell.borrow();
        match guard.as_ref().unwrap().to_rx.recv() {
            Ok(Xfer(Resume::Go(a, _))) => Ok(a),

            _ => Err("__luar_coroutine_closed__".to_string()),
        }
    });
    match got {
        Ok(args) => {
            family.threads.fetch_add(1, Ordering::SeqCst);
            COUNTED.set(true);
            family.relock(saved);
            Ok(args)
        }
        Err(e) => Err(e),
    }
}

pub fn blocking<T>(f: impl FnOnce() -> T) -> Result<T, String> {
    let ctx = YIELDER.with(|cell| {
        cell.borrow()
            .as_ref()
            .map(|y| (y.family.clone(), y.from_tx.clone(), y.hub_tx.clone()))
    });
    let Some((family, from_tx, hub_tx)) = ctx else {
        let out = match current_family() {
            Some(family) => {
                let saved = family.unlock_all();
                let out = f();
                family.relock(saved);
                out
            }
            None => f(),
        };
        pump_ready();
        return Ok(out);
    };
    from_tx
        .send(Xfer(Yielded::Detach))
        .map_err(|_| "coroutine: the resumer is gone".to_string())?;
    let weak = SELF_CORO.with(|cell| cell.borrow_mut().take());
    COUNTED.set(false);
    family.threads.fetch_sub(1, Ordering::SeqCst);
    let saved = family.unlock_all();
    let out = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
    if let Some(w) = weak {
        let _ = hub_tx.send(Xfer((w, family.clone())));
    }
    let resumed = YIELDER.with(|cell| {
        let guard = cell.borrow();
        match guard.as_ref().unwrap().to_rx.recv() {
            Ok(Xfer(Resume::Go(_, w))) => Ok(w),
            _ => Err("__luar_coroutine_closed__".to_string()),
        }
    });
    match resumed {
        Ok(w) => {
            family.threads.fetch_add(1, Ordering::SeqCst);
            COUNTED.set(true);
            family.relock(saved);
            SELF_CORO.with(|cell| *cell.borrow_mut() = Some(w));
            out.map_err(|_| "blocking call panicked".to_string())
        }
        Err(e) => Err(e),
    }
}

pub fn running() -> (Value, bool) {
    if let Some(rc) = SELF_CORO.with(|cell| cell.borrow().as_ref().and_then(Weak::upgrade)) {
        return (Value::Coroutine(rc), false);
    }
    MAIN_CORO.with(|m| {
        let mut slot = m.borrow_mut();
        let rc = slot.get_or_insert_with(|| Rc::new(RefCell::new(CoroState::main())));
        (Value::Coroutine(rc.clone()), true)
    })
}

struct Wires {
    family: Arc<Family>,
    backgrounded: Arc<AtomicBool>,
    to_coro: Option<Sender<Xfer<Resume>>>,
    from_coro: Arc<Receiver<Xfer<Yielded>>>,
}

fn wires(state: &Rc<RefCell<CoroState>>) -> Wires {
    let st = state.borrow();
    Wires {
        family: st.family.clone(),
        backgrounded: st.backgrounded.clone(),
        to_coro: st.to_coro.clone(),
        from_coro: st.from_coro.clone(),
    }
}

pub fn resume(state: &Rc<RefCell<CoroState>>, args: Vec<Value>) -> Vec<Value> {
    match state.borrow().status {
        Status::Dead => return vec![Value::Bool(false), Value::str("cannot resume dead coroutine")],
        Status::Running => {
            return vec![Value::Bool(false), Value::str("cannot resume non-suspended coroutine")]
        }
        Status::Detached => {
            return vec![Value::Bool(false), Value::str("cannot resume waiting coroutine")]
        }
        Status::Background => {
            let w = wires(state);
            state.borrow_mut().status = Status::Running;
            let saved = w.family.unlock_all();
            let received = w.from_coro.recv();
            w.family.relock(saved);
            let out = settle(state, received.map_err(|_| ()));
            release_if_settled(state);
            return out;
        }
        Status::Suspended => {}
    }

    state.borrow_mut().status = Status::Running;
    let w = wires(state);
    w.backgrounded.store(false, Ordering::SeqCst);

    if let Some(tx) = &w.to_coro {
        if tx.send(Xfer(Resume::Go(args, Rc::downgrade(state)))).is_err() {
            state.borrow_mut().status = Status::Dead;
            return vec![Value::Bool(false), Value::str("coroutine is gone")];
        }
    }
    let saved = w.family.unlock_all();
    let received = w.from_coro.recv_timeout(GRACE);
    w.family.relock(saved);

    match received {
        Ok(msg) => {
            let out = settle(state, Ok(msg));
            release_if_settled(state);
            out
        }
        Err(RecvTimeoutError::Timeout) => {
            w.backgrounded.store(true, Ordering::SeqCst);
            match w.from_coro.try_recv() {
                Ok(msg) => {
                    let out = settle(state, Ok(msg));
                    release_if_settled(state);
                    out
                }
                Err(_) => {
                    state.borrow_mut().status = Status::Background;
                    keep_alive(state);
                    vec![Value::Bool(true)]
                }
            }
        }
        Err(RecvTimeoutError::Disconnected) => {
            state.borrow_mut().status = Status::Dead;
            vec![Value::Bool(false), Value::str("coroutine ended unexpectedly")]
        }
    }
}

fn settle(
    state: &Rc<RefCell<CoroState>>,
    received: Result<Xfer<Yielded>, ()>,
) -> Vec<Value> {
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
        Ok(Xfer(Yielded::Detach)) => {
            state.borrow_mut().status = Status::Detached;
            keep_alive(state);
            vec![Value::Bool(true)]
        }
        Err(()) => {
            state.borrow_mut().status = Status::Dead;
            vec![Value::Bool(false), Value::str("coroutine ended unexpectedly")]
        }
    }
}

pub fn run_pending() {
    if SELF_CORO.with(|cell| cell.borrow().is_some()) {
        return;
    }
    loop {
        if KEEP.with(|k| k.borrow().is_empty()) {
            return;
        }
        let msg = HUB.with(|h| h.rx.recv());
        let Ok(Xfer((weak, family))) = msg else { return };
        family.acquire();
        if let Some(rc) = weak.upgrade() {
            reattach(&rc);
        }
        family.release();
    }
}

pub fn pump_ready() {
    if SELF_CORO.with(|cell| cell.borrow().is_some()) {
        return;
    }
    loop {
        let msg = HUB.with(|h| h.rx.try_recv());
        let Ok(Xfer((weak, family))) = msg else { return };
        family.acquire();
        if let Some(rc) = weak.upgrade() {
            reattach(&rc);
        }
        family.release();
    }
}

fn reattach(state: &Rc<RefCell<CoroState>>) {
    let w = wires(state);
    if let Ok(msg) = w.from_coro.try_recv() {
        let out = settle(state, Ok(msg));
        release_if_settled(state);
        report_background_failure(&out);
        return;
    }
    if state.borrow().status != Status::Detached {
        return;
    }
    state.borrow_mut().status = Status::Running;
    w.backgrounded.store(true, Ordering::SeqCst);
    if let Some(tx) = &w.to_coro {
        if tx.send(Xfer(Resume::Go(Vec::new(), Rc::downgrade(state)))).is_err() {
            state.borrow_mut().status = Status::Dead;
            release_if_settled(state);
            return;
        }
    }
    let saved = w.family.unlock_all();
    let received = w.from_coro.recv_timeout(GRACE);
    w.family.relock(saved);
    match received {
        Ok(msg) => {
            let out = settle(state, Ok(msg));
            release_if_settled(state);
            report_background_failure(&out);
        }
        Err(RecvTimeoutError::Timeout) => match w.from_coro.try_recv() {
            Ok(msg) => {
                let out = settle(state, Ok(msg));
                release_if_settled(state);
                report_background_failure(&out);
            }
            Err(_) => {
                state.borrow_mut().status = Status::Background;
                keep_alive(state);
            }
        },
        Err(RecvTimeoutError::Disconnected) => {
            state.borrow_mut().status = Status::Dead;
            release_if_settled(state);
        }
    }
}

fn report_background_failure(out: &[Value]) {
    if let (Some(Value::Bool(false)), Some(msg)) = (out.first(), out.get(1)) {
        if msg.as_str() != Some("__luar_coroutine_closed__") {
            eprintln!("error in background coroutine: {msg}");
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
            Status::Running | Status::Background => return false,
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
    KEEP.with(|k| k.borrow_mut().retain(|rc| !Rc::ptr_eq(rc, state)));
    true
}
