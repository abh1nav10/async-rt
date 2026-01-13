#![allow(unused)]

// TODO: Fix the orderings, they are definitely wrong as
// the executor needs to establish a happens before relationship with the operations that
// happened last time when the task was enqueued.. and thus Ordering::Relaxed won't suffice

use crate::runtime::executor::{COMPLETED, IDLE, Metadata, NOTIFIED, POLLING};
use crate::runtime::runtime::{Carrier, HIGH_QUEUE};
use std::sync::atomic::Ordering;
use std::task::{Context, RawWaker, RawWakerVTable, Waker};

pub(crate) const VTABLE: RawWakerVTable = RawWakerVTable::new(clone, wake, wake_by_ref, drop);

fn clone(data: *const ()) -> RawWaker {
    let metadata = data as *const Metadata;
    unsafe { (*metadata).refcount.fetch_add(1, Ordering::Relaxed) };
    RawWaker::new(data, &VTABLE)
}

fn wake(data: *const ()) {
    let metadata = data as *const Metadata;
    let state = unsafe { &(*metadata).state };
    let refcount = unsafe { &(*metadata).refcount };
    loop {
        match state.load(Ordering::Relaxed) {
            IDLE => {
                if state
                    .compare_exchange(IDLE, POLLING, Ordering::Relaxed, Ordering::Relaxed)
                    .is_ok()
                {
                    // Decrementing the refcount is necessary because even though the standard library's
                    // wake implementation takes ownership of the waker through self, it uses
                    // ManuallyDrop internally to prevent the waker from getting dropped and thus
                    // the drop function does not run... and thus we manually decrement the refcount
                    // so that the task actually gets dropped once no more wakers are there and the
                    // underlying future in the task has been polled to completion
                    //
                    // We do not drop here because the task is going to be polled and a new waker is
                    // going to be created and dropping here will undoubtedly lead to undefined
                    // behaviour due to a dangling pointer dereference later when the executor
                    // executes the task
                    //
                    // If we were to enqueue before decrementing the refcount that could,
                    // leak the task in an interleaving where the this thread gets
                    // prempted after enqueuing and before decrementing the refcount... as in
                    // if the waker given out by the poll function is just dropped, then it will
                    // still see the refcount to be more than 0, and hence will not drop the task
                    // and when this thread wakes up it will decrement the refcount but will not
                    // drop the task and thus the task has not been leaked forever
                    // Doing it this way, makes sure that if the task is completed, and the
                    // refcount gets to zero, the task is dropped for sure.
                    refcount.fetch_sub(1, Ordering::Relaxed);
                    HIGH_QUEUE.enqueue(Carrier::new(data));
                    break;
                }
            }
            POLLING => {
                if state
                    .compare_exchange(POLLING, NOTIFIED, Ordering::Relaxed, Ordering::Relaxed)
                    .is_ok()
                {
                    refcount.fetch_sub(1, Ordering::Relaxed);
                }
            }
            NOTIFIED => {}
            COMPLETED => {}
            _ => unreachable!(),
        }
    }
}

fn wake_by_ref(data: *const ()) {
    todo!()
}

fn drop(data: *const ()) {
    let metadata = data as *const Metadata;
    let refcount = unsafe { &(*metadata).refcount };
    let state = unsafe { &(*metadata).state };
    refcount.fetch_sub(1, Ordering::Relaxed);
    // If we were to drop the task just by checking whether the waker refcount is zero,
    // we would be generating possibilites of Undefined behaviour as follows..
    // Assuming that there is are two wakers held by a user for a task, the user calls
    // wake through one of them, the refcount is decremented to one, and before the task is
    // executed, the user just drops the other waker, the task will be dropped and when
    // the execute function is called by the executor, it would dereference a dangling pointer
    // and hence cause UB.
    if refcount.load(Ordering::Relaxed) == 0 && state.load(Ordering::Relaxed) == COMPLETED {
        unsafe { ((*metadata).drop_func)(metadata) };
    }
}
