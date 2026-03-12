#![allow(dead_code)]

use slab::Slab;
use std::cell::UnsafeCell;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, RwLock};
use std::task::Waker;

const IDLE: u8 = 0b00;
const UPDATING: u8 = 0b01;
const WAKING: u8 = 0b10;

struct AtomicWaker {
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
    fn new() -> Self {
        Self {
            state: AtomicU8::new(IDLE),
            waker: UnsafeCell::new(None),
        }
    }

    fn new_with_waker(waker: Waker) -> Self {
        Self {
            state: AtomicU8::new(IDLE),
            waker: UnsafeCell::new(Some(waker)),
        }
    }

    fn update(&self, waker: Waker) {
        // Only one instance of the carrier for our future can be in the queue at once! And we
        // register the new waker while polling the future! Therefore the only states that we can
        // encounter while CASing the state from IDLE to UPDATING are either IDLE or WAKING but
        // never UPDATING
        loop {
            match self
                .state
                .compare_exchange(IDLE, UPDATING, Ordering::SeqCst, Ordering::SeqCst)
            {
                Ok(_) => {
                    // we will try to register the waker now and then try to CAS back to IDLE from UPDATING.
                    unsafe { *self.waker.get() = Some(waker) };

                    match self.state.compare_exchange(
                        UPDATING,
                        IDLE,
                        Ordering::SeqCst,
                        Ordering::SeqCst,
                    ) {
                        Ok(_) => break,
                        Err(_) => {
                            // At this stage, the state can only be set to 0b11 as the waker thread
                            // might have fetch_or'd it to that state.

                            unsafe {
                                let waker = (*self.waker.get()).take();
                                waker.unwrap_unchecked().wake();
                            }

                            // We simply store here because nobody can be updating the state at
                            // this point of time.
                            self.state.store(IDLE, Ordering::SeqCst);
                            break;
                        }
                    }
                }
                Err(_) => {
                    continue;
                }
            }
        }
    }

    fn wake(&self) {
        if self.state.fetch_or(WAKING, Ordering::SeqCst) == IDLE {
            // Since it is the reactor that wakes the tasks up, if it is waking the only two states
            // that we can encounter when doing this fetch_or are IDLE or UPDATING. If the state is
            // updating, we just do nothing because the updater will wake the waker for us and if
            // the state if IDLE, we wake the waker up.

            // We then try to set it back to IDLE from WAKING with a CAS from WAKING to IDLE... if
            // we succeed great, else the state was UPDATING in which case the UPDATING thread will
            // take the appropriate action!
            let waker = unsafe { (*self.waker.get()).take() };
            if let Some(waker) = waker {
                waker.wake();
            }

            // We simply store IDLE here because given the back that we transitioned from IDLE to
            // WAKING proves that the updating thread can do nothing but spin until the state is
            // set back to IDLE!
            self.state.store(IDLE, Ordering::SeqCst);
        }
    }
}

pub(crate) struct Torque {
    data: RwLock<Slab<Arc<AtomicWaker>>>,
}

impl Default for Torque {
    fn default() -> Self {
        Self::new()
    }
}

impl Torque {
    pub(crate) fn new() -> Self {
        Torque {
            data: RwLock::new(Slab::new()),
        }
    }

    pub(crate) fn insert(&self, waker: Waker) -> usize {
        let mut guard = self.data.write().expect("RwLock is poisoned");
        guard.insert(Arc::new(AtomicWaker::new_with_waker(waker)))
    }

    pub(crate) fn insert_without_waker(&self) -> usize {
        let mut guard = self.data.write().expect("RwLock is poisoned!");
        let atomic_waker = Arc::new(AtomicWaker::default());
        guard.insert(atomic_waker)
    }

    pub(crate) fn update(&self, index: usize, waker: Waker) {
        let guard = self.data.read().expect("RwLock is poisoned");
        let atomic_waker = if let Some(waker) = guard.get(index) {
            Arc::clone(waker)
        } else {
            return;
        };

        drop(guard);
        atomic_waker.update(waker);
    }

    pub(crate) fn wake(&self, index: usize) {
        let guard = self.data.read().expect("RwLock is poisoned!");
        let atomic_waker = if let Some(waker) = guard.get(index) {
            Arc::clone(waker)
        } else {
            return;
        };

        drop(guard);
        atomic_waker.wake();
    }

    pub(crate) fn remove(&self, entry: usize) {
        let mut guard = self.data.write().expect("RwLock is poisoned");
        guard.remove(entry);
    }
}
