#![allow(dead_code)]

use crate::executor::{JoinHandle, Metadata, Task};
use flume::{Receiver, Sender};
use std::cell::UnsafeCell;
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicUsize, Ordering};
use std::sync::{Arc, mpsc};
use std::task::{Context, Poll, Waker};
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
        let allowed: usize = if let Ok(n) = std::thread::available_parallelism() {
            n.into()
        } else {
            5
        };
        let flag = Arc::new(AtomicBool::new(true));
        let (low_sender, low_receiver) = flume::unbounded::<Carrier>();
        let (high_sender, high_receiver) = flume::unbounded::<Carrier>();
        let arc_low_receiver = Arc::new(low_receiver);
        let arc_high_receiver = Arc::new(high_receiver);
        let low = Runtime::spawn_low_threads(
            1,
            Arc::clone(&flag),
            Arc::clone(&arc_low_receiver),
            Arc::clone(&arc_high_receiver),
        );
        let high = Runtime::spawn_high_threads(
            allowed - 1,
            Arc::clone(&flag),
            Arc::clone(&arc_low_receiver),
            Arc::clone(&arc_high_receiver),
        );
        RuntimeInstance {
            low_handles: low,
            high_handles: high,
            low_sender: Arc::new(low_sender),
            high_sender: Arc::new(high_sender),
            flag,
        }
    }

    pub fn builder() -> RuntimeBuilder {
        let allowed: usize = if let Ok(n) = std::thread::available_parallelism() {
            n.into()
        } else {
            5
        };
        RuntimeBuilder {
            high_threads: None,
            low_threads: None,
            allowed,
        }
    }

    pub fn start_from_config(self) -> RuntimeInstance {
        let flag = Arc::new(AtomicBool::new(true));
        let (low_sender, low_receiver) = flume::unbounded::<Carrier>();
        let (high_sender, high_receiver) = flume::unbounded::<Carrier>();
        let arc_low_receiver = Arc::new(low_receiver);
        let arc_high_receiver = Arc::new(high_receiver);

        let low = Runtime::spawn_low_threads(
            self.low,
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
        RuntimeInstance {
            low_handles: low,
            high_handles: high,
            low_sender: Arc::new(low_sender),
            high_sender: Arc::new(high_sender),
            flag,
        }
    }

    fn spawn_low_threads(
        num: usize,
        flag: Arc<AtomicBool>,
        low_rec: Arc<Receiver<Carrier>>,
        high_rec: Arc<Receiver<Carrier>>,
    ) -> Vec<Join<()>> {
        (0..num)
            .map(|_| {
                let rec_low = Arc::clone(&low_rec);
                let rec_high = Arc::clone(&high_rec);
                let cloned_flag = Arc::clone(&flag);
                std::thread::spawn(move || {
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
                        }
                    }
                    while let Ok(t) = rec_low.try_recv() {
                        let metadata = t.data as *const Metadata;
                        unsafe { ((*metadata).func)(metadata as *const ()) }
                    }
                })
            })
            .collect::<Vec<_>>()
    }

    fn spawn_high_threads(
        num: usize,
        flag: Arc<AtomicBool>,
        low_rec: Arc<Receiver<Carrier>>,
        high_rec: Arc<Receiver<Carrier>>,
    ) -> Vec<Join<()>> {
        (0..num)
            .map(|_| {
                let rec_low = Arc::clone(&low_rec);
                let rec_high = Arc::clone(&high_rec);
                let cloned_flag = Arc::clone(&flag);
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
                        }
                    }
                    while let Ok(t) = rec_high.try_recv() {
                        let metadata = t.data as *const Metadata;
                        unsafe { ((*metadata).func)(metadata as *const ()) }
                    }
                })
            })
            .collect::<Vec<_>>()
    }
}

pub struct RuntimeInstance {
    low_handles: Vec<Join<()>>,
    high_handles: Vec<Join<()>>,
    low_sender: Arc<Sender<Carrier>>,
    high_sender: Arc<Sender<Carrier>>,
    flag: Arc<AtomicBool>,
}

pub struct RuntimeBuilder {
    high_threads: Option<usize>,
    low_threads: Option<usize>,
    allowed: usize,
}

impl RuntimeBuilder {
    pub fn low_priority_threads(mut self, num: usize) -> Self {
        if num <= self.allowed {
            self.low_threads = Some(num);
            self.allowed -= num;
        } else {
            self.low_threads = Some(1);
            self.allowed -= 1;
        }
        self
    }

    pub fn high_priority_threads(mut self, num: usize) -> Self {
        if num <= self.allowed {
            self.high_threads = Some(num);
            self.allowed -= num;
        } else {
            self.high_threads = Some(self.allowed - 1);
            self.allowed = 1;
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

        // Nightly feature!
        //let waker = std::task::waker_fn(move || {
        //    id.unpark();
        //});

        let waker = Waker::noop();
        let mut context = Context::from_waker(waker);

        loop {
            let fut = pinned_fut.as_mut();
            if let Poll::Ready(output) = Future::poll(fut, &mut context) {
                return output;
            } else {
                std::hint::spin_loop();
            }
        }
    }

    pub fn spawn<F>(&self, future: F) -> JoinHandle<F>
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        let metadata = Metadata {
            state: AtomicUsize::new(crate::executor::IDLE),
            refcount: AtomicUsize::new(0),
            func: Task::<F>::execute,
            drop_func: Task::<F>::drop_task,
            sender: Arc::clone(&self.high_sender),
        };
        let waker = Arc::new(AtomicPtr::new(std::ptr::null_mut()));
        let (tx, rx) = mpsc::channel::<F::Output>();
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
        JoinHandle {
            handle: rx,
            waker: Arc::clone(&waker),
        }
    }

    pub fn spawn_low_priority<F>(&self, future: F) -> JoinHandle<F>
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        let metadata = Metadata {
            state: AtomicUsize::new(crate::executor::IDLE),
            refcount: AtomicUsize::new(0),
            func: Task::<F>::execute,
            drop_func: Task::<F>::drop_task,
            sender: Arc::clone(&self.low_sender),
        };
        let waker = Arc::new(AtomicPtr::new(std::ptr::null_mut()));
        let (tx, rx) = mpsc::channel::<F::Output>();
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
        if self.flag.load(Ordering::Relaxed) {
            self.flag.store(false, Ordering::Relaxed);

            let mut low_failures = 0;
            let mut high_failures = 0;

            for handle in self.low_handles.drain(..) {
                if handle.join().is_err() {
                    low_failures += 1;
                }
            }

            for handle in self.high_handles.drain(..) {
                if handle.join().is_err() {
                    high_failures += 1;
                }
            }

            if low_failures > 0 || high_failures > 0 {
                panic!(
                    "{} of the high priority and {} of the low priority threads could not be joined successfully!",
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
