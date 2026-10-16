//! Minimal RESP2/RESP3 support for [CacheWitness](https://github.com/AsyncNomad/CacheWitness).
//!
//! - [`parse`] is an incremental parser over a byte buffer: it returns the
//!   first complete frame and how many bytes it used, or `None` when more
//!   bytes are needed.
//! - [`encode`] writes a command as a RESP array of bulk strings.
//! - [`Client`] is a small async client that sends one command at a time,
//!   with `AUTH`, database selection, and TLS (`rediss://`, see [`tls`]).
//!
//! It is deliberately small: no pooling, no pipelining, no pub/sub, no
//! cluster support. It is what the CacheWitness runner needs for the hook
//! protocol. For a general Redis client use `redis` or `fred`.
//!
//! # Example
//!
//! ```
//! use cawit_resp::{Frame, encode, parse};
//!
//! assert_eq!(encode(&["GET", "k"]), b"*2\r\n$3\r\nGET\r\n$1\r\nk\r\n");
//!
//! // A complete bulk string, followed by the start of the next reply.
//! let buf = b"$5\r\nhello\r\n:4";
//! let (frame, used) = parse(buf).unwrap().unwrap();
//! assert_eq!(frame.as_str().as_deref(), Some("hello"));
//! assert_eq!(used, 11);
//! assert_eq!(parse(&buf[used..]).unwrap(), None); // incomplete
//! ```
//!
//! RESP3 types are folded into the RESP2 shapes: maps and sets become
//! [`Frame::Array`] (a map as alternating keys and values), doubles, booleans
//! and big numbers become [`Frame::Simple`], verbatim strings become
//! [`Frame::Bulk`], and attributes are skipped.

#![warn(missing_docs)]

pub mod tls;

use std::io;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream;

/// A reply from Redis.
#[derive(Debug, Clone, PartialEq)]
pub enum Frame {
    /// Simple string (`+`); also RESP3 doubles, booleans, and big numbers.
    Simple(String),
    /// Error reply (`-`, or RESP3 bulk error `!`).
    Error(String),
    /// Integer (`:`).
    Integer(i64),
    /// Bulk string (`$`, or RESP3 verbatim `=`); `None` is the RESP2 null bulk string.
    Bulk(Option<Vec<u8>>),
    /// Array (`*`), or RESP3 set or map; `None` is the RESP2 null array.
    Array(Option<Vec<Frame>>),
    /// RESP3 null (`_`).
    Null,
    /// RESP3 push message (`>`).
    Push(Vec<Frame>),
}

impl Frame {
    /// The value as UTF-8 text, for simple strings and bulk strings.
    pub fn as_str(&self) -> Option<String> {
        match self {
            Frame::Simple(s) => Some(s.clone()),
            Frame::Bulk(Some(b)) => Some(String::from_utf8_lossy(b).into_owned()),
            _ => None,
        }
    }

    /// Whether this is any form of null: RESP3 null, null bulk string, or null array.
    pub fn is_nil(&self) -> bool {
        matches!(self, Frame::Null | Frame::Bulk(None) | Frame::Array(None))
    }
}

/// Error for bytes that are not RESP. The string describes what was wrong.
#[derive(Debug, Clone, PartialEq)]
pub struct ProtocolError(pub String);

impl std::fmt::Display for ProtocolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "RESP protocol error: {}", self.0)
    }
}

impl std::error::Error for ProtocolError {}

/// Parse one frame from the start of `buf`.
///
/// Returns `Ok(Some((frame, used)))` when a complete frame is available,
/// `Ok(None)` when more bytes are needed, and `Err` when the bytes are not
/// RESP. Nothing is consumed: drop the first `used` bytes yourself.
pub fn parse(buf: &[u8]) -> Result<Option<(Frame, usize)>, ProtocolError> {
    parse_at(buf, 0)
}

fn line_end(buf: &[u8], from: usize) -> Option<usize> {
    if buf.len() < 2 {
        return None;
    }
    (from..buf.len() - 1).find(|&i| buf[i] == b'\r' && buf[i + 1] == b'\n')
}

fn number(line: &[u8]) -> Result<i64, ProtocolError> {
    std::str::from_utf8(line)
        .ok()
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| ProtocolError(format!("bad length {:?}", String::from_utf8_lossy(line))))
}

fn parse_at(buf: &[u8], pos: usize) -> Result<Option<(Frame, usize)>, ProtocolError> {
    if pos >= buf.len() {
        return Ok(None);
    }
    let Some(end) = line_end(buf, pos + 1) else { return Ok(None) };
    let line = &buf[pos + 1..end];
    let next = end + 2;
    let text = || String::from_utf8_lossy(line).into_owned();
    let frame = match buf[pos] {
        b'+' | b'#' | b',' | b'(' => (Frame::Simple(text()), next),
        b'-' => (Frame::Error(text()), next),
        b':' => (Frame::Integer(number(line)?), next),
        b'_' => (Frame::Null, next),
        b'$' | b'!' | b'=' => {
            let n = number(line)?;
            if n < 0 {
                (Frame::Bulk(None), next)
            } else {
                let n = n as usize;
                if buf.len() < next + n + 2 {
                    return Ok(None);
                }
                let body = buf[next..next + n].to_vec();
                let f = if buf[pos] == b'!' {
                    Frame::Error(String::from_utf8_lossy(&body).into_owned())
                } else {
                    Frame::Bulk(Some(body))
                };
                (f, next + n + 2)
            }
        }
        b'*' | b'~' | b'>' | b'%' | b'|' => {
            let n = number(line)?;
            if n < 0 {
                return Ok(Some((Frame::Array(None), next)));
            }
            let count = if matches!(buf[pos], b'%' | b'|') {
                n as usize * 2
            } else {
                n as usize
            };
            let mut items = Vec::with_capacity(count);
            let mut p = next;
            for _ in 0..count {
                match parse_at(buf, p)? {
                    Some((f, np)) => {
                        items.push(f);
                        p = np;
                    }
                    None => return Ok(None),
                }
            }
            match buf[pos] {
                b'|' => return parse_at(buf, p), // attribute decorates the next frame
                b'>' => (Frame::Push(items), p),
                _ => (Frame::Array(Some(items)), p),
            }
        }
        other => return Err(ProtocolError(format!("unexpected type byte {other:#x}"))),
    };
    Ok(Some(frame))
}

/// Encode a command as a RESP array of bulk strings, the form Redis expects
/// from clients.
pub fn encode<A: AsRef<[u8]>>(args: &[A]) -> Vec<u8> {
    let mut out = format!("*{}\r\n", args.len()).into_bytes();
    for a in args {
        let a = a.as_ref();
        out.extend_from_slice(format!("${}\r\n", a.len()).as_bytes());
        out.extend_from_slice(a);
        out.extend_from_slice(b"\r\n");
    }
    out
}

/// A parsed Redis URL: `redis://[[user]:password@]host[:port][/db]`, or
/// `rediss://` for TLS. User and password are percent-decoded.
///
/// ```
/// use cawit_resp::RedisUrl;
///
/// let u = RedisUrl::parse("rediss://app:p%40ss@cache.internal:6380/2").unwrap();
/// assert!(u.tls);
/// assert_eq!((u.host.as_str(), u.port, u.db), ("cache.internal", 6380, 2));
/// assert_eq!((u.user.as_deref(), u.password.as_deref()), (Some("app"), Some("p@ss")));
///
/// let u = RedisUrl::parse("redis://:secret@127.0.0.1").unwrap();
/// assert_eq!((u.user, u.password.as_deref(), u.port), (None, Some("secret"), 6379));
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RedisUrl {
    /// `rediss://`: connect with TLS.
    pub tls: bool,
    /// Host name or IP address (IPv6 without brackets).
    pub host: String,
    /// Port, 6379 by default.
    pub port: u16,
    /// ACL user name, if any.
    pub user: Option<String>,
    /// Password, if any.
    pub password: Option<String>,
    /// Database number, 0 by default.
    pub db: u32,
}

fn percent_decode(s: &str) -> io::Result<String> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            let hex = s
                .get(i + 1..i + 3)
                .ok_or_else(|| io::Error::other(format!("bad percent escape in {s:?}")))?;
            out.push(u8::from_str_radix(hex, 16).map_err(|_| io::Error::other(format!("bad percent escape in {s:?}")))?);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).map_err(|_| io::Error::other(format!("percent escapes in {s:?} are not UTF-8")))
}

/// The URL with its password replaced by `***`, for logs and error messages.
///
/// ```
/// assert_eq!(cawit_resp::redact("redis://app:hunter2@h:6379/1"), "redis://app:***@h:6379/1");
/// assert_eq!(cawit_resp::redact("rediss://secret@h"), "rediss://***@h");
/// assert_eq!(cawit_resp::redact("redis://h"), "redis://h");
/// ```
pub fn redact(url: &str) -> String {
    let Some(scheme_end) = url.find("://").map(|i| i + 3) else {
        return url.to_string();
    };
    let rest = &url[scheme_end..];
    let authority_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let Some(at) = rest[..authority_end].rfind('@') else {
        return url.to_string();
    };
    let userinfo = &rest[..at];
    let masked = match userinfo.split_once(':') {
        Some((user, _)) => format!("{user}:***"),
        None => "***".to_string(),
    };
    format!("{}{masked}{}", &url[..scheme_end], &rest[at..])
}

impl RedisUrl {
    /// Parse a `redis://` or `rediss://` URL.
    pub fn parse(url: &str) -> io::Result<RedisUrl> {
        let bad = |why: &str| io::Error::other(format!("bad Redis URL {:?}: {why}", redact(url)));
        let (tls, rest) = if let Some(r) = url.strip_prefix("rediss://") {
            (true, r)
        } else if let Some(r) = url.strip_prefix("redis://") {
            (false, r)
        } else {
            return Err(bad("expected redis:// or rediss://"));
        };
        let rest = rest.split(['?', '#']).next().unwrap_or("");
        let (userinfo, rest) = match rest.rsplit_once('@') {
            Some((u, r)) => (Some(u), r),
            None => (None, rest),
        };
        let (hostport, db) = match rest.split_once('/') {
            Some((hp, "")) => (hp, 0),
            Some((hp, db)) => (hp, db.parse::<u32>().map_err(|_| bad("the database must be a number"))?),
            None => (rest, 0),
        };
        let (host, port) = if let Some(v6) = hostport.strip_prefix('[') {
            let (h, after) = v6.split_once(']').ok_or_else(|| bad("unclosed [ in host"))?;
            match after.strip_prefix(':') {
                Some(p) => (h, p.parse::<u16>().map_err(|_| bad("bad port"))?),
                None => (h, 6379),
            }
        } else {
            match hostport.rsplit_once(':') {
                Some((h, p)) => (h, p.parse::<u16>().map_err(|_| bad("bad port"))?),
                None => (hostport, 6379),
            }
        };
        if host.is_empty() {
            return Err(bad("missing host"));
        }
        let (user, password) = match userinfo {
            None => (None, None),
            Some(u) => match u.split_once(':') {
                Some((user, pass)) => {
                    let user = if user.is_empty() { None } else { Some(percent_decode(user)?) };
                    (user, Some(percent_decode(pass)?))
                }
                // `redis://secret@host`: a lone userinfo is the password, as in redis-cli.
                None => (None, Some(percent_decode(u)?)),
            },
        };
        Ok(RedisUrl {
            tls,
            host: host.to_string(),
            port,
            user,
            password,
            db,
        })
    }
}

trait Stream: AsyncRead + AsyncWrite + Unpin + Send + Sync {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send + Sync> Stream for T {}

/// A small async Redis client: one connection, one command in flight at a time.
///
/// ```no_run
/// # async fn demo() -> std::io::Result<()> {
/// use cawit_resp::{Client, Frame};
///
/// let mut redis = Client::connect("redis://:secret@127.0.0.1:6379/0").await?;
/// redis.cmd(&["SET", "k", "v"]).await?;
/// assert_eq!(redis.cmd(&["GET", "k"]).await?, Frame::Bulk(Some(b"v".to_vec())));
/// # Ok(())
/// # }
/// ```
pub struct Client {
    stream: Box<dyn Stream>,
    buf: Vec<u8>,
}

impl Client {
    /// Connect to a [`RedisUrl`]: TLS for `rediss://` (verified against the
    /// system trust store), `AUTH` when the URL has a password, `SELECT` when
    /// it names a database other than 0. A failed `AUTH` or `SELECT` is an
    /// error.
    pub async fn connect(url: &str) -> io::Result<Client> {
        Client::connect_with(url, None).await
    }

    /// Like [`Client::connect`], with the TLS configuration to use for
    /// `rediss://` (for a private CA, or no verification).
    pub async fn connect_with(url: &str, tls: Option<&tls::TlsConfig>) -> io::Result<Client> {
        let u = RedisUrl::parse(url)?;
        let tcp = TcpStream::connect((u.host.as_str(), u.port)).await?;
        tcp.set_nodelay(true)?;
        let stream: Box<dyn Stream> = if u.tls {
            let config = match tls {
                Some(c) => c.clone(),
                None => tls::TlsConfig::default_roots()?,
            };
            Box::new(config.connect(&u.host, tcp).await?)
        } else {
            Box::new(tcp)
        };
        let mut client = Client { stream, buf: Vec::new() };
        if let Some(password) = &u.password {
            let reply = match &u.user {
                Some(user) => client.cmd(&["AUTH", user, password]).await?,
                None => client.cmd(&["AUTH", password]).await?,
            };
            if let Frame::Error(e) = reply {
                return Err(io::Error::new(io::ErrorKind::PermissionDenied, format!("Redis AUTH failed: {e}")));
            }
        }
        if u.db != 0 {
            if let Frame::Error(e) = client.cmd(&["SELECT", &u.db.to_string()]).await? {
                return Err(io::Error::other(format!("Redis SELECT {} failed: {e}", u.db)));
            }
        }
        Ok(client)
    }

    /// Send one command and wait for its reply. Error replies are returned as
    /// `Frame::Error`, not as `Err`.
    pub async fn cmd<A: AsRef<[u8]>>(&mut self, args: &[A]) -> io::Result<Frame> {
        self.stream.write_all(&encode(args)).await?;
        self.stream.flush().await?;
        let mut chunk = [0u8; 8192];
        loop {
            match parse(&self.buf).map_err(|e| io::Error::other(e.0))? {
                Some((frame, used)) => {
                    self.buf.drain(..used);
                    return Ok(frame);
                }
                None => {
                    let n = self.stream.read(&mut chunk).await?;
                    if n == 0 {
                        return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "redis closed the connection"));
                    }
                    self.buf.extend_from_slice(&chunk[..n]);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redis_urls() {
        let u = RedisUrl::parse("redis://127.0.0.1").unwrap();
        assert_eq!(
            (u.tls, u.host.as_str(), u.port, u.db, u.user, u.password),
            (false, "127.0.0.1", 6379, 0, None, None)
        );
        let u = RedisUrl::parse("redis://default:pw@h:7000/3?ignored=1").unwrap();
        assert_eq!(
            (u.user.as_deref(), u.password.as_deref(), u.port, u.db),
            (Some("default"), Some("pw"), 7000, 3)
        );
        let u = RedisUrl::parse("redis://secret@h").unwrap();
        assert_eq!((u.user, u.password.as_deref()), (None, Some("secret")));
        let u = RedisUrl::parse("rediss://[::1]:6380/").unwrap();
        assert_eq!((u.tls, u.host.as_str(), u.port, u.db), (true, "::1", 6380, 0));
        let u = RedisUrl::parse("redis://u:a%3Ab%40c@h").unwrap();
        assert_eq!(u.password.as_deref(), Some("a:b@c"));
        for bad in [
            "http://h",
            "redis://",
            "redis://h:port",
            "redis://h/x",
            "redis://u:%zz@h",
            "redis://[::1",
        ] {
            assert!(RedisUrl::parse(bad).is_err(), "{bad} should fail");
        }
    }

    #[test]
    fn parses_scalars_and_aggregates() {
        assert_eq!(parse(b"+OK\r\n").unwrap(), Some((Frame::Simple("OK".into()), 5)));
        assert_eq!(parse(b":42\r\n").unwrap().unwrap().0, Frame::Integer(42));
        assert_eq!(parse(b"$-1\r\n").unwrap().unwrap().0, Frame::Bulk(None));
        assert_eq!(parse(b"_\r\n").unwrap().unwrap().0, Frame::Null);
        assert_eq!(
            parse(b"*2\r\n$1\r\na\r\n:1\r\n").unwrap().unwrap().0,
            Frame::Array(Some(vec![Frame::Bulk(Some(b"a".to_vec())), Frame::Integer(1)]))
        );
        assert!(!parse(b"%1\r\n+k\r\n:1\r\n").unwrap().unwrap().0.is_nil());
    }

    #[test]
    fn waits_for_more_bytes() {
        assert_eq!(parse(b"$5\r\nab").unwrap(), None);
        assert_eq!(parse(b"*2\r\n$1\r\na\r\n").unwrap(), None);
        assert_eq!(parse(b"").unwrap(), None);
    }

    #[test]
    fn attribute_decorates_next_frame() {
        assert_eq!(parse(b"|1\r\n+a\r\n+b\r\n:7\r\n").unwrap().unwrap().0, Frame::Integer(7));
    }

    #[test]
    fn rejects_garbage() {
        assert!(parse(b"?x\r\n").is_err());
    }

    #[test]
    fn encodes_bulk_array() {
        assert_eq!(encode(&["GET", "k"]), b"*2\r\n$3\r\nGET\r\n$1\r\nk\r\n".to_vec());
    }
}
