//! A scriptable HTTP/1.1 server for the tests: each request gets the next scripted reply
//! (default: 202 accepted), and every request is recorded.
#![allow(dead_code, unreachable_pub)]

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

pub const KEY: &str = "honk_ab12cd34ef56_ABCDEFGHIJKLMNOPQRSTUVWXYZ012345";

#[derive(Clone, Debug)]
pub enum Reply {
    Answer {
        status: u16,
        body: String,
        headers: Vec<(String, String)>,
    },
    /// Read the request, then close the connection without answering.
    Drop,
    /// Wait, then reply.
    Delay(Duration, Box<Reply>),
}

impl Reply {
    pub fn accepted(id: &str) -> Reply {
        Reply::json(
            202,
            &format!(
                r#"{{"id":"{id}","status":"accepted","duplicate":false,"received_at":"2026-10-04T12:20:05.123Z"}}"#
            ),
        )
    }

    pub fn duplicate(id: &str) -> Reply {
        Reply::json(
            202,
            &format!(
                r#"{{"id":"{id}","status":"accepted","duplicate":true,"received_at":"2026-10-04T12:20:05.123Z"}}"#
            ),
        )
    }

    pub fn json(status: u16, body: &str) -> Reply {
        Reply::Answer {
            status,
            body: body.to_owned(),
            headers: vec![("Content-Type".into(), "application/json".into())],
        }
    }

    pub fn text(status: u16, body: &str) -> Reply {
        Reply::Answer {
            status,
            body: body.to_owned(),
            headers: vec![("Content-Type".into(), "text/html".into())],
        }
    }

    pub fn error(status: u16, code: &str, message: &str) -> Reply {
        Reply::json(
            status,
            &format!(
                r#"{{"error":{{"code":"{code}","message":"{message}","request_id":"req_test"}}}}"#
            ),
        )
    }

    pub fn header(self, name: &str, value: &str) -> Reply {
        match self {
            Reply::Answer {
                status,
                body,
                mut headers,
            } => {
                headers.push((name.into(), value.into()));
                Reply::Answer {
                    status,
                    body,
                    headers,
                }
            }
            other => other,
        }
    }

    pub fn after(self, delay: Duration) -> Reply {
        Reply::Delay(delay, Box::new(self))
    }
}

#[derive(Clone, Debug)]
pub struct Recorded {
    pub method: String,
    pub path: String,
    pub headers: HashMap<String, String>,
    pub body: String,
}

impl Recorded {
    pub fn header(&self, name: &str) -> &str {
        self.headers
            .get(&name.to_ascii_lowercase())
            .map_or("", String::as_str)
    }

    pub fn json(&self) -> serde_json::Value {
        serde_json::from_str(&self.body).expect("the body is JSON")
    }
}

pub struct Mock {
    pub url: String,
    requests: Arc<Mutex<Vec<Recorded>>>,
}

impl Mock {
    pub async fn start(script: Vec<Reply>) -> Mock {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let script = Arc::new(Mutex::new(VecDeque::from(script)));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let recorded = requests.clone();
        tokio::spawn(async move {
            loop {
                let Ok((socket, _)) = listener.accept().await else {
                    return;
                };
                let (script, recorded) = (script.clone(), recorded.clone());
                tokio::spawn(async move {
                    if let Some(req) = read_request(socket).await {
                        let (socket, rec) = req;
                        recorded.lock().unwrap().push(rec);
                        let reply = script
                            .lock()
                            .unwrap()
                            .pop_front()
                            .unwrap_or_else(|| Reply::accepted("msg_default"));
                        answer(socket, reply).await;
                    }
                });
            }
        });
        Mock { url, requests }
    }

    pub fn requests(&self) -> Vec<Recorded> {
        self.requests.lock().unwrap().clone()
    }

    pub fn count(&self) -> usize {
        self.requests.lock().unwrap().len()
    }
}

async fn read_request(mut socket: TcpStream) -> Option<(TcpStream, Recorded)> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 8192];
    let head_end = loop {
        let n = socket.read(&mut chunk).await.ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&chunk[..n]);
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break i + 4;
        }
    };
    let head = String::from_utf8_lossy(&buf[..head_end]).into_owned();
    let mut lines = head.split("\r\n");
    let mut first = lines.next()?.split(' ');
    let (method, path) = (first.next()?.to_owned(), first.next()?.to_owned());
    let headers: HashMap<String, String> = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_owned()))
        .collect();
    let length: usize = headers
        .get("content-length")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    while buf.len() < head_end + length {
        let n = socket.read(&mut chunk).await.ok()?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
    }
    let body =
        String::from_utf8_lossy(&buf[head_end..(head_end + length).min(buf.len())]).into_owned();
    Some((
        socket,
        Recorded {
            method,
            path,
            headers,
            body,
        },
    ))
}

async fn answer(mut socket: TcpStream, reply: Reply) {
    let mut reply = reply;
    loop {
        match reply {
            Reply::Delay(d, next) => {
                tokio::time::sleep(d).await;
                reply = *next;
            }
            Reply::Drop => return,
            Reply::Answer {
                status,
                body,
                headers,
            } => {
                let mut out = format!(
                    "HTTP/1.1 {status} X\r\nContent-Length: {}\r\nConnection: close\r\n",
                    body.len()
                );
                for (k, v) in headers {
                    out.push_str(&format!("{k}: {v}\r\n"));
                }
                out.push_str("\r\n");
                out.push_str(&body);
                let _ = socket.write_all(out.as_bytes()).await;
                let _ = socket.shutdown().await;
                return;
            }
        }
    }
}

/// A client for the mock with fast backoff.
pub fn client(mock: &Mock) -> honk_me::HonkBuilder {
    honk_me::Honk::builder()
        .url(&mock.url)
        .key(KEY)
        .backoff(Duration::from_millis(1), Duration::from_millis(5))
}

/// A URL where nothing listens.
pub async fn closed_url() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    drop(listener);
    url
}
