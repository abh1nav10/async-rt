#![allow(dead_code)]

include!(concat!(env!("OUT_DIR"), "/available.rs"));

use crate::executor::{JoinHandle, Metadata, Task};
use flume::{Receiver, Sender};
use std::cell::UnsafeCell;
use std::io::ErrorKind;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicPtr, AtomicUsize, Ordering};
use std::task::{Context, Poll};
use std::thread::JoinHandle as Join;

pub(crate) struct Carrier {
    data: *const (),
}

unsafe impl Send for Carrier {}

impl Carrier {
    pub(crate) fn new(data: *const ()) -> Self {
        Self { data }
    }
}

pub struct Runtime {
    low: usize,
    high: usize,
}

impl Runtime {
    pub fn start() -> RuntimeInstance {
        // We now use build scripts to get the availabe parallelism on the machine instead of doing
        // it here!
        let flag = Arc::new(AtomicBool::new(true));
        let (low_sender, low_receiver) = flume::unbounded::<Carrier>();
        let (high_sender, high_receiver) = flume::unbounded::<Carrier>();
        let arc_low_receiver = Arc::new(low_receiver);
        let arc_high_receiver = Arc::new(high_receiver);
        let low = Runtime::spawn_low_threads(
            Arc::clone(&flag),
            Arc::clone(&arc_low_receiver),
            Arc::clone(&arc_high_receiver),
        );

        let high = Runtime::spawn_high_threads(
            AVAILABLE_PARALLELISM - 1,
            Arc::clone(&flag),
            Arc::clone(&arc_low_receiver),
            Arc::clone(&arc_high_receiver),
        );

        let worker_meta = WorkerMeta {
            wakers: high.1,
            blocking: low.1,
            capacity: AVAILABLE_PARALLELISM - 1,
        };
        RuntimeInstance {
            low_handle: Some(low.0),
            high_handles: high.0,
            low_sender: Arc::new(low_sender),
            high_sender: Arc::new(high_sender),
            flag,
            worker_meta,
        }
    }

    pub fn builder() -> RuntimeBuilder {
        RuntimeBuilder {
            high_threads: None,
            low_threads: None,
            allowed: AVAILABLE_PARALLELISM,
        }
    }

    pub fn start_from_config(self) -> RuntimeInstance {
        let flag = Arc::new(AtomicBool::new(true));
        let (low_sender, low_receiver) = flume::unbounded::<Carrier>();
        let (high_sender, high_receiver) = flume::unbounded::<Carrier>();
        let arc_low_receiver = Arc::new(low_receiver);
        let arc_high_receiver = Arc::new(high_receiver);

        let low = Runtime::spawn_low_threads(
            Arc::clone(&flag),
            Arc::clone(&arc_low_receiver),
            Arc::clone(&arc_high_receiver),
        );

        let high = Runtime::spawn_high_threads(
            self.high,
            Arc::clone(&flag),
            Arc::clone(&arc_low_receiver),
            Arc::clone(&arc_high_receiver),
        );
        let worker_meta = WorkerMeta {
            wakers: high.1,
            blocking: low.1,
            capacity: self.high,
        };
        RuntimeInstance {
            low_handle: Some(low.0),
            high_handles: high.0,
            low_sender: Arc::new(low_sender),
            high_sender: Arc::new(high_sender),
            flag,
            worker_meta,
        }
    }

    fn spawn_low_threads(
        flag: Arc<AtomicBool>,
        low_rec: Arc<Receiver<Carrier>>,
        high_rec: Arc<Receiver<Carrier>>,
    ) -> (Join<()>, Arc<mio::Waker>) {
        let rec_low = Arc::clone(&low_rec);
        let rec_high = Arc::clone(&high_rec);
        let cloned_flag = Arc::clone(&flag);

        const MIO_TOKEN: mio::Token = mio::Token(7);
        let mut poll = mio::Poll::new().expect("OS syscall to create FD for worker thread failed!");
        // I initially thought that since poll::registry() gives a reference to registry, we cannot
        // create Poll outside of the thread and then move it into the spawned thread as that would
        // be a violation of the borrowing rules as a value would be moved when a shared reference
        // to that value is alive. However, MIO internally makes the waker register to the same FD
        // by using low level OS mechanisms and not by directly using a shared reference. If it
        // were to be directly using a shared reference, I would have had to use a channel to send
        // the waker out of the worker thread to be used by tasks to wake the threads up which
        // would have been really unfortunate!
        let mio_waker = Arc::new(
            mio::Waker::new(poll.registry(), MIO_TOKEN).expect("Mio::Waker could not be created!"),
        );
        let mut events = mio::Events::with_capacity(1);

        let handle = std::thread::spawn(move || {
            loop {
                if !cloned_flag.load(Ordering::Relaxed) {
                    break;
                }
                while let Ok(t) = rec_low.try_recv() {
                    let metadata = t.data as *const Metadata;
                    unsafe { ((*metadata).func)(metadata as *const ()) }
                }
                if let Ok(t) = rec_high.try_recv() {
                    let metadata = t.data as *const Metadata;
                    unsafe { ((*metadata).func)(metadata as *const ()) }
                } else {
                    // Thread will be woken up when a task wakes the mio::Waker!
                    match poll.poll(&mut events, None) {
                        //Some(std::time::Duration::from_millis(50))) {
                        Ok(_) => {
                            assert!(events.iter().next().is_some());
                        }
                        Err(e)
                            if e.kind() == ErrorKind::WouldBlock
                                || e.kind() == ErrorKind::Interrupted =>
                        {
                            continue;
                        }
                        Err(e) => {
                            panic!("Unexpected error {} occurred!", e);
                        }
                    }
                }
            }
            while let Ok(t) = rec_low.try_recv() {
                let metadata = t.data as *const Metadata;
                unsafe { ((*metadata).func)(metadata as *const ()) }
            }
        });
        (handle, mio_waker)
    }

    fn spawn_high_threads(
        num: usize,
        flag: Arc<AtomicBool>,
        low_rec: Arc<Receiver<Carrier>>,
        high_rec: Arc<Receiver<Carrier>>,
    ) -> (Vec<Join<()>>, Vec<Arc<mio::Waker>>) {
        let mut vector = Vec::with_capacity(AVAILABLE_PARALLELISM);

        let handles = (0..num)
            .map(|_| {
                let rec_low = Arc::clone(&low_rec);
                let rec_high = Arc::clone(&high_rec);
                let cloned_flag = Arc::clone(&flag);

                const MIO_TOKEN: mio::Token = mio::Token(7);
                let mut poll =
                    mio::Poll::new().expect("OS syscall to create FD for worker thread failed!");
                let mio_waker = Arc::new(
                    mio::Waker::new(poll.registry(), MIO_TOKEN)
                        .expect("Mio::Waker could not be created!"),
                );
                let mut events = mio::Events::with_capacity(1);

                vector.push(mio_waker);

                std::thread::spawn(move || {
                    loop {
                        if !cloned_flag.load(Ordering::Relaxed) {
                            break;
                        }
                        while let Ok(t) = rec_high.try_recv() {
                            let metadata = t.data as *const Metadata;
                            unsafe { ((*metadata).func)(metadata as *const ()) }
                        }
                        if let Ok(t) = rec_low.try_recv() {
                            let metadata = t.data as *const Metadata;
                            unsafe { ((*metadata).func)(metadata as *const ()) }
                        } else {
                            match poll.poll(&mut events, None)//Some(std::time::Duration::from_millis(50)))
                            {
                                Ok(_) => {
                                    assert!(events.iter().next().is_some());
                                    continue;
                                }
                                Err(e)
                                    if e.kind() == ErrorKind::WouldBlock
                                        || e.kind() == ErrorKind::Interrupted =>
                                {
                                    continue;
                                }
                                Err(e) => {
                                    panic!("Unexpected error {} occurred!", e);
                                }
                            }
                        }
                    }
                    while let Ok(t) = rec_high.try_recv() {
                        let metadata = t.data as *const Metadata;
                        unsafe { ((*metadata).func)(metadata as *const ()) }
                    }
                })
            })
            .collect::<Vec<_>>();
        (handles, vector)
    }
}

pub struct RuntimeInstance {
    // ALways Some, but still required to join the thread during shutdown using Option::take()
    low_handle: Option<Join<()>>,
    high_handles: Vec<Join<()>>,
    low_sender: Arc<Sender<Carrier>>,
    high_sender: Arc<Sender<Carrier>>,
    flag: Arc<AtomicBool>,
    worker_meta: WorkerMeta,
}

struct WorkerMeta {
    wakers: Vec<Arc<mio::Waker>>,
    blocking: Arc<mio::Waker>,
    capacity: usize,
}

pub struct RuntimeBuilder {
    high_threads: Option<usize>,
    low_threads: Option<usize>,
    allowed: usize,
}

impl RuntimeBuilder {
    pub fn high_priority_threads(mut self, num: usize) -> Self {
        if num < self.allowed {
            self.high_threads = Some(num);
            self.low_threads = Some(self.allowed - num);
        } else {
            self.high_threads = Some(self.allowed - 1);
            self.low_threads = Some(1);
        }
        self
    }

    pub fn build(self) -> Runtime {
        let low = self.low_threads.unwrap_or(1);
        let high = self.high_threads.unwrap_or(1);
        Runtime { low, high }
    }
}

impl RuntimeInstance {
    pub fn block_on<F>(&self, future: F) -> F::Output
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        let mut pinned_fut = Box::pin(future);

        let id = std::thread::current();

        // Nightly feature!
        let waker = std::task::waker_fn(move || {
            id.unpark();
        });

        let mut context = Context::from_waker(&waker);

        loop {
            let fut = pinned_fut.as_mut();
            if let Poll::Ready(output) = Future::poll(fut, &mut context) {
                return output;
            } else {
                std::thread::park();
            }
        }
    }

    pub fn spawn<F>(&self, future: F) -> JoinHandle<F>
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        // Random number generation if costly! We will get rid of this!
        let random = rand::random_range(0..self.worker_meta.capacity);
        let mio_waker = Arc::clone(&self.worker_meta.wakers[random]);
        let mio_waker_cloned = Arc::clone(&mio_waker);

        let metadata = Metadata {
            // Because we will send the task on the queue, we must begin with the POLLING state!
            state: AtomicUsize::new(crate::executor::POLLING),
            refcount: AtomicIsize::new(0),
            func: Task::<F>::execute,
            drop_func: Task::<F>::drop_task,
            sender: Arc::clone(&self.high_sender),
            mio_waker,
            // TODO: Think of an efficient way to wake workers up as in unconditionally waking them
            // up can end up being not very useful as by the time it wakes up some other worker
            // takes the task away and the woken worker simply goes to sleep again!
            // Maybe, store an atomic that signifies the currently active number of workers and then
            // the waker of the task decides whether to wake its worker or not accordingly!
        };

        let waker = Arc::new(AtomicPtr::new(std::ptr::null_mut()));
        let (tx, rx) = flume::bounded::<F::Output>(1);
        let task = Task {
            metadata,
            future: UnsafeCell::new(Some(Box::pin(future))),
            sender: tx,
            waker: Arc::clone(&waker),
        };

        let boxed = Box::into_raw(Box::new(task));
        let raw_metadata = unsafe { &(*boxed).metadata } as *const Metadata as *const ();
        let carrier = Carrier::new(raw_metadata);

        let _ = self.high_sender.send(carrier);
        let _ = mio_waker_cloned.wake();

        JoinHandle {
            handle: rx,
            waker: Arc::clone(&waker),
        }
    }

    pub fn spawn_blocking<F>(&self, future: F) -> JoinHandle<F>
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        let wkr = Arc::clone(&self.worker_meta.blocking);
        let wkr_cloned = Arc::clone(&wkr);
        let metadata = Metadata {
            // Because we will send the task on the queue, we must begin with the POLLING state!
            state: AtomicUsize::new(crate::executor::POLLING),
            refcount: AtomicIsize::new(0),
            func: Task::<F>::execute,
            drop_func: Task::<F>::drop_task,
            sender: Arc::clone(&self.low_sender),
            mio_waker: wkr,
        };

        let waker = Arc::new(AtomicPtr::new(std::ptr::null_mut()));
        let (tx, rx) = flume::bounded::<F::Output>(1);
        let task = Task {
            metadata,
            future: UnsafeCell::new(Some(Box::pin(future))),
            sender: tx,
            waker: Arc::clone(&waker),
        };

        let boxed = Box::into_raw(Box::new(task));
        let raw_metadata = unsafe { &(*boxed).metadata } as *const Metadata as *const ();
        let carrier = Carrier::new(raw_metadata);
        let _ = self.low_sender.send(carrier);
        let _ = wkr_cloned.wake();

        JoinHandle {
            handle: rx,
            waker: Arc::clone(&waker),
        }
    }

    // private fn
    fn shutdown_private(&mut self) {
        // Relaxed is fine because &mut self guarantees that it can be called by only one thread
        // at a time... which means that the Vec's will be drained and even if the store is not
        // visible on second call, the Vec's will have no JoinHandles... the store is eventually
        // going to become visible!
        if self.flag.load(Ordering::SeqCst) {
            self.flag.store(false, Ordering::SeqCst);

            for waker in self.worker_meta.wakers.iter() {
                let _ = waker.wake();
            }
            let _ = self.worker_meta.blocking.wake();

            let mut low_failures = 0;
            let mut high_failures = 0;

            if self
                .low_handle
                .take()
                .expect("The runtime always spawns a single low priority thread!")
                .join()
                .is_err()
            {
                low_failures = 1;
            }

            for handle in self.high_handles.drain(..) {
                if handle.join().is_err() {
                    high_failures += 1;
                }
            }

            if low_failures > 0 || high_failures > 0 {
                panic!(
                    "{} of the high priority and {} of the single low priority thread could not be joined successfully!",
                    high_failures, low_failures
                );
            }
        }
    }

    pub fn shutdown(mut self) {
        self.shutdown_private();
    }
}

impl Drop for RuntimeInstance {
    fn drop(&mut self) {
        self.shutdown_private();
    }
}
