#![allow(unused)]

use crate::Queue;
use std::sync::LazyLock;
use std::thread::JoinHandle;

pub(crate) struct Carrier;

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
        (0..self.low_threads).map(|_| {
            let handle = std::thread::spawn(move || {
                loop {
                    // do something
                }
            });
            low_handles.push(handle);
        });
        (0..self.high_threads).map(|_| {
            let handle = std::thread::spawn(move || {
                loop {
                    // do something
                }
            });
            high_handles.push(handle);
        });
        RuntimeHandle {
            low_handles,
            high_handles,
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
