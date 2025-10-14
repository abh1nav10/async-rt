use crate::Queue;
use std::panic::{self, AssertUnwindSafe};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};

type Task = Box<dyn FnOnce() + Send + 'static>;

pub struct ThreadPool {
    threads: Vec<JoinHandle<()>>,
    tasks: Arc<Queue<Task>>,
    execution: Arc<AtomicBool>,
}

impl Drop for ThreadPool {
    fn drop(&mut self) {
        self.execution.store(false, Ordering::Relaxed);
        let mut counter = 0;
        for thread in self.threads.drain(..) {
            if thread.join().is_err() {
                counter += 1;
            }
        }
        eprintln!("{} threads panicked while joining!", counter);
    }
}

impl ThreadPool {
    pub fn new(number: usize) -> ThreadPool {
        ThreadPool {
            threads: Vec::with_capacity(number),
            tasks: Arc::new(Queue::<Task>::new()),
            execution: Arc::new(AtomicBool::new(true)),
        }
    }

    pub fn spawn(&mut self) {
        for _ in 0..self.threads.capacity() {
            let queue: Arc<Queue<Task>> = Arc::clone(&self.tasks);
            let execution: Arc<AtomicBool> = Arc::clone(&self.execution);
            let thread = thread::spawn(move || {
                loop {
                    if !execution.load(Ordering::Relaxed) {
                        while let Ok(func) = queue.dequeue() {
                            // Using AssertUnwindSafe here is fine in order to make the catch_unwind
                            // succeed because we are never operating on the state of the underlying
                            // things after the error is caught.
                            let _ = panic::catch_unwind(AssertUnwindSafe(func));
                        }
                        break;
                    }
                    if let Ok(func) = queue.dequeue() {
                        // Using AssertUnwindSafe here is fine in order to make the catch_unwind
                        // succeed because we are never operating on the state of the underlying
                        // things after the error is caught.
                        let _ = panic::catch_unwind(AssertUnwindSafe(func));
                    } else {
                        thread::yield_now();
                    }
                }
            });
            self.threads.push(thread);
        }
    }

    pub fn execute_task<T>(&self, task: T)
    where
        T: FnOnce() + Send + 'static,
    {
        let boxed = Box::new(task);
        self.tasks.enqueue(boxed);
    }
}
