// We run the example and then test the server with `wrk` by configuring threads, concurrent
// connections and duration to get the stats for our runtime. We do the same for other example
// that starts the server with our own tokio as the runtime.

use async_runtime::{Runtime, TcpListener};
use hyper::body::Incoming;
use hyper::service::Service;
use hyper::{Request, Response};
use std::io::Error;
use std::sync::Arc;

struct HyperService;

struct ConnectionFuture;

impl Future for ConnectionFuture {
    type Output = Result<Response<String>, Error>;

    fn poll(
        self: std::pin::Pin<&mut Self>,
        _: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        let response = Response::new(String::from("Hello there!!"));
        std::task::Poll::Ready(Ok(response))
    }
}

impl Service<Request<Incoming>> for HyperService {
    type Response = Response<String>;
    type Error = Error;
    type Future = ConnectionFuture;

    fn call(&self, _: Request<Incoming>) -> Self::Future {
        ConnectionFuture
    }
}

fn main() {
    let socketaddr: std::net::SocketAddr = "127.0.0.1:3000".parse().unwrap();
    let rt = Arc::new(
        Runtime::builder()
            .high_priority_threads(8)
            .build()
            .start_from_config(),
    );

    let cloned_rt = Arc::clone(&rt);

    rt.block_on(async move {
        println!("Binding");
        let listener = TcpListener::bind(socketaddr).unwrap();
        loop {
            let service = HyperService;
            let (stream, _) = listener.accept().await.unwrap();
            println!("Accepted stream!");

            cloned_rt.spawn(async move {
                if let Err(e) = hyper::server::conn::http1::Builder::new()
                    .serve_connection(stream, service)
                    .await
                {
                    eprintln!("Failed to serve connection {:?}", e);
                }
            });
        }
    });
}
