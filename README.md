# Implementation of an Async Runtime

## Benchmark 

### Run with-
- `wrk -t4 -c1000 -d30s --latency`
- Ran on ArchLinux on WSL2
- The different server implementations used for the benchmark can be 
  seen in the examples directory.

### Results

| Architecture | Req/sec | p50 | p75 | p90 | p99 | Errors |
|---|---|---|---|---|---|---|
| Single-threaded | 109,586 | 0.86ms | 1.42ms | 99.17ms | 923ms | 3,294,267 |
| One-thread-per-connection | 13,545 | 9.48ms | 10.28ms | 11.15ms | 223ms | 407,287 |
| *async-rt* | *312,071* | *2.92ms* | *3.34ms* | *3.81ms* | *5.58ms* | *0* |
| Tokio | 411,404 | 1.45ms | 1.97ms | 2.62ms | 4.05ms | 0 |

### Analysis

- `async-rt` achieves almost *75%* of the throughput acheived by `Tokio`.
- `async-rt`thread-per-connection server by a factor of 23.
- *40x* better p99 tail-latency over the server with one thread per connection.  
- Zero errors while handling connections!
- Falls behind with a delta of *1.53ms* p99 tail latency compared to Tokio.

## Usage

```rust
use std::net::SocketAddr;
use std::sync::Arc;

let socketaddr: SocketAddr = "127.0.0.1:3000".parse().unwrap();

let rt = Arc::new(Runtime::builder().high_priority_threads(7).build().start_from_config());

let cloned_rt = Arc::clone(&rt);

rt.block_on(async move {
    let listener = TcpListener::bind(socketaddr).unwrap();

    loop {
        let (stream, _) = listener.accept().await.unwrap();
        cloned_rt.spawn(async move {
            // handle the received stream
        });
    }
});
```

## Integrating Hyper 

```rust
// This examples shows how to integrate with hyper's HTTP/1.1 implementation.
// In order to integrate with hyper's HTTP/2, the `Builder` will require an 
// executor. We provide `HyperExecutor` type which is a tuple struct wrapping 
//`Arc<Runtime>`. It can be used to flawlessly integrate with HTTP/2.

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
        let listener = TcpListener::bind(socketaddr).unwrap();
        loop {
            let service = HyperService;
            let (stream, _) = listener.accept().await.unwrap();

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
```
