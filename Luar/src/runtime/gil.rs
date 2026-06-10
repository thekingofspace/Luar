use std::cell::RefCell;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::ThreadId;

pub struct Family {
    state: Mutex<GilState>,
    cv: Condvar,
    next_ticket: AtomicU64,
    pub threads: AtomicUsize,
}

struct GilState {
    serving: u64,
    owner: Option<ThreadId>,
    depth: u32,
}

impl Family {
    pub fn new() -> Family {
        Family {
            state: Mutex::new(GilState { serving: 0, owner: None, depth: 0 }),
            cv: Condvar::new(),
            next_ticket: AtomicU64::new(0),
            threads: AtomicUsize::new(0),
        }
    }

    pub fn acquire(&self) {
        let me = std::thread::current().id();
        let mut st = self.state.lock().unwrap();
        if st.owner == Some(me) {
            st.depth += 1;
            return;
        }
        let ticket = self.next_ticket.fetch_add(1, Ordering::SeqCst);
        while st.owner.is_some() || st.serving != ticket {
            st = self.cv.wait(st).unwrap();
        }
        st.owner = Some(me);
        st.depth = 1;
    }

    pub fn release(&self) {
        let mut st = self.state.lock().unwrap();
        if st.owner != Some(std::thread::current().id()) {
            return;
        }
        st.depth -= 1;
        if st.depth == 0 {
            st.owner = None;
            st.serving += 1;
            drop(st);
            self.cv.notify_all();
        }
    }

    pub fn holds(&self) -> bool {
        self.state.lock().unwrap().owner == Some(std::thread::current().id())
    }

    pub fn unlock_all(&self) -> Option<u32> {
        let me = std::thread::current().id();
        let mut st = self.state.lock().unwrap();
        if st.owner != Some(me) {
            return None;
        }
        let depth = st.depth;
        st.owner = None;
        st.depth = 0;
        st.serving += 1;
        drop(st);
        self.cv.notify_all();
        Some(depth)
    }

    pub fn relock(&self, saved: Option<u32>) {
        let Some(depth) = saved else {
            return;
        };
        self.acquire();
        if depth > 1 {
            let mut st = self.state.lock().unwrap();
            st.depth = depth;
        }
    }

    pub fn preempt_point(&self) {
        if self.threads.load(Ordering::Relaxed) <= 1 {
            return;
        }
        let saved = self.unlock_all();
        if saved.is_none() {
            return;
        }
        std::thread::yield_now();
        self.relock(saved);
    }
}

thread_local! {
    static FAMILY_STACK: RefCell<Vec<Arc<Family>>> = const { RefCell::new(Vec::new()) };
}

pub fn current_family() -> Option<Arc<Family>> {
    FAMILY_STACK.with(|s| s.borrow().last().cloned())
}

pub struct FamilyScope {
    family: Arc<Family>,
    counted: bool,
}

impl FamilyScope {
    pub fn enter(family: &Arc<Family>) -> FamilyScope {
        let counted =
            FAMILY_STACK.with(|s| !s.borrow().iter().any(|f| Arc::ptr_eq(f, family)));
        family.acquire();
        if counted {
            family.threads.fetch_add(1, Ordering::SeqCst);
        }
        FAMILY_STACK.with(|s| s.borrow_mut().push(family.clone()));
        FamilyScope { family: family.clone(), counted }
    }
}

impl Drop for FamilyScope {
    fn drop(&mut self) {
        FAMILY_STACK.with(|s| {
            s.borrow_mut().pop();
        });
        if self.counted {
            self.family.threads.fetch_sub(1, Ordering::SeqCst);
        }
        self.family.release();
    }
}
