#![allow(unused)]

use crate::runtime::runtime::{Carrier, HIGH_QUEUE};
use crate::runtime::waker::VTABLE;
use std::cell::UnsafeCell;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::task::{Context, Poll, Waker};

// States of a task
pub(crate) const IDLE: usize = 0;
pub(crate) const POLLING: usize = 1;
pub(crate) const NOTIFIED: usize = 2;
pub(crate) const COMPLETED: usize = 3;

struct JoinHandle<F>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    handle: mpsc::Receiver<F::Output>,
    waker: Arc<Mutex<Option<Waker>>>,
}

impl<F> Future for JoinHandle<F>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    type Output = F::Output;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // We first check whether the waker has been provided or not..
        // If yes, that means this is the second poll in which case the value is
        // guaranteed to be there because the execute function wakes up the waker
        // only after the value has been sent...
        //
        // Further, if the waker is not there, then this is the first poll which either of
        // the following
        //   -- the output has been sent but the waking has not yet happened in which
        //      case either the execute function misses the waker completely or it gets
        //      it after this function places it in there.. in either of the the operations
        //      are linearizable because in the former, the waker being missed implies that
        //      the value is already sent which means that this function will read it and return
        //      Poll::Ready.. the latter case implies that even if the joinhandle is polled
        //      for once and returns Poll::Pending, it will definitely be polled again by the
        //      execute fn
        //
        //   -- the execute function has sent the output but saw that no waker was there
        //      which is completely fine because that means that the output will be read
        //      on the first poll itself..
        //
        //   -- the operations are not vulnerable to any kind wierd sequence of premption and
        //      rescheduling of the threads by the operating system
        let mut lock = self.waker.lock().unwrap();
        if (*lock).is_none() {
            *lock = Some(cx.waker().clone());
            std::mem::drop(lock);
        }
        if let Ok(output) = self.handle.try_recv() {
            Poll::Ready(output)
        } else {
            Poll::Pending
        }
    }
}

pub(crate) struct Metadata {
    pub(crate) state: AtomicUsize,
    pub(crate) refcount: AtomicUsize,
    pub(crate) func: fn(*const ()),
    pub(crate) drop_func: fn(*const Metadata),
}

#[repr(C)]
struct Task<F>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    metadata: Metadata,
    future: UnsafeCell<Option<Pin<Box<F>>>>,
    sender: mpsc::Sender<F::Output>,
    waker: Arc<Mutex<Option<Waker>>>,
}

impl<F> Task<F>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    fn execute(metadata: *const ()) {
        let meta = metadata as *const Metadata;
        let task = meta as *const Task<F>;
        let state = unsafe { &(*meta).state };
        if let Some(future) = unsafe { &mut (*(*task).future.get()) } {
            let pinned_future = future.as_mut();
            let waker = unsafe { Waker::new(metadata, &VTABLE) };
            let mut context = Context::from_waker(&waker);
            let result = Future::poll(pinned_future, &mut context);
            if let Poll::Ready(output) = result {
                let lock = unsafe {
                    (*task).sender.send(output);
                    // TODO: Get rid of unwraps!
                    (*task).waker.lock().unwrap()
                };
                // Its fine to not obtain the waker because even if that is the case
                // the value has already been sent and the joinhandle on being polled
                // for the very first time will obtain the value and return Poll::Ready.
                if let Some(ref waker) = *lock {
                    waker.wake_by_ref();
                }
                // Ordering has been kept to Relaxed because it is only
                // the waker which when woken up check the state and
                // decide what to do.. Since only the state is checked
                // the wakers need not establish a happens before relationship
                // with whatever has happened prior to the latest store on the
                // state variable and hence can load using Ordering::Relaxed and
                // therefore we have used Ordering::Relaxed in the store.
                state.store(COMPLETED, Ordering::Relaxed);
            } else {
                loop {
                    match state.load(Ordering::Relaxed) {
                        POLLING => {
                            if state
                                .compare_exchange(
                                    POLLING,
                                    IDLE,
                                    Ordering::Relaxed,
                                    Ordering::Relaxed,
                                )
                                .is_ok()
                            {
                                break;
                            }
                        }
                        NOTIFIED => {
                            state.store(POLLING, Ordering::Relaxed);
                            unsafe { ((*meta).func)(metadata) };
                            break;
                        }
                        _ => unreachable!(),
                    }
                }
            }
        }
    }

    fn drop_task(data: *const Metadata) {
        let task = data as *const Task<F>;
        let owned = unsafe { Box::from_raw(task as *mut Task<F>) };
        std::mem::drop(owned);
    }
}

fn spawn<F>(future: F) -> JoinHandle<F>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    let metadata = Metadata {
        state: AtomicUsize::new(IDLE),
        refcount: AtomicUsize::new(0),
        func: Task::<F>::execute,
        drop_func: Task::<F>::drop_task,
    };
    let waker = Arc::new(Mutex::new(None));
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
    HIGH_QUEUE.enqueue(carrier);
    JoinHandle {
        handle: rx,
        waker: Arc::clone(&waker),
    }
}
