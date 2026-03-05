use crate::runtime::Carrier;
use crate::waker::VTABLE;
use std::cell::UnsafeCell;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicIsize, AtomicPtr, AtomicUsize, Ordering};
use std::task::{Context, Poll, Waker};

// States of a task
pub(crate) const IDLE: usize = 0;
pub(crate) const POLLING: usize = 1;
pub(crate) const NOTIFIED: usize = 2;
pub(crate) const COMPLETED: usize = 3;

pub struct JoinHandle<F>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    pub(crate) handle: flume::Receiver<F::Output>,
    pub(crate) waker: Arc<AtomicPtr<Waker>>,
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
        let mut ptr = self.waker.load(Ordering::SeqCst);
        if ptr.is_null() {
            let boxed = Box::into_raw(Box::new(cx.waker().clone()));
            self.waker.store(boxed, Ordering::SeqCst);
            ptr = boxed;
        }
        if let Ok(output) = self.handle.try_recv() {
            // At this point, ptr is not null, and the output has been received which means
            // we must deallocate the waker allocation!
            self.waker.store(std::ptr::null_mut(), Ordering::SeqCst);
            let d = unsafe { Box::from_raw(ptr) };
            drop(d);
            Poll::Ready(output)
        } else {
            Poll::Pending
        }
    }
}

impl<F> Drop for JoinHandle<F>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    fn drop(&mut self) {
        let ptr = self.waker.swap(std::ptr::null_mut(), Ordering::SeqCst);
        if !ptr.is_null() {
            let _ = unsafe { Box::from_raw(ptr) };
        }
    }
}

pub(crate) struct Metadata {
    pub(crate) state: AtomicUsize,
    pub(crate) refcount: AtomicIsize,
    pub(crate) func: fn(*const ()),
    pub(crate) drop_func: fn(*const Metadata),
    pub(crate) sender: Arc<flume::Sender<Carrier>>,
    pub(crate) mio_waker: Arc<mio::Waker>,
}

#[repr(C)]
pub(crate) struct Task<F>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    pub(crate) metadata: Metadata,
    pub(crate) future: UnsafeCell<Option<Pin<Box<F>>>>,
    pub(crate) sender: flume::Sender<F::Output>,
    pub(crate) waker: Arc<AtomicPtr<Waker>>,
}

impl<F> Task<F>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    pub(crate) fn execute(metadata: *const ()) {
        loop {
            let meta = metadata as *const Metadata;
            let task = meta as *const Task<F>;

            let task_ref = unsafe { &(*task) };
            let meta_ref = unsafe { &(*meta) };
            let state = &meta_ref.state;

            if let Some(future) = unsafe { &mut (*(task_ref).future.get()) } {
                let pinned_future = future.as_mut();
                let waker = unsafe { Waker::new(metadata, &VTABLE) };

                meta_ref.refcount.fetch_add(1, Ordering::SeqCst);

                let mut context = Context::from_waker(&waker);
                let result = Future::poll(pinned_future, &mut context);

                if let Poll::Ready(output) = result {
                    let ptr = {
                        let _ = task_ref.sender.send(output);
                        task_ref.waker.load(Ordering::SeqCst)
                    };
                    // Its fine to not obtain the waker because even if that is the case
                    // the value has already been sent and the joinhandle on being polled
                    // for the very first time will obtain the value and return Poll::Ready.
                    if !ptr.is_null() {
                        unsafe {
                            (*ptr).wake_by_ref();
                        }
                    }
                    // We drop the future here itself as it has been polled to completion.
                    unsafe {
                        *(task_ref).future.get() = None;
                    }
                    state.store(COMPLETED, Ordering::SeqCst);
                    break;
                }

                match state.load(Ordering::SeqCst) {
                    POLLING => {
                        if state
                            .compare_exchange(POLLING, IDLE, Ordering::SeqCst, Ordering::SeqCst)
                            .is_ok()
                        {
                            break;
                        } else {
                            // We continue here, and if we don't continue we will the unreachable
                            // code with that statement because if the CAS fails, we will just roll
                            // over and hit that statement. Prevously this else branch and this
                            // simple continue statement was not there and it cause me a lot of debugging
                            // pain!
                            //
                            // Always double check the control flow!
                            continue;
                        }
                    }
                    NOTIFIED => {
                        state.store(POLLING, Ordering::SeqCst);
                        continue;
                    }
                    _ => unreachable!(),
                }
            }
            unreachable!(
                "We should not reach here as that would mean that the task has been pushed on the
            queue without the underlying future!"
            );
        }
    }

    pub(crate) fn drop_task(data: *const Metadata) {
        let task = data as *const Task<F>;
        let owned = unsafe { Box::from_raw(task as *mut Task<F>) };
        std::mem::drop(owned);
    }
}
