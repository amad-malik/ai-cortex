//! A FAKE HTTP/1.1 server on 127.0.0.1:0 for the client tests (std only). It records the raw request head and body of every
//! request, answers from a script (then a default), and can close the connection, delay, trickle or stream, so the tests see
//! exactly what the client sent and how it behaves when the server misbehaves. One response per connection (`Connection: close`).
#![allow(dead_code)]

use std::collections::VecDeque;
use std::io::{Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct Recorded {
    /// The request line and headers exactly as received (CRLF), without the blank line.
    pub head: String,
    pub method: String,
    pub target: String,
    /// Lower-cased names, values as sent.
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Recorded {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }

    pub fn headers_named(&self, name: &str) -> Vec<&str> {
        self.headers
            .iter()
            .filter(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
            .collect()
    }
}

#[derive(Debug, Clone)]
pub enum Reply {
    Http {
        status: u16,
        headers: Vec<(String, String)>,
        body: Vec<u8>,
        delay: Duration,
    },
    /// Read the whole request, then close without a byte of answer.
    CloseAfterRequest,
    /// Status line and headers at once (Content-Length = body.len()), then the body one byte per `per_byte`.
    Trickle {
        status: u16,
        body: Vec<u8>,
        per_byte: Duration,
    },
    /// Status line and headers WITHOUT Content-Length, then `count` chunks of `chunk` every `interval` (ends with the close).
    Stream {
        status: u16,
        chunk: Vec<u8>,
        interval: Duration,
        count: usize,
    },
    /// Headers that promise `claimed` bytes, then nothing for `hold`, then close.
    PromiseOnly {
        status: u16,
        claimed: u64,
        hold: Duration,
    },
}

impl Reply {
    pub fn http(status: u16, headers: &[(&str, &str)], body: &[u8]) -> Reply {
        Reply::Http {
            status,
            headers: headers
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            body: body.to_vec(),
            delay: Duration::ZERO,
        }
    }

    pub fn json(status: u16, body: &str) -> Reply {
        Reply::http(
            status,
            &[("Content-Type", "application/json")],
            body.as_bytes(),
        )
    }

    pub fn ok() -> Reply {
        Reply::json(200, "{}")
    }

    /// An `application/problem+json` answer with the members of R-13.
    pub fn problem(status: u16, code: &str) -> Reply {
        Reply::problem_with(status, code, "Title", None, "Do the fix.")
    }

    pub fn problem_with(
        status: u16,
        code: &str,
        title: &str,
        detail: Option<&str>,
        fix: &str,
    ) -> Reply {
        let mut o = serde_json::json!({
            "type": code, "title": title, "status": status, "fix": fix,
            "request_id": "0198d2c0-7000-7000-8000-000000000001"
        });
        if let Some(d) = detail {
            o["detail"] = serde_json::Value::String(d.to_owned());
        }
        Reply::http(
            status,
            &[("Content-Type", "application/problem+json")],
            o.to_string().as_bytes(),
        )
    }

    pub fn header(mut self, name: &str, value: &str) -> Reply {
        if let Reply::Http { headers, .. } = &mut self {
            headers.push((name.to_owned(), value.to_owned()));
        }
        self
    }

    pub fn delayed(mut self, d: Duration) -> Reply {
        if let Reply::Http { delay, .. } = &mut self {
            *delay = d;
        }
        self
    }
}

struct Shared {
    log: Mutex<Vec<Recorded>>,
    script: Mutex<VecDeque<Reply>>,
    fallback: Mutex<Reply>,
    connections: Mutex<usize>,
}

pub struct Fake {
    pub addr: SocketAddr,
    shared: Arc<Shared>,
    stop: Arc<AtomicBool>,
}

impl Fake {
    pub fn start() -> Fake {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind the fake");
        let addr = listener.local_addr().unwrap();
        let shared = Arc::new(Shared {
            log: Mutex::new(Vec::new()),
            script: Mutex::new(VecDeque::new()),
            fallback: Mutex::new(Reply::ok()),
            connections: Mutex::new(0),
        });
        let stop = Arc::new(AtomicBool::new(false));
        let (s2, stop2) = (shared.clone(), stop.clone());
        thread::spawn(move || {
            for conn in listener.incoming() {
                if stop2.load(Ordering::SeqCst) {
                    break;
                }
                if let Ok(stream) = conn {
                    *s2.connections.lock().unwrap() += 1;
                    let s3 = s2.clone();
                    thread::spawn(move || handle(stream, s3));
                }
            }
        });
        Fake { addr, shared, stop }
    }

    /// `http://127.0.0.1:PORT`
    pub fn url(&self) -> String {
        format!("http://{}", self.addr)
    }

    pub fn endpoint(&self) -> cortex_client::Endpoint {
        cortex_client::Endpoint::parse(&self.url()).expect("the fake is a loopback address")
    }

    /// Queue an answer for the next request.
    pub fn push(&self, r: Reply) -> &Fake {
        self.shared.script.lock().unwrap().push_back(r);
        self
    }

    pub fn push_n(&self, n: usize, r: Reply) -> &Fake {
        for _ in 0..n {
            self.push(r.clone());
        }
        self
    }

    /// The answer when the script is empty.
    pub fn set_default(&self, r: Reply) -> &Fake {
        *self.shared.fallback.lock().unwrap() = r;
        self
    }

    pub fn requests(&self) -> Vec<Recorded> {
        self.shared.log.lock().unwrap().clone()
    }

    pub fn count(&self) -> usize {
        self.shared.log.lock().unwrap().len()
    }

    /// Connections accepted, including ones that never sent a request.
    pub fn connections(&self) -> usize {
        *self.shared.connections.lock().unwrap()
    }
}

impl Drop for Fake {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect(self.addr);
    }
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        201 => "Created",
        204 => "No Content",
        301 => "Moved Permanently",
        302 => "Found",
        303 => "See Other",
        307 => "Temporary Redirect",
        308 => "Permanent Redirect",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        409 => "Conflict",
        413 => "Content Too Large",
        422 => "Unprocessable Content",
        428 => "Precondition Required",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        503 => "Service Unavailable",
        _ => "Status",
    }
}

fn read_request(stream: &mut TcpStream) -> Option<Recorded> {
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok()?;
    let mut buf = Vec::new();
    let mut tmp = [0u8; 4096];
    let head_end = loop {
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break i;
        }
        if buf.len() > 64 * 1024 {
            return None;
        }
        let n = stream.read(&mut tmp).ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&tmp[..n]);
    };
    let head = String::from_utf8_lossy(&buf[..head_end]).into_owned();
    let mut lines = head.split("\r\n");
    let request_line = lines.next()?.to_owned();
    let mut parts = request_line.split(' ');
    let method = parts.next()?.to_owned();
    let target = parts.next()?.to_owned();
    let headers: Vec<(String, String)> = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_owned()))
        .collect();
    let len: usize = headers
        .iter()
        .find(|(k, _)| k == "content-length")
        .and_then(|(_, v)| v.parse().ok())
        .unwrap_or(0);
    let mut body = buf[head_end + 4..].to_vec();
    while body.len() < len {
        let n = stream.read(&mut tmp).ok()?;
        if n == 0 {
            break;
        }
        body.extend_from_slice(&tmp[..n]);
    }
    Some(Recorded {
        head,
        method,
        target,
        headers,
        body,
    })
}

fn handle(mut stream: TcpStream, shared: Arc<Shared>) {
    let Some(req) = read_request(&mut stream) else {
        return;
    };
    shared.log.lock().unwrap().push(req);
    let reply = shared
        .script
        .lock()
        .unwrap()
        .pop_front()
        .unwrap_or_else(|| shared.fallback.lock().unwrap().clone());
    match reply {
        Reply::CloseAfterRequest => {
            let _ = stream.shutdown(Shutdown::Both);
        }
        Reply::Http {
            status,
            headers,
            body,
            delay,
        } => {
            thread::sleep(delay);
            let mut out = format!("HTTP/1.1 {status} {}\r\n", reason(status));
            for (k, v) in &headers {
                out.push_str(&format!("{k}: {v}\r\n"));
            }
            out.push_str(&format!(
                "Content-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            ));
            let mut bytes = out.into_bytes();
            bytes.extend_from_slice(&body);
            let _ = stream.write_all(&bytes);
            let _ = stream.shutdown(Shutdown::Both);
        }
        Reply::Trickle {
            status,
            body,
            per_byte,
        } => {
            let head = format!(
                "HTTP/1.1 {status} {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                reason(status),
                body.len()
            );
            let _ = stream.write_all(head.as_bytes());
            for b in body {
                thread::sleep(per_byte);
                if stream.write_all(&[b]).is_err() {
                    break;
                }
            }
            let _ = stream.shutdown(Shutdown::Both);
        }
        Reply::Stream {
            status,
            chunk,
            interval,
            count,
        } => {
            let head = format!(
                "HTTP/1.1 {status} {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n",
                reason(status)
            );
            let _ = stream.write_all(head.as_bytes());
            for _ in 0..count {
                thread::sleep(interval);
                if stream.write_all(&chunk).is_err() {
                    break;
                }
            }
            let _ = stream.shutdown(Shutdown::Both);
        }
        Reply::PromiseOnly {
            status,
            claimed,
            hold,
        } => {
            let head = format!(
                "HTTP/1.1 {status} {}\r\nContent-Type: application/json\r\nContent-Length: {claimed}\r\nConnection: close\r\n\r\n",
                reason(status)
            );
            let _ = stream.write_all(head.as_bytes());
            thread::sleep(hold);
            let _ = stream.shutdown(Shutdown::Both);
        }
    }
}
