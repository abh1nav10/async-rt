#![allow(dead_code)]

// Tested with Valgrind!
// `valgrind --tool=memcheck --track-origins=yes --verbose {..}`
// where {} -> placeholder for the location of the test binary generated with
// `cargo test --no-run`

// The tests other than the one for TcpListener have been commented out due to the problem that
// arises on having multiple runtimes in the same process because of using a static that gets
// shared between them. When run individually, all tests pass! It shall be fixed later.

#[allow(unused)]
use async_runtime::Runtime;
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

//#[test]
//fn check() {
//    let rt = Runtime::start();
//    rt.block_on(async {
//        let fut = CounterFuture { count: 5 };
//        let output = fut.await;
//        assert_eq!(output, 0);
//    });
//    let _ = rt.shutdown();
//}

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

//#[test]
//fn checkfut() {
//    let waker: Arc<Mutex<Option<Waker>>> = Arc::new(Mutex::new(None));
//    let value = Arc::new(AtomicUsize::new(0));
//    let cloned = Arc::clone(&waker);
//    let cloned_value = Arc::clone(&value);
//    let handle = std::thread::spawn(move || {
//        loop {
//            let lock = cloned.lock().unwrap();
//            if let Some(ref waker) = *lock {
//                cloned_value.store(7, Ordering::SeqCst);
//                waker.wake_by_ref();
//                break;
//            }
//        }
//    });
//
//    let rt = Runtime::builder()
//        .high_priority_threads(1)
//        .build()
//        .start_from_config();
//
//    rt.block_on(async {
//        let check_fut = CheckFuture { waker, value };
//        let output = check_fut.await;
//        assert_eq!(output, 7);
//    });
//
//    handle.join().unwrap();
//    let _ = rt.shutdown();
//}
//
//#[test]
//fn checkfut_spawn() {
//    let waker: Arc<Mutex<Option<Waker>>> = Arc::new(Mutex::new(None));
//    let value = Arc::new(AtomicUsize::new(0));
//    let cloned = Arc::clone(&waker);
//    let cloned_value = Arc::clone(&value);
//    let handle = std::thread::spawn(move || {
//        loop {
//            let lock = cloned.lock().unwrap();
//            if let Some(ref waker) = *lock {
//                cloned_value.store(7, Ordering::SeqCst);
//                waker.wake_by_ref();
//                break;
//            }
//        }
//    });
//
//    let rt = Arc::new(
//        Runtime::builder()
//            .high_priority_threads(1)
//            .build()
//            .start_from_config(),
//    );
//
//    let cloned_rt = Arc::clone(&rt);
//
//    rt.block_on(async move {
//        let check_fut = CheckFuture { waker, value };
//        let handle = cloned_rt.spawn(check_fut);
//        let output = handle.await;
//        println!("Received output through channel. The output is {}", output);
//        assert_eq!(output, 7);
//    });
//
//    handle.join().unwrap();
//}

#[allow(unused)]
use async_runtime::TcpListener;
#[allow(unused)]
use std::net::SocketAddr;
//#[test]
//fn bind_listener() {
//    let rt = Runtime::builder()
//        .high_priority_threads(1)
//        .build()
//        .start_from_config();
//
//    let socketaddr: SocketAddr = "127.0.0.1:3000".parse().unwrap();
//
//    let handle = std::thread::spawn(move || {
//        loop {
//            if std::net::TcpStream::connect(socketaddr).is_ok() {
//                break;
//            }
//            std::thread::sleep(std::time::Duration::from_millis(50));
//        }
//    });
//
//    let listener = TcpListener::bind(socketaddr).unwrap();
//    rt.block_on(async move {
//        let output = listener.accept().await;
//
//        // The very fact the we reach here makes the test pass! The following assertion is useless!
//        assert!(output.is_ok() | output.is_err());
//    });
//
//    drop(rt);
//
//    handle.join().unwrap();
//}

#[allow(unused)]
use async_runtime::TcpStream;
//#[test]
//fn test_tcpstream() {
//    let socketaddr: SocketAddr = "127.0.0.1:3000".parse().unwrap();
//
//    let server = std::net::TcpListener::bind(socketaddr).unwrap();
//    let handle = std::thread::spawn(move || {
//        while server.accept().is_err() {
//            std::thread::sleep(std::time::Duration::from_millis(50));
//        }
//    });
//
//    let rt = Runtime::builder()
//        .high_priority_threads(1)
//        .build()
//        .start_from_config();
//
//    rt.block_on(async move {
//        let stream = TcpStream::connect(socketaddr).unwrap();
//
//        let output = stream.await.unwrap();
//
//        assert_eq!(output.1, socketaddr);
//    });
//
//    handle.join().unwrap();
//}

//#[test]
//fn check_read_write() {
//    let socketaddr: SocketAddr = "127.0.0.1:3000".parse().unwrap();
//
//    let server = std::net::TcpListener::bind(socketaddr).unwrap();
//    let handle = std::thread::spawn(move || {
//        loop {
//            if let Ok((mut c, _)) = server.accept() {
//                let buffer = vec![1, 2, 3, 4, 5, 6, 7, 8, 9];
//
//                use std::io::Write;
//                let _ = c.write(&buffer);
//                c.flush().unwrap();
//
//                use std::io::Read;
//                let mut buffer = String::new();
//                loop {
//                    if c.read_to_string(&mut buffer).is_ok() {
//                        assert_eq!(buffer.as_str(), "Hello Abhinav");
//
//                        break;
//                    }
//                    std::thread::sleep(std::time::Duration::from_millis(50));
//                }
//                break;
//            }
//            std::thread::sleep(std::time::Duration::from_millis(50));
//        }
//    });
//
//    let rt = Arc::new(
//        Runtime::builder()
//            .high_priority_threads(1)
//            .build()
//            .start_from_config(),
//    );
//
//    let cloned_rt = Arc::clone(&rt);
//
//    rt.block_on(async move {
//        let output = cloned_rt.spawn(async move {
//            let stream = TcpStream::connect(socketaddr).unwrap();
//
//            let (mut stream, addr) = stream.await.unwrap();
//
//            let mut buffer = vec![0; 9];
//
//            use futures_util::AsyncReadExt;
//            stream.read_exact(&mut buffer).await.unwrap();
//
//            assert_eq!(buffer, vec![1, 2, 3, 4, 5, 6, 7, 8, 9]);
//
//            use futures_util::AsyncWriteExt;
//            let _ = stream.write_all(b"Hello Abhinav").await;
//
//            // Flushing here does nothing because we are not doing buffered writes, I anyway have just
//            // kept it here!
//            stream.flush().await.unwrap();
//
//            // We close the stream so that the listener gets the EOF and does not hang forever!
//            stream.close().await.unwrap();
//
//            assert_eq!(addr, socketaddr);
//        });
//
//        output.await;
//    });
//
//    handle.join().unwrap();
//}
