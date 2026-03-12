#![allow(dead_code)]

use slab::Slab;
use std::cell::UnsafeCell;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, RwLock};
use std::task::Waker;

const IDLE: u8 = 0b00;
const UPDATING: u8 = 0b01;
const WAKING: u8 = 0b10;

pub struct AtomicWaker {
    state: AtomicU8,
    waker: UnsafeCell<Option<Waker>>,
}

unsafe impl Send for AtomicWaker {}
unsafe impl Sync for AtomicWaker {}

impl Default for AtomicWaker {
    fn default() -> Self {
        Self::new()
    }
}

impl AtomicWaker {
    pub fn new() -> Self {
        Self {
            state: AtomicU8::new(IDLE),
            waker: UnsafeCell::new(None),
        }
    }

    pub fn new_with_waker(waker: Waker) -> Self {
        Self {
            state: AtomicU8::new(IDLE),
            waker: UnsafeCell::new(Some(waker)),
        }
    }

    pub fn update(&self, _waker: Waker) {
        // Only once instance of the carrier for our future can be in the queue at once! And we
        // register the new waker while polling the future! Therefore the only states that we can
        // encounter while CASing the state from IDLE to UPDATING are either IDLE or WAKING but
        // never UPDATING
        loop {
            match self
                .state
                .compare_exchange(IDLE, UPDATING, Ordering::SeqCst, Ordering::SeqCst)
            {
                Ok(_) => {
                    // we will try to register the waker now
                    // and then try to CAS back to IDLE from UPDATING. If the state that we see is
                    // WAKING,
                    break;
                }
                Err(_) => {
                    continue;
                }
            }
        }
    }

    pub fn wake(&self) {
        if self.state.fetch_or(WAKING, Ordering::SeqCst) == IDLE {
            // Since it is the reactor that wakes the tasks up, if it is waking the only two states
            // that we can encounter when doing this fetch_or are IDLE or UPDATING. If the state is
            // updating, we just do nothing because the updater will wake the waker for us and if
            // the state if IDLE, we wake the waker up.

            // We then try to set it back to IDLE from WAKING with a CAS from WAKING to IDLE... if
            // we succeed great, else the state was UPDATING in which case the UPDATING thread will
            // take the appropriate action!
        }
    }
}

pub struct Torque {
    data: RwLock<Slab<Arc<AtomicWaker>>>,
}

impl Default for Torque {
    fn default() -> Self {
        Self::new()
    }
}

impl Torque {
    pub fn new() -> Self {
        Torque {
            data: RwLock::new(Slab::new()),
        }
    }

    pub fn insert(&self, waker: Waker) -> usize {
        let mut guard = self.data.write().expect("RwLock is poisoned");
        guard.insert(Arc::new(AtomicWaker::new_with_waker(waker)))
    }

    pub fn update(&self, index: usize, waker: Waker) {
        let guard = self.data.read().expect("RwLock is poisoned");
        let atomic_waker = if let Some(waker) = guard.get(index) {
            Arc::clone(waker)
        } else {
            return;
        };

        drop(guard);
        atomic_waker.update(waker);
    }

    pub fn wake(&self, index: usize) {
        let guard = self.data.read().expect("RwLock is poisoned!");
        let atomic_waker = if let Some(waker) = guard.get(index) {
            Arc::clone(waker)
        } else {
            return;
        };

        drop(guard);
        atomic_waker.wake();
    }

    pub fn remove(&self, entry: usize) {
        let mut guard = self.data.write().expect("RwLock is poisoned");
        guard.remove(entry);
    }
}
