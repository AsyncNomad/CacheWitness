//! Minimal RESP2/RESP3 support for CacheWitness.
//!
//! [`parse`] is an incremental parser over a byte buffer; [`encode`] writes a
//! command as an array of bulk strings; [`Client`] is a small async client
//! that sends one command at a time. It is enough for the verify runner's
//! sync protocol (docs/design/verify-design.md 3) and is the parser the
//! observation engine will share.

use std::io;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

/// A reply from Redis.
#[derive(Debug, Clone, PartialEq)]
pub enum Frame {
    Simple(String),
    Error(String),
    Integer(i64),
    Bulk(Option<Vec<u8>>),
    Array(Option<Vec<Frame>>),
    Null,
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

    pub fn is_nil(&self) -> bool {
        matches!(self, Frame::Null | Frame::Bulk(None) | Frame::Array(None))
    }
}

/// Error for bytes that are not RESP.
#[derive(Debug, Clone, PartialEq)]
pub struct ProtocolError(pub String);

/// Parse one frame from the start of `buf`.
/// `Ok(Some((frame, used)))` when complete, `Ok(None)` when more bytes are needed.
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

/// Encode a command as a RESP array of bulk strings.
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

/// A small async Redis client: one command in flight at a time.
pub struct Client {
    stream: TcpStream,
    buf: Vec<u8>,
}

impl Client {
    /// Connect to `redis://host:port[/db]`.
    pub async fn connect(url: &str) -> io::Result<Client> {
        let rest = url
            .strip_prefix("redis://")
            .ok_or_else(|| io::Error::other(format!("not a redis:// url: {url}")))?;
        let rest = rest.rsplit('@').next().unwrap_or(rest);
        let (hostport, db) = match rest.split_once('/') {
            Some((hp, db)) => (hp, db.parse::<u32>().unwrap_or(0)),
            None => (rest, 0),
        };
        let addr = if hostport.contains(':') {
            hostport.to_string()
        } else {
            format!("{hostport}:6379")
        };
        let stream = TcpStream::connect(addr).await?;
        stream.set_nodelay(true)?;
        let mut client = Client { stream, buf: Vec::new() };
        if db != 0 {
            client.cmd(&["SELECT", &db.to_string()]).await?;
        }
        Ok(client)
    }

    /// Send one command and wait for its reply. Error replies are returned as
    /// `Frame::Error`, not as `Err`.
    pub async fn cmd<A: AsRef<[u8]>>(&mut self, args: &[A]) -> io::Result<Frame> {
        self.stream.write_all(&encode(args)).await?;
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
