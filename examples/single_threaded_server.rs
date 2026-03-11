use std::io::Write;
fn main() {
    let socketaddr: std::net::SocketAddr = "127.0.0.1:3000".parse().unwrap();

    let listener = std::net::TcpListener::bind(socketaddr).unwrap();

    for connection in listener.incoming() {
        let mut stream = connection.unwrap();

        let response = "HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nOK";

        stream.write_all(response.as_bytes()).unwrap();
    }
}
