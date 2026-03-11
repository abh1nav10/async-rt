use hyper::body::Incoming;
use hyper::service::Service;
use hyper::{Request, Response};
use hyper_util::rt::tokio::TokioIo;
use std::io::Error;

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

#[tokio::main]
async fn main() {
    let socketaddr: std::net::SocketAddr = "127.0.0.1:3000".parse().unwrap();

    async move {
        println!("Binding");
        let listener = tokio::net::TcpListener::bind(socketaddr).await.unwrap();
        loop {
            let service = HyperService;
            let (stream, _) = listener.accept().await.unwrap();
            let stream = TokioIo::new(stream);
            println!("Accpeted stream!");

            tokio::spawn(async move {
                if hyper::server::conn::http1::Builder::new()
                    .serve_connection(stream, service)
                    .await
                    .is_err()
                {
                    eprintln!("Failed to serve connection!");
                }
            });
        }
    }
    .await;
}
