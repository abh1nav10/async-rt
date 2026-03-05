#![allow(dead_code)]

// Tested with Valgrind!
// `valgrind --tool=memcheck --track-origins=yes --verbose {..}`
// where {} -> placeholder for the location of the test binary generated with
// `cargo test --no-run`

use async_runtime::runtime::Runtime;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};

struct CounterFuture {
    count: usize,
}

impl Future for CounterFuture {
    type Output = usize;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        println!("Polling!!");

        self.count -= 1;
        if self.count > 0 {
            cx.waker().wake_by_ref();
            Poll::Pending
        } else {
            Poll::Ready(self.count)
        }
    }
}

#[test]
fn check() {
    let rt = Runtime::start();
    rt.block_on(async {
        let fut = CounterFuture { count: 5 };
        let output = fut.await;
        assert_eq!(output, 0);
    });
    rt.shutdown();
}

struct CheckFuture {
    waker: Arc<Mutex<Option<Waker>>>,
    value: Arc<AtomicUsize>,
}

impl Future for CheckFuture {
    type Output = usize;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let value = self.value.load(Ordering::SeqCst);
        if value > 0 {
            Poll::Ready(value)
        } else {
            let mut guard = self.waker.lock().unwrap();
            if guard.is_none() {
                *guard = Some(cx.waker().clone());
                Poll::Pending
            } else {
                let value = self.value.load(Ordering::SeqCst);
                if value > 0 {
                    Poll::Ready(value)
                } else {
                    Poll::Pending
                }
            }
        }
    }
}

#[test]
fn checkfut() {
    let waker: Arc<Mutex<Option<Waker>>> = Arc::new(Mutex::new(None));
    let value = Arc::new(AtomicUsize::new(0));
    let cloned = Arc::clone(&waker);
    let cloned_value = Arc::clone(&value);
    let handle = std::thread::spawn(move || {
        loop {
            let lock = cloned.lock().unwrap();
            if let Some(ref waker) = *lock {
                cloned_value.store(7, Ordering::SeqCst);
                waker.wake_by_ref();
                break;
            }
        }
    });

    let rt = Runtime::builder()
        .high_priority_threads(1)
        .build()
        .start_from_config();

    rt.block_on(async {
        let check_fut = CheckFuture { waker, value };
        let output = check_fut.await;
        assert_eq!(output, 7);
    });

    handle.join().unwrap();
    rt.shutdown();
}

#[test]
fn checkfut_spawn() {
    let waker: Arc<Mutex<Option<Waker>>> = Arc::new(Mutex::new(None));
    let value = Arc::new(AtomicUsize::new(0));
    let cloned = Arc::clone(&waker);
    let cloned_value = Arc::clone(&value);
    let handle = std::thread::spawn(move || {
        loop {
            let lock = cloned.lock().unwrap();
            if let Some(ref waker) = *lock {
                cloned_value.store(7, Ordering::SeqCst);
                waker.wake_by_ref();
                break;
            }
        }
    });

    let rt = Arc::new(
        Runtime::builder()
            .high_priority_threads(1)
            .build()
            .start_from_config(),
    );

    let cloned_rt = Arc::clone(&rt);

    rt.block_on(async move {
        let check_fut = CheckFuture { waker, value };
        let handle = cloned_rt.spawn(check_fut);
        let output = handle.await;
        println!("Received output through channel. The output is {}", output);
        assert_eq!(output, 7);
    });

    handle.join().unwrap();
}
