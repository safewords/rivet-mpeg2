//! A persistent set of worker threads that run borrowed closures: the
//! decoder's slice threads and the encoder's row threads, started once
//! instead of once per picture.
//!
//! [`Pool::run`] hands a closure to some of the workers, runs it on the
//! calling thread too, and returns only when every worker has finished it —
//! like `std::thread::scope`, which is why the closure may borrow from the
//! caller's stack. The work itself is shared out by the closure (the
//! callers take items from an atomic counter or a locked list), so which
//! thread does which item never affects a result.

use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::JoinHandle;

/// The closure being run, its lifetime erased (see `Pool::run`).
#[derive(Clone, Copy)]
struct Job(*const (dyn Fn() + Sync));

// SAFETY: a Job is only dereferenced by workers between Pool::run publishing
// it and Pool::run observing that every worker is done with it; the closure
// it points to is Sync, so calling it from several threads is allowed.
unsafe impl Send for Job {}

struct State {
    job: Option<Job>,
    /// Incremented for every run, so a worker runs each job once.
    generation: u64,
    /// Workers (by index) that take part in the current run.
    participants: usize,
    /// Workers that have not finished the current run.
    busy: usize,
    /// A worker's panic, re-raised by `run`.
    panic: Option<Box<dyn std::any::Any + Send>>,
    shutdown: bool,
}

struct Shared {
    state: Mutex<State>,
    work: Condvar,
    done: Condvar,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        // A panic is caught before it could poison the lock while held.
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// Worker threads waiting for work.
pub(crate) struct Pool {
    shared: Arc<Shared>,
    workers: Vec<JoinHandle<()>>,
}

impl Pool {
    /// A pool of `n` workers (the calling thread makes one more).
    pub(crate) fn new(n: usize) -> Pool {
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                job: None,
                generation: 0,
                participants: 0,
                busy: 0,
                panic: None,
                shutdown: false,
            }),
            work: Condvar::new(),
            done: Condvar::new(),
        });
        let workers = (0..n)
            .map(|i| {
                let shared = Arc::clone(&shared);
                std::thread::Builder::new()
                    .name(format!("mpeg2-{i}"))
                    .spawn(move || worker(&shared, i))
                    .expect("spawn a worker thread")
            })
            .collect();
        Pool { shared, workers }
    }

    /// Workers in the pool.
    pub(crate) fn size(&self) -> usize {
        self.workers.len()
    }

    /// Runs `f` on the calling thread and on `helpers` of the workers (at
    /// most all of them), returning when all have finished. A panic in any
    /// of them is re-raised here, after all have finished.
    pub(crate) fn run(&self, helpers: usize, f: &(dyn Fn() + Sync)) {
        let helpers = helpers.min(self.workers.len());
        if helpers == 0 {
            f();
            return;
        }
        // SAFETY: the transmute only erases the lifetime of `f`. Workers use
        // the pointer only until they report the run finished, and this
        // function does not return — nor unwind: the guard below waits in
        // its drop — before every worker taking part has reported it. So `f`
        // outlives every use.
        let job = Job(unsafe {
            std::mem::transmute::<*const (dyn Fn() + Sync + '_), *const (dyn Fn() + Sync)>(f)
        });
        {
            let mut st = self.shared.lock();
            st.job = Some(job);
            st.generation += 1;
            st.participants = helpers;
            st.busy = helpers;
            st.panic = None;
        }
        self.shared.work.notify_all();

        struct Wait<'a>(&'a Shared);
        impl Drop for Wait<'_> {
            fn drop(&mut self) {
                let mut st = self.0.lock();
                while st.busy > 0 {
                    st = self.0.done.wait(st).unwrap_or_else(|e| e.into_inner());
                }
                st.job = None;
            }
        }
        let wait = Wait(&self.shared);
        f();
        drop(wait);
        if let Some(p) = self.shared.lock().panic.take() {
            resume_unwind(p);
        }
    }
}

fn worker(shared: &Shared, index: usize) {
    let mut seen = 0u64;
    loop {
        let job = {
            let mut st = shared.lock();
            loop {
                if st.shutdown {
                    return;
                }
                if st.generation != seen {
                    seen = st.generation;
                    if index < st.participants {
                        break st.job.expect("a job with its generation");
                    }
                }
                st = shared.work.wait(st).unwrap_or_else(|e| e.into_inner());
            }
        };
        // SAFETY: see Pool::run — the closure lives until this worker has
        // decremented `busy` below.
        let result = catch_unwind(AssertUnwindSafe(|| unsafe { (*job.0)() }));
        let mut st = shared.lock();
        if let Err(p) = result {
            st.panic.get_or_insert(p);
        }
        st.busy -= 1;
        if st.busy == 0 {
            shared.done.notify_all();
        }
    }
}

impl Drop for Pool {
    fn drop(&mut self) {
        self.shared.lock().shutdown = true;
        self.shared.work.notify_all();
        for w in self.workers.drain(..) {
            let _ = w.join();
        }
    }
}

/// A pool of `threads − 1` workers in `slot`, made (or remade) when the
/// count changes; `None` for one thread.
pub(crate) fn ensure(slot: &mut Option<Pool>, threads: usize) -> Option<&Pool> {
    let want = threads.saturating_sub(1);
    if want == 0 {
        return None;
    }
    if slot.as_ref().is_none_or(|p| p.size() != want) {
        *slot = Some(Pool::new(want));
    }
    slot.as_ref()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn every_participant_runs_each_job_once() {
        let pool = Pool::new(5);
        for helpers in [0, 1, 3, 5, 9] {
            for _ in 0..200 {
                let n = AtomicUsize::new(0);
                pool.run(helpers, &|| {
                    n.fetch_add(1, Ordering::Relaxed);
                });
                assert_eq!(n.load(Ordering::Relaxed), 1 + helpers.min(5));
            }
        }
    }

    #[test]
    fn borrowed_work_is_shared_out_and_finished() {
        let pool = Pool::new(7);
        let items: Vec<u64> = (0..10_000).collect();
        let next = AtomicUsize::new(0);
        let sum = std::sync::atomic::AtomicU64::new(0);
        pool.run(7, &|| {
            loop {
                let i = next.fetch_add(1, Ordering::Relaxed);
                let Some(v) = items.get(i) else { break };
                sum.fetch_add(*v, Ordering::Relaxed);
            }
        });
        assert_eq!(sum.into_inner(), 10_000 * 9_999 / 2);
    }

    #[test]
    fn a_worker_panic_reaches_the_caller_and_the_pool_survives() {
        let pool = Pool::new(3);
        let first = AtomicUsize::new(0);
        let r = catch_unwind(AssertUnwindSafe(|| {
            pool.run(3, &|| {
                if first.fetch_add(1, Ordering::Relaxed) == 2 {
                    panic!("one job panics");
                }
            })
        }));
        assert!(r.is_err());
        let n = AtomicUsize::new(0);
        pool.run(3, &|| {
            n.fetch_add(1, Ordering::Relaxed);
        });
        assert_eq!(n.into_inner(), 4);
    }
}
