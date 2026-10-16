//! Minimal HTTP/1.1 client for scenario requests: one request per
//! connection (`Connection: close`), Content-Length or chunked responses,
//! over plain TCP or TLS (`https://`).

use anyhow::{Context, Result, bail};
use cawit_resp::tls::TlsConfig;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream;

/// An HTTP response.
#[derive(Debug, Clone)]
pub struct Response {
    /// Status code. `0` marks a request that failed and was allowed to
    /// (`allow_error`); the body then holds the error message.
    pub status: u16,
    /// Header names in lower case, in the order received.
    pub headers: Vec<(String, String)>,
    /// Body, with chunked transfer encoding removed.
    pub body: Vec<u8>,
}

impl Response {
    /// The first header named `name` (case-insensitive).
    pub fn header(&self, name: &str) -> Option<&str> {
        let name = name.to_ascii_lowercase();
        self.headers.iter().find(|(k, _)| *k == name).map(|(_, v)| v.as_str())
    }
}

trait Stream: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Stream for T {}

/// Send one request to `base` (`http://host[:port]` or `https://host[:port]`)
/// and read the whole response. `Host`, `Connection: close`, and
/// `Content-Length` are added. Status codes are not interpreted here. `tls`
/// is used for `https://`; without it, certificates are verified against the
/// system trust store.
pub async fn request(
    base: &str,
    method: &str,
    path: &str,
    headers: &[(String, String)],
    body: Option<Vec<u8>>,
    tls: Option<&TlsConfig>,
) -> Result<Response> {
    let (secure, hostport) = if let Some(rest) = base.strip_prefix("https://") {
        (true, rest)
    } else if let Some(rest) = base.strip_prefix("http://") {
        (false, rest)
    } else {
        bail!("app URL must start with http:// or https://: {base}");
    };
    let hostport = hostport.trim_end_matches('/');
    let (host, addr) = match hostport.rsplit_once(':') {
        Some((h, _)) if !hostport.ends_with(']') => (h.trim_matches(['[', ']']).to_string(), hostport.to_string()),
        _ => (
            hostport.trim_matches(['[', ']']).to_string(),
            format!("{hostport}:{}", if secure { 443 } else { 80 }),
        ),
    };
    let tcp = TcpStream::connect(&addr).await.with_context(|| format!("connecting to {addr}"))?;
    let mut stream: Box<dyn Stream> = if secure {
        let config = match tls {
            Some(c) => c.clone(),
            None => TlsConfig::default_roots()?,
        };
        Box::new(
            config
                .connect(&host, tcp)
                .await
                .with_context(|| format!("TLS handshake with {base}"))?,
        )
    } else {
        Box::new(tcp)
    };
    let body = body.unwrap_or_default();
    let mut req = format!(
        "{method} {path} HTTP/1.1\r\nHost: {hostport}\r\nConnection: close\r\nContent-Length: {}\r\n",
        body.len()
    );
    for (k, v) in headers {
        req.push_str(&format!("{k}: {v}\r\n"));
    }
    req.push_str("\r\n");
    stream.write_all(req.as_bytes()).await?;
    stream.write_all(&body).await?;
    stream.flush().await?;
    let mut raw = Vec::new();
    match stream.read_to_end(&mut raw).await {
        Ok(_) => {}
        // Many servers close a TLS connection without close_notify after
        // `Connection: close`; the response is complete by then.
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof && !raw.is_empty() => {}
        Err(e) => return Err(e.into()),
    }
    parse_response(&raw)
}

fn parse_response(raw: &[u8]) -> Result<Response> {
    let Some(split) = raw.windows(4).position(|w| w == b"\r\n\r\n") else {
        bail!("incomplete HTTP response")
    };
    let head = String::from_utf8_lossy(&raw[..split]);
    let mut lines = head.split("\r\n");
    let status: u16 = lines
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|s| s.parse().ok())
        .context("bad HTTP status line")?;
    let headers: Vec<(String, String)> = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string()))
        .collect();
    let chunked = headers
        .iter()
        .any(|(k, v)| k == "transfer-encoding" && v.to_ascii_lowercase().contains("chunked"));
    let rest = &raw[split + 4..];
    let body = if chunked { dechunk(rest)? } else { rest.to_vec() };
    Ok(Response { status, headers, body })
}

fn dechunk(mut b: &[u8]) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    loop {
        let Some(eol) = b.windows(2).position(|w| w == b"\r\n") else {
            bail!("bad chunk header")
        };
        let size_hex = String::from_utf8_lossy(&b[..eol]);
        let size = usize::from_str_radix(size_hex.split(';').next().unwrap_or("").trim(), 16).context("bad chunk size")?;
        b = &b[eol + 2..];
        if size == 0 {
            return Ok(out);
        }
        if b.len() < size + 2 {
            bail!("truncated chunk");
        }
        out.extend_from_slice(&b[..size]);
        b = &b[size + 2..];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_length_and_chunked_bodies() {
        let r = parse_response(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nhi").unwrap();
        assert_eq!((r.status, r.body.as_slice()), (200, &b"hi"[..]));
        assert_eq!(r.header("CONTENT-length"), Some("2"));
        let r = parse_response(b"HTTP/1.1 201 Created\r\nTransfer-Encoding: chunked\r\n\r\n2\r\nhe\r\n3\r\nllo\r\n0\r\n\r\n").unwrap();
        assert_eq!((r.status, r.body.as_slice()), (201, &b"hello"[..]));
    }
}
