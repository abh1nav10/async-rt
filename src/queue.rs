use crate::sync::atomic::AtomicPtr;
use std::marker::PhantomData;
use std::mem::MaybeUninit;
use std::ptr;
use std::sync::atomic::Ordering;

use crate::{BoxedPointer, Doer, Holder};

static DROPBOX: BoxedPointer = BoxedPointer::new();

struct Node<T> {
    value: MaybeUninit<T>,
    next: AtomicPtr<Node<T>>,
}

impl<T> Node<T> {
    fn new() -> Self {
        Self {
            value: MaybeUninit::uninit(),
            next: AtomicPtr::new(ptr::null_mut()),
        }
    }

    fn write(&mut self, value: T) {
        self.value.write(value);
    }
}

pub struct Queue<T> {
    head: AtomicPtr<Node<T>>,
    tail: AtomicPtr<Node<T>>,
    marker: PhantomData<Node<T>>,
}

unsafe impl<T> Send for Queue<T> where T: Send {}
unsafe impl<T> Sync for Queue<T> where T: Send {}

impl<T> Drop for Queue<T> {
    fn drop(&mut self) {
        let mut current = self.head.load(Ordering::Acquire);
        while !current.is_null() {
            let new = unsafe { (*current).next.load(Ordering::Acquire) };
            let owned = unsafe { Box::from_raw(current) };
            std::mem::drop(owned);
            current = new;
        }
    }
}

impl<T> Default for Queue<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> Queue<T> {
    pub fn new() -> Self {
        let sentinel_node = Box::into_raw(Box::new(Node::new()));
        Self {
            head: AtomicPtr::new(sentinel_node),
            tail: AtomicPtr::new(sentinel_node),
            marker: PhantomData,
        }
    }

    pub fn enqueue(&self, value: T) {
        let mut node = Node::new();
        node.write(value);
        let allocated = Box::into_raw(Box::new(node));
        loop {
            let mut holder = Holder::default();
            let guard = unsafe {
                holder
                    .load_pointer(&self.tail)
                    .expect("Sentinel node guarantees that the tail pointer is never null")
            };
            let cas_result = unsafe {
                (*guard.data).next.compare_exchange(
                    ptr::null_mut(),
                    allocated,
                    Ordering::AcqRel,
                    Ordering::Relaxed,
                )
            };
            if cas_result.is_ok() {
                let _ = self.tail.compare_exchange(
                    guard.data,
                    allocated,
                    Ordering::AcqRel,
                    Ordering::Relaxed,
                );
                return;
            }
        }
    }

    pub fn dequeue(&self) -> Result<T, &str> {
        loop {
            let mut current_head_holder = Holder::default();
            let mut next_node_holder = Holder::default();
            let current_head_guard = unsafe {
                current_head_holder
                    .load_pointer(&self.head)
                    .expect("Sentiled node will never allow it to be null")
            };
            let next_node_guard = if let Some(guard) =
                unsafe { next_node_holder.load_pointer(&(*current_head_guard.data).next) }
            {
                guard
            } else {
                return Err("There are no elements in the queue");
            };
            let mut tail_holder = Holder::default();
            let tail_guard = unsafe {
                tail_holder
                    .load_pointer(&self.tail)
                    .expect("Has to be there")
            };
            if tail_guard.data == current_head_guard.data {
                let _ = self.tail.compare_exchange(
                    tail_guard.data,
                    next_node_guard.data,
                    Ordering::AcqRel,
                    Ordering::Relaxed,
                );
            }
            if self
                .head
                .compare_exchange(
                    current_head_guard.data,
                    next_node_guard.data,
                    Ordering::AcqRel,
                    Ordering::Relaxed,
                )
                .is_ok()
            {
                let read_value = unsafe { (*next_node_guard.data).value.assume_init_read() };
                let mut swap_holder = Holder::default();
                let wrapper = unsafe {
                    swap_holder.get_wrapper(&AtomicPtr::new(current_head_guard.data), &DROPBOX)
                };
                if let Some(mut wrapper) = wrapper {
                    wrapper.retire();
                }
                return Ok(read_value);
            }
        }
    }
}
