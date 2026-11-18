//! A loopback HTTP/1.1 server for the transport tests: std only, one thread per connection, every request recorded.
#![allow(dead_code)]

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct Seen {
    pub request_line: String,
    pub headers: Vec<(String, String)>,
}

impl Seen {
    pub fn path(&self) -> &str {
        self.request_line.split(' ').nth(1).unwrap_or("")
    }
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

pub enum Reply {
    Raw(Vec<u8>),
    /// Accept the request and never answer.
    Hang,
}

/// `HTTP/1.1 <status>` with a Content-Length body and `Connection: close`.
pub fn response(status: &str, headers: &[(&str, &str)], body: &[u8]) -> Reply {
    let mut v = format!(
        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n",
        body.len()
    )
    .into_bytes();
    for (k, val) in headers {
        v.extend_from_slice(format!("{k}: {val}\r\n").as_bytes());
    }
    v.extend_from_slice(b"\r\n");
    v.extend_from_slice(body);
    Reply::Raw(v)
}

pub struct Server {
    pub addr: SocketAddr,
    pub seen: Arc<Mutex<Vec<Seen>>>,
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl Server {
    pub fn url(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}{path}", self.addr.port())
    }
    pub fn requests(&self) -> Vec<Seen> {
        self.seen.lock().unwrap().clone()
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

type Handler = Arc<dyn Fn(&Seen) -> Reply + Send + Sync>;

pub fn serve(handler: impl Fn(&Seen) -> Reply + Send + Sync + 'static) -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let addr = listener.local_addr().unwrap();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let stop = Arc::new(AtomicBool::new(false));
    let handler: Handler = Arc::new(handler);
    let (seen2, stop2) = (seen.clone(), stop.clone());
    let handle = std::thread::spawn(move || {
        let mut conns: Vec<JoinHandle<()>> = Vec::new();
        while !stop2.load(Ordering::SeqCst) {
            match listener.accept() {
                Ok((stream, _)) => {
                    let (h, s, st) = (handler.clone(), seen2.clone(), stop2.clone());
                    conns.push(std::thread::spawn(move || connection(stream, h, s, st)));
                }
                Err(_) => std::thread::sleep(Duration::from_millis(5)),
            }
        }
        for c in conns {
            let _ = c.join();
        }
    });
    Server {
        addr,
        seen,
        stop,
        handle: Some(handle),
    }
}

fn connection(
    mut stream: TcpStream,
    handler: Handler,
    seen: Arc<Mutex<Vec<Seen>>>,
    stop: Arc<AtomicBool>,
) {
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_read_timeout(Some(Duration::from_millis(200)));
    let mut head = Vec::new();
    let mut buf = [0u8; 1024];
    while !head.windows(4).any(|w| w == b"\r\n\r\n") {
        match stream.read(&mut buf) {
            Ok(0) => return,
            Ok(n) => head.extend_from_slice(&buf[..n]),
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                if stop.load(Ordering::SeqCst) {
                    return;
                }
            }
            Err(_) => return,
        }
    }
    let text = String::from_utf8_lossy(&head).to_string();
    let mut lines = text.split("\r\n");
    let request_line = lines.next().unwrap_or("").to_string();
    let headers = lines
        .take_while(|l| !l.is_empty())
        .filter_map(|l| {
            l.split_once(':')
                .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
        })
        .collect();
    let req = Seen {
        request_line,
        headers,
    };
    seen.lock().unwrap().push(req.clone());
    match handler(&req) {
        Reply::Raw(bytes) => {
            let _ = stream.write_all(&bytes);
            let _ = stream.flush();
        }
        Reply::Hang => {
            while !stop.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    }
}
