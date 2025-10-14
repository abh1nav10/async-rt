use criterion::{Criterion, criterion_group, criterion_main};
use electron::{Queue, Stack};
use std::collections::LinkedList;
use std::sync::Mutex;

fn std_mutex_stack(threads: usize) {
    let new = &Mutex::new(LinkedList::new());
    std::thread::scope(|s| {
        for i in 0..threads {
            s.spawn(move || {
                new.lock().unwrap().push_front(i);
            });
        }
        for _ in 0..threads {
            s.spawn(move || {
                let _ = new.lock().unwrap().pop_front();
            });
        }
    });
}

fn electron_stack(threads: usize) {
    let new = &Stack::new();
    std::thread::scope(|s| {
        for i in 0..threads {
            s.spawn(move || {
                let _ = new.insert(i);
            });
        }
        for _ in 0..threads {
            s.spawn(move || {
                let _ = new.delete();
            });
        }
    });
}

fn std_mutex_queue(threads: usize) {
    let new = &Mutex::new(LinkedList::new());
    std::thread::scope(|s| {
        for i in 0..threads {
            s.spawn(move || {
                new.lock().unwrap().push_back(i);
            });
        }
        for _ in 0..threads {
            s.spawn(move || {
                let _ = new.lock().unwrap().pop_front();
            });
        }
    });
}

fn electron_queue(threads: usize) {
    let new = &Queue::new();
    std::thread::scope(|s| {
        for i in 0..threads {
            s.spawn(move || {
                new.enqueue(i);
            });
        }
        for _ in 0..threads {
            s.spawn(move || {
                let _ = new.dequeue();
            });
        }
    });
}

macro_rules! generate_stack_benchmark {
    ($name: ident, $number: expr) => {
        fn $name(c: &mut Criterion) {
            let mut group = c.benchmark_group("Bravo");
            group.bench_function("Std_stack", |b| b.iter(|| std_mutex_stack($number)));
            group.bench_function("Electron_stack", |b| b.iter(|| electron_stack($number)));
            group.finish();
        }
    };
}

macro_rules! generate_queue_benchmark {
    ($name: ident, $number: expr) => {
        fn $name(c: &mut Criterion) {
            let mut group = c.benchmark_group("Delta");
            group.bench_function("Std_queue", |b| b.iter(|| std_mutex_queue($number)));
            group.bench_function("Electron_queue", |b| b.iter(|| electron_queue($number)));
            group.finish();
        }
    };
}
generate_stack_benchmark!(benchmark1, 10);
generate_stack_benchmark!(benchmark2, 100);
generate_queue_benchmark!(benchmark3, 10);
generate_queue_benchmark!(benchmark4, 100);

criterion_group! {name = benchmarks; config = Criterion::default(); targets = benchmark1, benchmark2, benchmark3, benchmark4}
criterion_main!(benchmarks);
