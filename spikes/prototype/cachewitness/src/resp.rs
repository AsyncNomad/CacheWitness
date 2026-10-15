//! Minimal incremental RESP2 parser. Parsing is passive: the proxy forwards
//! bytes untouched and only feeds a copy into this parser.

#[derive(Debug, Clone)]
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
    pub fn as_bytes(&self) -> Option<&[u8]> {
        match self {
            Frame::Bulk(Some(b)) => Some(b),
            Frame::Simple(s) => Some(s.as_bytes()),
            _ => None,
        }
    }
}

fn find_crlf(buf: &[u8], start: usize) -> Option<usize> {
    if buf.len() < 2 {
        return None;
    }
    (start..buf.len() - 1).find(|&i| buf[i] == b'\r' && buf[i + 1] == b'\n')
}

/// Returns Ok(Some((frame, consumed))) when a full frame is available,
/// Ok(None) when more bytes are needed, Err on protocol we don't understand.
pub fn parse(buf: &[u8]) -> Result<Option<(Frame, usize)>, ()> {
    parse_at(buf, 0)
}

fn parse_at(buf: &[u8], pos: usize) -> Result<Option<(Frame, usize)>, ()> {
    if pos >= buf.len() {
        return Ok(None);
    }
    let end = match find_crlf(buf, pos + 1) {
        Some(e) => e,
        None => return Ok(None),
    };
    let line = std::str::from_utf8(&buf[pos + 1..end]).map_err(|_| ())?;
    let next = end + 2;
    match buf[pos] {
        b'+' => Ok(Some((Frame::Simple(line.to_string()), next))),
        b'-' => Ok(Some((Frame::Error(line.to_string()), next))),
        // RESP3 scalars
        b'_' => Ok(Some((Frame::Null, next))),
        b'#' | b',' | b'(' => Ok(Some((Frame::Simple(line.to_string()), next))),
        b'!' | b'=' => {
            // blob error / verbatim string: length-prefixed like '$'
            let n: usize = line.parse().map_err(|_| ())?;
            if buf.len() < next + n + 2 {
                return Ok(None);
            }
            let body = buf[next..next + n].to_vec();
            if buf[pos] == b'!' {
                Ok(Some((Frame::Error(String::from_utf8_lossy(&body).to_string()), next + n + 2)))
            } else {
                Ok(Some((Frame::Bulk(Some(body)), next + n + 2)))
            }
        }
        // RESP3 aggregates: map/attribute have 2n elements, set/push n elements
        b'%' | b'|' | b'~' | b'>' => {
            let n: usize = line.parse().map_err(|_| ())?;
            let count = if matches!(buf[pos], b'%' | b'|') { n * 2 } else { n };
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
                // an attribute decorates the frame that follows it
                b'|' => parse_at(buf, p),
                b'>' => Ok(Some((Frame::Push(items), p))),
                _ => Ok(Some((Frame::Array(Some(items)), p))),
            }
        }
        b':' => Ok(Some((Frame::Integer(line.parse().map_err(|_| ())?), next))),
        b'$' => {
            let n: i64 = line.parse().map_err(|_| ())?;
            if n < 0 {
                return Ok(Some((Frame::Bulk(None), next)));
            }
            let n = n as usize;
            if buf.len() < next + n + 2 {
                return Ok(None);
            }
            Ok(Some((Frame::Bulk(Some(buf[next..next + n].to_vec())), next + n + 2)))
        }
        b'*' => {
            let n: i64 = line.parse().map_err(|_| ())?;
            if n < 0 {
                return Ok(Some((Frame::Array(None), next)));
            }
            let mut items = Vec::with_capacity(n as usize);
            let mut p = next;
            for _ in 0..n {
                match parse_at(buf, p)? {
                    Some((f, np)) => {
                        items.push(f);
                        p = np;
                    }
                    None => return Ok(None),
                }
            }
            Ok(Some((Frame::Array(Some(items)), p)))
        }
        _ => Err(()),
    }
}

/// Encode a command as a RESP array of bulk strings.
pub fn encode(args: &[&[u8]]) -> Vec<u8> {
    let mut out = format!("*{}\r\n", args.len()).into_bytes();
    for a in args {
        out.extend_from_slice(format!("${}\r\n", a.len()).as_bytes());
        out.extend_from_slice(a);
        out.extend_from_slice(b"\r\n");
    }
    out
}
