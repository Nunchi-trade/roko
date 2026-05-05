#[cfg(test)]
mod tests {
    use std::net::TcpStream;
    use std::io::{Read, Write};

    #[test]
    fn test_hello_world_response() {
        let mut stream = TcpStream::connect("127.0.0.1:7878").unwrap();
        let request = "GET / HTTP/1.1\r\n\r\n";
        stream.write(request.as_bytes()).expect("Failed to write to stream");

        let mut buffer = [0; 1024];
        stream.read(&mut buffer).expect("Failed to read from stream");

        let response = String::from_utf8_lossy(&buffer[..]);
        assert!(response.contains("Hello, world!"));
    }
}