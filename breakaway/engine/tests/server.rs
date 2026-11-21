//! End-to-end tests against the real server binary over TCP.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

struct Server {
    child: Child,
    addr: String,
}

fn data_dir(name: &str) -> String {
    let d = std::env::temp_dir().join(format!("breakaway-srv-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d.to_string_lossy().into()
}

fn start(data: &str, speed: &str) -> Server {
    let manifest = env!("CARGO_MANIFEST_DIR");
    let mut child = Command::new(env!("CARGO_BIN_EXE_server"))
        .args(["--port", "0", "--data", data, "--fair", &format!("{manifest}/data/fair"), "--web", &format!("{manifest}/../web"), "--speed", speed])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("start server");
    let mut line = String::new();
    BufReader::new(child.stdout.take().unwrap()).read_line(&mut line).unwrap();
    let addr = line.trim().strip_prefix("listening on http://").expect("listening line").to_string();
    Server { child, addr }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Minimal HTTP client: one request per connection.
fn req(addr: &str, method: &str, path: &str, token: Option<&str>, body: &str) -> (u16, String) {
    let mut s = TcpStream::connect(addr).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    let auth = token.map(|t| format!("Authorization: Bearer {t}\r\n")).unwrap_or_default();
    write!(s, "{method} {path} HTTP/1.1\r\nHost: x\r\n{auth}Content-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
    let mut raw = String::new();
    s.read_to_string(&mut raw).unwrap();
    let code = raw[9..12].parse().unwrap();
    let body = raw.split_once("\r\n\r\n").map(|x| x.1.to_string()).unwrap_or_default();
    (code, body)
}

fn field<'a>(json: &'a str, key: &str) -> &'a str {
    let k = format!("\"{key}\":");
    let rest = &json[json.find(&k).unwrap_or_else(|| panic!("{key} in {json}")) + k.len()..];
    let rest = rest.trim_start_matches('"');
    let end = rest.find(|c: char| c == ',' || c == '}' || c == '"').unwrap();
    &rest[..end]
}

fn join(addr: &str) -> String {
    let (code, body) = req(addr, "POST", "/api/join", None, "");
    assert_eq!(code, 201, "{body}");
    field(&body, "token").to_string()
}

#[test]
fn trading_flow_and_validation() {
    let srv = start(&data_dir("flow"), "0.001");
    let a = &srv.addr;
    let t = join(a);
    let (code, body) = req(a, "POST", "/api/orders", Some(&t), r#"{"side":"buy","price":20,"qty":10}"#);
    assert_eq!(code, 200, "{body}");
    assert!(body.contains("\"rested\""));
    let id = field(&body, "order_id").to_string();

    let (_, me) = req(a, "GET", "/api/me", Some(&t), "");
    assert_eq!(field(&me, "cash_reserved"), "200");
    assert_eq!(field(&me, "cash"), "99800");

    assert_eq!(req(a, "POST", "/api/orders", Some(&t), r#"{"side":"buy","price":90,"qty":5000}"#).0, 422, "risk check");
    assert_eq!(req(a, "POST", "/api/orders", Some(&t), r#"{"side":"hold","price":20,"qty":1}"#).0, 400);
    assert_eq!(req(a, "POST", "/api/orders", Some(&t), r#"{"side":"buy","price":0,"qty":1}"#).0, 422);
    assert_eq!(req(a, "POST", "/api/orders", Some(&t), r#"{"side":"buy","price":20,"qty":1,"qty":2}"#).0, 400, "duplicate keys");
    assert_eq!(req(a, "POST", "/api/orders", None, r#"{"side":"buy","price":20,"qty":1}"#).0, 401);
    assert_eq!(req(a, "POST", "/api/orders", Some("deadbeef"), r#"{"side":"buy","price":20,"qty":1}"#).0, 401);

    let other = join(a);
    assert_eq!(req(a, "DELETE", &format!("/api/orders/{id}"), Some(&other), "").0, 404, "can't cancel someone else's order");
    let (code, body) = req(a, "DELETE", &format!("/api/orders/{id}"), Some(&t), "");
    assert_eq!(code, 200, "{body}");
    assert_eq!(field(&req(a, "GET", "/api/me", Some(&t), "").1, "cash"), "100000");

    assert_eq!(req(a, "GET", "/../Cargo.toml", None, "").0, 404, "no path escape");
    assert_eq!(req(a, "GET", "/", None, "").0, 200);
    assert_eq!(req(a, "PUT", "/api/orders", Some(&t), "").0, 405);
    let (_, state) = req(a, "GET", "/api/state", None, "");
    assert_eq!(field(&state, "check"), "ok");
}

#[test]
fn rate_limit() {
    let srv = start(&data_dir("rate"), "0.001");
    let t = join(&srv.addr);
    let mut limited = 0;
    for _ in 0..40 {
        if req(&srv.addr, "POST", "/api/orders", Some(&t), r#"{"side":"buy","price":2,"qty":1}"#).0 == 429 {
            limited += 1;
        }
    }
    assert!(limited > 0, "40 orders in well under a second must hit the limit");
}

#[test]
fn hard_kill_and_restart_recovers_identical_state() {
    let dir = data_dir("crash");
    let (state, me_a, me_b, ta, tb);
    {
        // clock effectively frozen, so nothing changes between our snapshot and the kill
        let srv = start(&dir, "0.0001");
        let a = &srv.addr;
        ta = join(a);
        tb = join(a);
        for p in [40, 45, 50] {
            req(a, "POST", "/api/orders", Some(&ta), &format!(r#"{{"side":"sell","price":{p},"qty":6}}"#));
        }
        req(a, "POST", "/api/orders", Some(&tb), r#"{"side":"buy","price":46,"qty":9,"tif":"ioc"}"#);
        req(a, "POST", "/api/orders", Some(&tb), r#"{"side":"buy","price":20,"qty":5}"#);
        state = req(a, "GET", "/api/state", None, "").1;
        me_a = req(a, "GET", "/api/me", Some(&ta), "").1;
        me_b = req(a, "GET", "/api/me", Some(&tb), "").1;
        assert_eq!(field(&me_b, "yes"), "9", "the IOC buy filled 6 @ 40 and 3 @ 45");
        // dropping `srv` sends SIGKILL: no graceful shutdown, no final flush
    }
    let srv = start(&dir, "0.0001");
    let a = &srv.addr;
    let state2 = req(a, "GET", "/api/state", None, "").1;
    assert_eq!(field(&state2, "state_hash"), field(&state, "state_hash"), "book identical after replay");
    assert_eq!(field(&state2, "journal_entries"), field(&state, "journal_entries"));
    assert_eq!(field(&state2, "escrow"), field(&state, "escrow"));
    assert_eq!(field(&state2, "check"), "ok");
    // tokens survive, and so does every account, order and share
    assert_eq!(req(a, "GET", "/api/me", Some(&ta), "").1, me_a);
    assert_eq!(req(a, "GET", "/api/me", Some(&tb), "").1, me_b);
    // and the recovered exchange keeps trading
    assert_eq!(req(a, "POST", "/api/orders", Some(&tb), r#"{"side":"buy","price":50,"qty":2}"#).0, 200);
}

#[test]
fn many_concurrent_traders_keep_the_books_balanced() {
    let srv = start(&data_dir("concurrent"), "5");
    let addr = srv.addr.clone();
    let handles: Vec<_> = (0..16)
        .map(|i| {
            let addr = addr.clone();
            std::thread::spawn(move || {
                let t = join(&addr);
                for k in 0..30u32 {
                    let side = if (i + k) % 2 == 0 { "buy" } else { "sell" };
                    let price = 35 + (i * 7 + k * 3) % 30;
                    let tif = if k % 4 == 0 { "ioc" } else { "gtc" };
                    let body = format!(r#"{{"side":"{side}","price":{price},"qty":{},"tif":"{tif}"}}"#, 1 + k % 9);
                    let (code, _) = req(&addr, "POST", "/api/orders", Some(&t), &body);
                    assert!(matches!(code, 200 | 422 | 429), "unexpected {code}");
                    std::thread::sleep(Duration::from_millis(60));
                }
            })
        })
        .collect();
    for h in handles {
        h.join().unwrap();
    }
    let (_, state) = req(&addr, "GET", "/api/state", None, "");
    assert_eq!(field(&state, "check"), "ok", "{state}");
}
