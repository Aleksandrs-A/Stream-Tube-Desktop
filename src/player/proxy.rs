//! Local streaming proxy for googlevideo URLs.
//!
//! YouTube refuses open-ended range requests on stream URLs (HTTP 403) and
//! throttles large single requests. mpv asks for whole files, so it talks to
//! this proxy instead, which serves mpv's (possibly open-ended) ranges by
//! fetching bounded chunks from YouTube, like yt-dlp's chunked downloader.
use parking_lot::Mutex;
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::OnceLock;

const FIRST_CHUNK: u64 = 1 << 20; // small first chunk: fast start and seeks
const CHUNK: u64 = 8 << 20;

struct Entry {
    url: String,
    len: Option<u64>,
    mime: String,
}

struct Proxy {
    port: u16,
    streams: Mutex<HashMap<u64, Entry>>,
    next: Mutex<u64>,
}

fn proxy() -> Option<&'static Proxy> {
    static P: OnceLock<Option<Proxy>> = OnceLock::new();
    P.get_or_init(|| {
        let listener = TcpListener::bind("127.0.0.1:0").ok()?;
        let port = listener.local_addr().ok()?.port();
        std::thread::Builder::new()
            .name("stream-proxy".into())
            .spawn(move || {
                for conn in listener.incoming().flatten() {
                    std::thread::spawn(move || {
                        let _ = serve(conn);
                    });
                }
            })
            .ok()?;
        Some(Proxy { port, streams: Mutex::new(HashMap::new()), next: Mutex::new(0) })
    })
    .as_ref()
}

/// Returns a local URL that serves `url` to mpv. Non-googlevideo URLs pass through.
pub fn local_url(url: &str, len: Option<u64>, mime: &str) -> String {
    if !url.contains("googlevideo.com") {
        return url.to_string();
    }
    let Some(p) = proxy() else { return url.to_string() };
    let id = {
        let mut n = p.next.lock();
        *n += 1;
        *n
    };
    let mut s = p.streams.lock();
    // keep the table small: old entries belong to videos no longer playing
    if s.len() > 64 {
        let min_keep = id.saturating_sub(32);
        s.retain(|k, _| *k >= min_keep);
    }
    s.insert(id, Entry { url: url.to_string(), len, mime: mime.to_string() });
    format!("http://127.0.0.1:{}/s/{id}", p.port)
}

fn agent() -> &'static ureq::Agent {
    static A: OnceLock<ureq::Agent> = OnceLock::new();
    A.get_or_init(|| {
        ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_connect(Some(std::time::Duration::from_secs(10)))
            .timeout_recv_body(Some(std::time::Duration::from_secs(30)))
            .build()
            .new_agent()
    })
}

/// Learns the total size with a 1-byte request when the API didn't provide it.
fn probe_len(url: &str) -> Option<u64> {
    let r = agent().get(url).header("User-Agent", crate::api::http::BROWSER_UA).header("Range", "bytes=0-0").call().ok()?;
    let cr = r.headers().get("content-range")?.to_str().ok()?.to_string();
    cr.rsplit('/').next()?.trim().parse().ok()
}

fn serve(mut conn: TcpStream) -> std::io::Result<()> {
    conn.set_nodelay(true).ok();
    let mut reader = BufReader::new(conn.try_clone()?);
    let mut request_line = String::new();
    reader.read_line(&mut request_line)?;
    let mut range: Option<(u64, Option<u64>)> = None;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 || line == "\r\n" || line == "\n" {
            break;
        }
        let lower = line.to_ascii_lowercase();
        if let Some(v) = lower.strip_prefix("range:") {
            if let Some(spec) = v.trim().strip_prefix("bytes=") {
                let (a, b) = spec.split_once('-').unwrap_or((spec, ""));
                if let Ok(a) = a.trim().parse::<u64>() {
                    range = Some((a, b.trim().parse::<u64>().ok()));
                }
            }
        }
    }
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("");
    let path = parts.next().unwrap_or("");
    let id: Option<u64> = path.strip_prefix("/s/").and_then(|s| s.parse().ok());
    let Some(p) = proxy() else { return Ok(()) };
    let entry = id.and_then(|id| p.streams.lock().get(&id).map(|e| (e.url.clone(), e.len, e.mime.clone())));
    let Some((url, mut len, mime)) = entry else {
        conn.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")?;
        return Ok(());
    };
    if len.is_none() {
        len = probe_len(&url);
        if let (Some(id), Some(l)) = (id, len) {
            if let Some(e) = p.streams.lock().get_mut(&id) {
                e.len = Some(l);
            }
        }
    }
    let Some(total) = len else {
        conn.write_all(b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")?;
        return Ok(());
    };
    let (start, end) = match range {
        Some((a, b)) => (a.min(total), b.unwrap_or(total - 1).min(total - 1)),
        None => (0, total.saturating_sub(1)),
    };
    if start >= total {
        let h = format!("HTTP/1.1 416 Range Not Satisfiable\r\nContent-Range: bytes */{total}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
        conn.write_all(h.as_bytes())?;
        return Ok(());
    }
    let body_len = end - start + 1;
    let head = if range.is_some() {
        format!(
            "HTTP/1.1 206 Partial Content\r\nContent-Type: {mime}\r\nAccept-Ranges: bytes\r\nContent-Range: bytes {start}-{end}/{total}\r\nContent-Length: {body_len}\r\nConnection: close\r\n\r\n"
        )
    } else {
        format!("HTTP/1.1 200 OK\r\nContent-Type: {mime}\r\nAccept-Ranges: bytes\r\nContent-Length: {body_len}\r\nConnection: close\r\n\r\n")
    };
    conn.write_all(head.as_bytes())?;
    if method == "HEAD" {
        return Ok(());
    }
    let mut pos = start;
    let mut first = true;
    let mut stalls = 0;
    let mut buf = vec![0u8; 64 * 1024];
    while pos <= end {
        let size = if first { FIRST_CHUNK } else { CHUNK };
        first = false;
        let chunk_end = (pos + size - 1).min(end);
        let mut attempt = 0;
        loop {
            let resp = agent()
                .get(&url)
                .header("User-Agent", crate::api::http::BROWSER_UA)
                .header("Range", &format!("bytes={pos}-{chunk_end}"))
                .call();
            if crate::perf::on() {
                match &resp {
                    Ok(r) => eprintln!("[proxy] {pos}-{chunk_end} -> {}", r.status()),
                    Err(e) => eprintln!("[proxy] {pos}-{chunk_end} -> error {e}"),
                }
            }
            match resp {
                Ok(r) if r.status().as_u16() == 206 || r.status().as_u16() == 200 => {
                    let mut body = r.into_body().into_reader();
                    loop {
                        let n = match body.read(&mut buf) {
                            Ok(n) => n,
                            Err(e) => {
                                if crate::perf::on() {
                                    eprintln!("[proxy] upstream read error at {pos}: {e}");
                                }
                                break;
                            }
                        };
                        if n == 0 {
                            break;
                        }
                        // a write error means mpv closed the connection (e.g. it seeked)
                        if let Err(e) = conn.write_all(&buf[..n]) {
                            if crate::perf::on() {
                                eprintln!("[proxy] client closed at {pos}: {e}");
                            }
                            return Err(e);
                        }
                        pos += n as u64;
                    }
                    break;
                }
                _ if attempt < 3 => {
                    attempt += 1;
                    std::thread::sleep(std::time::Duration::from_millis(300 * attempt));
                }
                _ => return Ok(()),
            }
        }
        if pos <= chunk_end {
            // upstream ended early; retry from where we are (but don't spin forever)
            stalls += 1;
            if stalls > 5 {
                return Ok(());
            }
        }
    }
    conn.flush()
}
