#![allow(unused)]

use std::task::{Context, RawWaker, RawWakerVTable, Waker};

pub(crate) const VTABLE: RawWakerVTable = RawWakerVTable::new(clone, wake, wake_by_ref, drop);

fn clone(data: *const ()) -> RawWaker {
    todo!()
}

fn wake(data: *const ()) {
    todo!()
}

fn wake_by_ref(data: *const ()) {
    todo!()
}

fn drop(data: *const ()) {
    todo!()
}
