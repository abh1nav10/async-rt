#![allow(unused)]

// TODO:
//   Tie to queues with the lifetime of the runtimehandle and ensure that an
//   appropriate error is returned whenever a user tries to spawn a task in
//   the absense of a working executor

use crate::Queue;
use crate::runtime::executor::Metadata;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock};
use std::thread::JoinHandle;

pub(crate) struct Carrier {
    data: *const (),
}

unsafe impl Send for Carrier {}

impl Carrier {
    pub(crate) fn new(data: *const ()) -> Self {
        Self { data }
    }
}

pub(crate) static HIGH_QUEUE: LazyLock<Queue<Carrier>> = LazyLock::new(Queue::new);

pub(crate) static LOW_QUEUE: LazyLock<Queue<Carrier>> = LazyLock::new(Queue::new);

struct Runtime {
    low_threads: usize,
    high_threads: usize,
}

struct RuntimeBuilder;

struct RuntimeHandle {
    low_handles: Vec<JoinHandle<()>>,
    high_handles: Vec<JoinHandle<()>>,
    flag: Arc<AtomicBool>,
}

impl Runtime {
    fn new() -> Self {
        let cpu: usize = std::thread::available_parallelism().unwrap().into();
        Self {
            low_threads: 1,
            high_threads: cpu - 1,
        }
    }

    fn configure() -> RuntimeBuilder {
        RuntimeBuilder
    }

    fn spawn(self) -> RuntimeHandle {
        let mut low_handles = Vec::with_capacity(self.low_threads);
        let mut high_handles = Vec::with_capacity(self.high_threads);
        let flag = Arc::new(AtomicBool::new(true));
        (0..self.low_threads).map(|_| {
            let f = Arc::clone(&flag);
            let handle = std::thread::spawn(move || {
                loop {
                    if f.load(Ordering::Relaxed) {
                        if let Ok(carrier) = LOW_QUEUE.dequeue() {
                            let metadata = carrier.data as *const Metadata;
                            unsafe {
                                ((*metadata).func)(metadata as *const ());
                            }
                        }
                    } else {
                        while let Ok(carrier) = LOW_QUEUE.dequeue() {
                            let metadata = carrier.data as *const Metadata;
                            unsafe {
                                ((*metadata).func)(metadata as *const ());
                            }
                        }
                        break;
                    }
                }
            });
            low_handles.push(handle);
        });
        (0..self.high_threads).map(|_| {
            let f = Arc::clone(&flag);
            let handle = std::thread::spawn(move || {
                loop {
                    if f.load(Ordering::Relaxed) {
                        if let Ok(carrier) = HIGH_QUEUE.dequeue() {
                            let metadata = carrier.data as *const Metadata;
                            unsafe {
                                ((*metadata).func)(metadata as *const ());
                            }
                        }
                    } else {
                        while let Ok(carrier) = HIGH_QUEUE.dequeue() {
                            let metadata = carrier.data as *const Metadata;
                            unsafe {
                                ((*metadata).func)(metadata as *const ());
                            }
                        }
                        break;
                    }
                }
            });
            high_handles.push(handle);
        });
        RuntimeHandle {
            low_handles,
            high_handles,
            flag,
        }
    }
}

impl RuntimeBuilder {
    fn set_low_threads(&mut self, number: usize) -> Runtime {
        let cpu: usize = std::thread::available_parallelism().unwrap().into();
        if number > cpu {
            panic!(
                "The number of threads exeeds the allowed threshhold of {}",
                cpu
            );
        } else {
            Runtime {
                high_threads: cpu - number,
                low_threads: number,
            }
        }
    }
    fn set_high_threads(&mut self, number: usize) -> Runtime {
        let cpu: usize = std::thread::available_parallelism().unwrap().into();
        if number > cpu {
            panic!(
                "The number of threads exeeds the allowed threshhold of {}",
                cpu
            );
        } else {
            Runtime {
                high_threads: number,
                low_threads: cpu - number,
            }
        }
    }
}

impl std::fmt::Debug for RuntimeHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let high_num = self.high_handles.len();
        let low_num = self.low_handles.len();
        write!(
            f,
            "Number of low priority threads: {} & number of high priority threads {}",
            low_num, high_num
        )
    }
}

impl RuntimeHandle {
    fn number_of_low_priority_threads(&self) -> usize {
        self.low_handles.len()
    }

    fn number_of_high_priority_threads(&self) -> usize {
        self.high_handles.len()
    }

    fn graceful_shutdown(mut self) {
        self.flag.store(false, Ordering::Relaxed);
        self.high_handles.drain(..).map(|t| {
            t.join()
                .expect("One of the high_priority threads failed to exit cleanly");
        });
        self.low_handles.drain(..).map(|t| {
            t.join()
                .expect("One of the low_priority threads failed to exit cleanly");
        });
    }
}

impl Drop for RuntimeHandle {
    fn drop(&mut self) {
        if self.flag.load(Ordering::Relaxed) {
            self.flag.store(false, Ordering::Relaxed);
        }
    }
}
