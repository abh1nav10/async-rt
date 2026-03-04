use async_runtime::runtime::Runtime;
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

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

#[test]
fn check() {
    let rt = Runtime::start();
    rt.block_on(async {
        let fut = CounterFuture { count: 5 };
        let output = fut.await;
        assert_eq!(output, 0);
    });
    rt.shutdown();
}
