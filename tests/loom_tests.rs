#![allow(unexpected_cfgs)]

#[cfg(test)]
#[cfg(loom)]
mod stack_test {
    use electron::Stack;
    use loom::sync::Arc;
    #[test]
    fn test_stack() {
        loom::model(|| {
            let new = Arc::new(Stack::new());
            let cloned1 = Arc::clone(&new);
            let cloned2 = Arc::clone(&new);
            let _ = new.insert(5);
            let t1 = loom::thread::spawn(move || {
                let _ = cloned1.insert(7);
            });
            let t2 = loom::thread::spawn(move || {
                let _ = cloned2.delete();
            });
            t1.join().unwrap();
            t2.join().unwrap();
        });
    }
}

#[cfg(test)]
#[cfg(loom)]
mod queue_test {
    use electron::Queue;
    use loom::sync::Arc;
    #[test]
    fn test_queue() {
        loom::model(|| {
            let new = Arc::new(Queue::new());
            let cloned1 = Arc::clone(&new);
            let cloned2 = Arc::clone(&new);
            let _ = new.enqueue(5);
            let t1 = loom::thread::spawn(move || {
                let _ = cloned1.enqueue(7);
            });
            let t2 = loom::thread::spawn(move || {
                let _ = cloned2.dequeue();
            });
            t1.join().unwrap();
            t2.join().unwrap();
        });
    }
}

#[cfg(test)]
#[cfg(loom)]
mod hazard_test {
    use electron::sync::atomic::{AtomicPtr, AtomicUsize};
    use electron::{BoxedPointer, Doer, Holder};
    use loom::sync::Arc;
    use std::sync::atomic::Ordering;
    struct CountDrops(Arc<AtomicUsize>);
    impl Drop for CountDrops {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }
    impl CountDrops {
        fn get_number_of_drops(&self) -> usize {
            self.0.load(Ordering::Relaxed)
        }
    }
    #[test]
    fn test_hazard() {
        loom::model(|| {
            let new = Arc::new(AtomicUsize::new(0));
            let check = CountDrops(new.clone());
            let value1 = CountDrops(new.clone());
            let value2 = CountDrops(new.clone());
            let boxed1 = Box::into_raw(Box::new(value1));
            let boxed2 = Box::into_raw(Box::new(value2));
            let atm_ptr = AtomicPtr::new(boxed1);
            let mut holder = Holder::default();
            let guard = unsafe { holder.load_pointer(&atm_ptr) };
            static DROPBOX: BoxedPointer = BoxedPointer::new();
            std::mem::drop(guard);
            if let Some(mut wrapper) = unsafe { holder.swap(&atm_ptr, boxed2, &DROPBOX) } {
                wrapper.retire();
            }
            assert_eq!(check.get_number_of_drops(), 1 as usize);
            let _ = unsafe { Box::from_raw(boxed2) };
            std::mem::drop(check);
        });
    }
}
