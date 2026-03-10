use async_runtime::HyperExecutor;
use async_runtime::{Runtime, TcpListener, TcpStream};
use hyper::body::Incoming;
use hyper::{Request, Response, StatusCode, Uri};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

struct ConnectionService;

struct ConnectionFuture;

impl Future for ConnectionFuture {
    type Output = Result<Response<String>, std::io::Error>;

    fn poll(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Self::Output> {
        let response = String::from("Abhinav says Hello there!!");
        let response = Response::new(response);
        Poll::Ready(Ok(response))
    }
}

impl hyper::service::Service<Request<Incoming>> for ConnectionService {
    type Response = Response<String>;
    type Error = std::io::Error;
    type Future = ConnectionFuture;

    fn call(&self, _: Request<Incoming>) -> Self::Future {
        ConnectionFuture
    }
}

#[test]
fn hyper_integrated() {
    let socketaddr: std::net::SocketAddr = "127.0.0.1:3000".parse().unwrap();

    let runtime = Arc::new(
        Runtime::builder()
            .high_priority_threads(1)
            .build()
            .start_from_config(),
    );
    let cloned_rt = Arc::clone(&runtime);

    // We bind the listener here because it is a blocking call. However we ensure that we do so
    // after starting the runtime as only then will we have the registry to register the source
    // into!
    let listener = TcpListener::bind(socketaddr).unwrap();

    runtime.block_on(async move {
        let server_rt = Arc::clone(&cloned_rt);

        // Server
        cloned_rt.spawn(async move {
            let executor = HyperExecutor::new(server_rt);

            let (stream, _) = listener.accept().await.unwrap();

            let service = ConnectionService;

            let server = hyper::server::conn::http2::Builder::new(executor)
                .serve_connection(stream, service);

            server.await.unwrap();
        });

        // Client
        let client_rt = Arc::clone(&cloned_rt);
        cloned_rt.spawn(async move {
            let driver_rt = Arc::clone(&client_rt);
            let executor = HyperExecutor::new(client_rt);

            let mut stream = TcpStream::connect(socketaddr).unwrap();

            let stream = loop {
                if let Ok((s, _)) = stream.await {
                    break s;
                } else {
                    stream = TcpStream::connect(socketaddr).unwrap();
                }
            };

            let (mut sender, connection) = hyper::client::conn::http2::Builder::new(executor)
                .handshake(stream)
                .await
                .unwrap();

            // Drive the connection
            driver_rt.spawn(async move {
                if let Err(e) = connection.await {
                    eprintln!("Connection failed! {:?}", e)
                }
            });

            let uri: Uri = "http://127.0.0.1:3000/".parse().unwrap();
            let request = Request::builder()
                .uri(uri)
                .body(String::from("Hello there!"))
                .unwrap();

            let res = sender.send_request(request).await.unwrap();

            assert_eq!(res.status(), StatusCode::OK);

            // We take a dev-dependency on the `http-body-util` crate to get the `BodyExt` trait so
            // that we can use the `collect` method to drive the `Incoming` iterator to completion!
            use http_body_util::BodyExt;

            let body = res.into_body().collect().await.unwrap();
            let bytes = body.to_bytes();
            assert_eq!((*bytes).as_ref(), b"Abhinav says Hello there!!");
        });
    });
}
