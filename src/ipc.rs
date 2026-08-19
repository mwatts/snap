use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;

#[derive(Debug, thiserror::Error)]
pub enum IpcError {
    #[error("daemon is not running")]
    NotRunning,
    #[error("{0}")]
    Io(String),
    #[error("invalid ipc: {0}")]
    Codec(String),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "lowercase")]
pub enum Request {
    Ping,
    Capture,
    Use { path: String },
    Status,
    Shutdown,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Response {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl Response {
    pub fn ok(data: serde_json::Value) -> Self {
        Self {
            ok: true,
            data: Some(data),
            error: None,
        }
    }

    pub fn err(msg: impl Into<String>) -> Self {
        Self {
            ok: false,
            data: None,
            error: Some(msg.into()),
        }
    }
}

pub fn read_request(stream: &mut impl BufRead) -> Result<Request, IpcError> {
    let mut line = String::new();
    stream
        .read_line(&mut line)
        .map_err(|e| IpcError::Io(e.to_string()))?;
    serde_json::from_str(line.trim()).map_err(|e| IpcError::Codec(e.to_string()))
}

pub fn write_response(stream: &mut impl Write, resp: &Response) -> Result<(), IpcError> {
    let line = serde_json::to_string(resp).map_err(|e| IpcError::Codec(e.to_string()))?;
    writeln!(stream, "{line}").map_err(|e| IpcError::Io(e.to_string()))
}

pub fn send_request(sock: &Path, req: &Request) -> Result<Response, IpcError> {
    let mut stream = UnixStream::connect(sock).map_err(|_| IpcError::NotRunning)?;
    let line = serde_json::to_string(req).map_err(|e| IpcError::Codec(e.to_string()))?;
    writeln!(stream, "{line}").map_err(|e| IpcError::Io(e.to_string()))?;
    let mut reply = String::new();
    BufReader::new(&stream)
        .read_line(&mut reply)
        .map_err(|e| IpcError::Io(e.to_string()))?;
    serde_json::from_str(reply.trim()).map_err(|e| IpcError::Codec(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixListener;
    use std::thread;

    #[test]
    fn request_tag_is_op() {
        let v = serde_json::to_value(Request::Ping).unwrap();
        assert_eq!(v, serde_json::json!({"op":"ping"}));
        let v = serde_json::to_value(Request::Use { path: "/tmp/r".into() }).unwrap();
        assert_eq!(v, serde_json::json!({"op":"use","path":"/tmp/r"}));
    }

    #[test]
    fn send_request_round_trips() {
        let tmp = tempfile::tempdir().unwrap();
        let sock = tmp.path().join("s.sock");
        let listener = UnixListener::bind(&sock).unwrap();
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut line = String::new();
            BufReader::new(&stream).read_line(&mut line).unwrap();
            let req: Request = serde_json::from_str(line.trim()).unwrap();
            assert_eq!(req, Request::Ping);
            let resp = Response::ok(serde_json::json!({"pid": 1}));
            writeln!(stream, "{}", serde_json::to_string(&resp).unwrap()).unwrap();
        });
        let resp = send_request(&sock, &Request::Ping).unwrap();
        assert!(resp.ok);
        assert_eq!(resp.data.unwrap()["pid"], 1);
        handle.join().unwrap();
    }
}
