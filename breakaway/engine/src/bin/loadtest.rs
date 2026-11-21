//! End-to-end load test for the server.
//!
//!     server --rate 0 &            (rate limit off)
//!     loadtest 127.0.0.1:8080 [clients=32] [seconds=10]
//!
//! Each client joins, then sends orders back to back over one keep-alive
//! connection (a mix of resting limits around 50c and crossing IOCs), timing
//! every request from write to complete response. Every reply is durable:
//! the server fsyncs its journal before answering.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

fn request(r: &mut BufReader<TcpStream>, w: &mut TcpStream, method: &str, path: &str, token: &str, body: &str) -> (u16, String) {
    let auth = if token.is_empty() { String::new() } else { format!("Authorization: Bearer {token}\r\n") };
    write!(w, "{method} {path} HTTP/1.1\r\nHost: x\r\n{auth}Content-Length: {}\r\n\r\n{body}", body.len()).unwrap();
    let mut line = String::new();
    r.read_line(&mut line).unwrap();
    let code = line[9..12].parse().unwrap();
    let mut len = 0;
    loop {
        line.clear();
        r.read_line(&mut line).unwrap();
        if line == "\r\n" {
            break;
        }
        if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
            len = v.trim().parse().unwrap();
        }
    }
    let mut buf = vec![0; len];
    r.read_exact(&mut buf).unwrap();
    (code, String::from_utf8(buf).unwrap())
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let addr = args.first().cloned().unwrap_or_else(|| "127.0.0.1:8080".into());
    let clients: usize = args.get(1).and_then(|v| v.parse().ok()).unwrap_or(32);
    let secs: u64 = args.get(2).and_then(|v| v.parse().ok()).unwrap_or(10);
    let deadline = Instant::now() + Duration::from_secs(secs);
    let started = Instant::now();
    let handles: Vec<_> = (0..clients)
        .map(|i| {
            let addr = addr.clone();
            std::thread::spawn(move || {
                let s = TcpStream::connect(&addr).unwrap();
                s.set_nodelay(true).unwrap();
                let mut w = s.try_clone().unwrap();
                let mut r = BufReader::new(s);
                let (_, j) = request(&mut r, &mut w, "POST", "/api/join", "", "");
                let token = j.split("\"token\":\"").nth(1).unwrap().split('"').next().unwrap().to_string();
                let mut lat = Vec::with_capacity(100_000);
                let mut codes = [0u64; 600];
                let mut k = 0u64;
                while Instant::now() < deadline {
                    k += 1;
                    let cross = k % 5 == 0;
                    let side = if (i as u64 + k) % 2 == 0 { "buy" } else { "sell" };
                    let price = if cross { if side == "buy" { 60 } else { 40 } } else if side == "buy" { 44 + k % 5 } else { 52 + k % 5 };
                    let tif = if cross { "ioc" } else { "gtc" };
                    let body = format!(r#"{{"side":"{side}","price":{price},"qty":1,"tif":"{tif}"}}"#);
                    let t0 = Instant::now();
                    let (code, _) = request(&mut r, &mut w, "POST", "/api/orders", &token, &body);
                    lat.push(t0.elapsed().as_micros() as u32);
                    codes[code as usize] += 1;
                }
                (lat, codes)
            })
        })
        .collect();
    let mut lat = Vec::new();
    let mut codes = [0u64; 600];
    for h in handles {
        let (l, c) = h.join().unwrap();
        lat.extend(l);
        for (a, b) in codes.iter_mut().zip(c) {
            *a += b;
        }
    }
    let elapsed = started.elapsed().as_secs_f64();
    lat.sort_unstable();
    let p = |q: f64| lat[((lat.len() - 1) as f64 * q) as usize];
    println!("Breakaway server load test: {clients} clients, {secs} s, one keep-alive connection each");
    println!("  requests    {} ({:.0}/s)", lat.len(), lat.len() as f64 / elapsed);
    println!("  responses   200: {}, 422 (risk): {}, 429: {}, other: {}", codes[200], codes[422], codes[429], lat.len() as u64 - codes[200] - codes[422] - codes[429]);
    println!("  round trip  p50 {} µs, p90 {} µs, p99 {} µs, p99.9 {} µs, max {} µs", p(0.5), p(0.9), p(0.99), p(0.999), lat[lat.len() - 1]);
}
