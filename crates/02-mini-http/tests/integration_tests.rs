use std::io::{Read, Write};
use std::net::TcpStream;
use std::process::{Child, Command};
use std::thread;
use std::time::Duration;

struct Server {
    child: Child,
}

impl Server {
    fn new() -> Self {
        let child = Command::new("cargo")
            .args(["run", "-p", "mini-http"])
            .spawn()
            .expect("Failed to start server");
        
        // Give the server a moment to bind to the port
        thread::sleep(Duration::from_secs(2));
        Server { child }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

#[test]
fn test_server_response() {
    let _server = Server::new();

    let mut stream = TcpStream::connect("127.0.0.1:7878").expect("Failed to connect to server");
    stream.write_all(b"GET / HTTP/1.1\r\n\r\n").expect("Failed to write to stream");

    let mut buffer = [0; 1024];
    stream.read(&mut buffer).expect("Failed to read from stream");

    let response = String::from_utf8_lossy(&buffer);
    assert!(response.contains("HTTP/1.1 200 OK"));
    assert!(response.contains("Served by Phase 2 Mini HTTP"));
}

#[test]
fn test_404_response() {
    let _server = Server::new();

    let mut stream = TcpStream::connect("127.0.0.1:7878").expect("Failed to connect to server");
    stream.write_all(b"GET /unknown HTTP/1.1\r\n\r\n").expect("Failed to write to stream");

    let mut buffer = [0; 1024];
    stream.read(&mut buffer).expect("Failed to read from stream");

    let response = String::from_utf8_lossy(&buffer);
    assert!(response.contains("HTTP/1.1 404 NOT FOUND"));
}
