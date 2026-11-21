//! A small, strict HTTP/1.1 implementation on std::net: enough for a JSON API,
//! static files and Server-Sent Events. Limits are deliberately tight.

use std::collections::HashMap;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::{Component, Path, PathBuf};

pub const MAX_HEADER_BYTES: usize = 8 * 1024;
pub const MAX_BODY_BYTES: usize = 4 * 1024;

pub struct Request {
    pub method: String,
    pub path: String,
    pub headers: HashMap<String, String>,
    pub body: Vec<u8>,
}

impl Request {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).map(|s| s.as_str())
    }
    pub fn keep_alive(&self) -> bool {
        !self.header("connection").map_or(false, |v| v.eq_ignore_ascii_case("close"))
    }
    pub fn bearer(&self) -> Option<&str> {
        self.header("authorization")?.strip_prefix("Bearer ")
    }
}

pub enum ReadError {
    Closed,
    Bad(&'static str, u16),
    Io(#[allow(dead_code)] io::Error),
}

/// Reads one request. `Closed` on a clean end of stream between requests.
pub fn read_request(r: &mut BufReader<TcpStream>) -> Result<Request, ReadError> {
    let mut head = Vec::with_capacity(512);
    loop {
        let n = r.by_ref().take((MAX_HEADER_BYTES - head.len()) as u64 + 1).read_until(b'\n', &mut head).map_err(ReadError::Io)?;
        if n == 0 {
            return if head.is_empty() { Err(ReadError::Closed) } else { Err(ReadError::Bad("truncated request", 400)) };
        }
        if head.len() > MAX_HEADER_BYTES {
            return Err(ReadError::Bad("headers too large", 431));
        }
        if head.ends_with(b"\r\n\r\n") || head.ends_with(b"\n\n") {
            break;
        }
    }
    let text = std::str::from_utf8(&head).map_err(|_| ReadError::Bad("headers are not UTF-8", 400))?;
    let mut lines = text.split("\r\n").flat_map(|l| l.split('\n'));
    let start = lines.next().unwrap_or("");
    let mut parts = start.split(' ');
    let (method, target, version) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""), parts.next().unwrap_or(""));
    if method.is_empty() || !target.starts_with('/') || !version.starts_with("HTTP/1.") {
        return Err(ReadError::Bad("malformed request line", 400));
    }
    let mut headers = HashMap::new();
    for line in lines {
        if line.is_empty() {
            continue;
        }
        let (k, v) = line.split_once(':').ok_or(ReadError::Bad("malformed header", 400))?;
        headers.insert(k.trim().to_ascii_lowercase(), v.trim().to_string());
    }
    if headers.get("transfer-encoding").is_some() {
        return Err(ReadError::Bad("chunked bodies are not supported", 411));
    }
    let len: usize = match headers.get("content-length") {
        Some(v) => v.parse().map_err(|_| ReadError::Bad("bad content-length", 400))?,
        None => 0,
    };
    if len > MAX_BODY_BYTES {
        return Err(ReadError::Bad("body too large", 413));
    }
    let mut body = vec![0; len];
    r.read_exact(&mut body).map_err(ReadError::Io)?;
    let path = target.split('?').next().unwrap_or("/").to_string();
    Ok(Request { method: method.to_string(), path, headers, body })
}

pub fn status_text(code: u16) -> &'static str {
    match code {
        200 => "OK",
        201 => "Created",
        204 => "No Content",
        400 => "Bad Request",
        401 => "Unauthorized",
        404 => "Not Found",
        405 => "Method Not Allowed",
        409 => "Conflict",
        411 => "Length Required",
        413 => "Payload Too Large",
        429 => "Too Many Requests",
        431 => "Request Header Fields Too Large",
        503 => "Service Unavailable",
        _ => "Internal Server Error",
    }
}

pub fn respond(w: &mut TcpStream, code: u16, ctype: &str, body: &[u8], keep_alive: bool) -> io::Result<()> {
    let head = format!(
        "HTTP/1.1 {code} {}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nConnection: {}\r\n\r\n",
        status_text(code),
        body.len(),
        if keep_alive { "keep-alive" } else { "close" }
    );
    let mut buf = Vec::with_capacity(head.len() + body.len());
    buf.extend_from_slice(head.as_bytes());
    buf.extend_from_slice(body);
    w.write_all(&buf)
}

pub fn json(w: &mut TcpStream, code: u16, body: &str, keep_alive: bool) -> io::Result<()> {
    respond(w, code, "application/json", body.as_bytes(), keep_alive)
}

pub fn error(w: &mut TcpStream, code: u16, msg: &str, keep_alive: bool) -> io::Result<()> {
    json(w, code, &format!("{{\"error\":{}}}", quote(msg)), keep_alive)
}

/// Resolve a URL path inside `root`, refusing anything that escapes it.
pub fn static_path(root: &Path, url: &str) -> Option<PathBuf> {
    let rel = match url {
        "/" => "index.html",
        "/trade" => "trade.html",
        u => u.trim_start_matches('/'),
    };
    let mut p = root.to_path_buf();
    for c in Path::new(rel).components() {
        match c {
            Component::Normal(s) => p.push(s),
            _ => return None, // "..", absolute paths, prefixes
        }
    }
    p.is_file().then_some(p)
}

pub fn content_type(p: &Path) -> &'static str {
    match p.extension().and_then(|e| e.to_str()).unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "js" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" => "application/json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        _ => "application/octet-stream",
    }
}

// ---------------------------------------------------------------- tiny JSON
pub fn quote(s: &str) -> String {
    let mut o = String::with_capacity(s.len() + 2);
    o.push('"');
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            c if (c as u32) < 0x20 => o.push_str(&format!("\\u{:04x}", c as u32)),
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

/// Parses a flat JSON object of string / number / boolean values, which is all
/// the API accepts. Returns values as their raw text (strings unquoted).
pub fn parse_flat(body: &[u8]) -> Option<HashMap<String, String>> {
    let s = std::str::from_utf8(body).ok()?.trim();
    let inner = s.strip_prefix('{')?.strip_suffix('}')?;
    let mut out = HashMap::new();
    let b = inner.as_bytes();
    let mut i = 0;
    let skip_ws = |i: &mut usize| {
        while *i < b.len() && (b[*i] as char).is_ascii_whitespace() {
            *i += 1
        }
    };
    let read_str = |i: &mut usize| -> Option<String> {
        if b.get(*i) != Some(&b'"') {
            return None;
        }
        *i += 1;
        let start = *i;
        while *i < b.len() && b[*i] != b'"' {
            if b[*i] == b'\\' {
                return None; // no escapes needed by this API
            }
            *i += 1;
        }
        let v = std::str::from_utf8(&b[start..*i]).ok()?.to_string();
        *i += 1;
        Some(v)
    };
    loop {
        skip_ws(&mut i);
        if i >= b.len() {
            break;
        }
        let key = read_str(&mut i)?;
        skip_ws(&mut i);
        if b.get(i) != Some(&b':') {
            return None;
        }
        i += 1;
        skip_ws(&mut i);
        let val = if b.get(i) == Some(&b'"') {
            read_str(&mut i)?
        } else {
            let start = i;
            while i < b.len() && b[i] != b',' && !(b[i] as char).is_ascii_whitespace() {
                i += 1;
            }
            let raw = std::str::from_utf8(&b[start..i]).ok()?;
            if raw.is_empty() || !(raw == "true" || raw == "false" || raw.parse::<f64>().is_ok()) {
                return None;
            }
            raw.to_string()
        };
        if out.insert(key, val).is_some() {
            return None; // duplicate key
        }
        skip_ws(&mut i);
        match b.get(i) {
            Some(b',') => i += 1,
            None => break,
            _ => return None,
        }
    }
    Some(out)
}
