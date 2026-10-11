use std::io::{self, Write};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("decode error")]
    DecodeError,
    #[error(transparent)]
    Io(#[from] io::Error),
}

impl PartialEq for Error {
    fn eq(&self, other: &Self) -> bool {
        matches!((self, other), (Error::DecodeError, Error::DecodeError))
    }
}

/// Decode date from the buffer that has been encoded in the same way that
/// `bash`'s `printf %q` encodes a string and write it to the writer. If an
/// error is returned garbage may have been written to the buffer.
pub fn printf_q_decode<W: Write>(writer: &mut W, buf: &[u8]) -> Result<(), Error> {
    let data: &[u8] = if buf.starts_with(b"$'") {
        // Strip off `$''` quoting.
        if buf.len() < 3 || !buf.ends_with(b"'") {
            return Err(Error::DecodeError);
        }
        &buf[2..buf.len() - 1]
    } else if buf.starts_with(b"'") {
        // Strip off `''` quoting.
        if buf.len() < 2 || !buf.ends_with(b"'") {
            return Err(Error::DecodeError);
        }
        &buf[1..buf.len() - 1]
    } else {
        buf
    };

    let mut src = 0;
    while src < data.len() {
        match data[src] {
            b'\\' => {
                if src + 1 >= data.len() {
                    return Err(Error::DecodeError);
                }
                let out = match data[src + 1] {
                    c @ (b' ' | b'\\' | b'"' | b'\'' | b'$') => c,
                    b'e' => 0x1b,
                    b'n' => b'\n',
                    b'r' => b'\r',
                    b't' => b'\t',
                    b'v' => 0x0b,
                    _ => return Err(Error::DecodeError),
                };
                writer.write_all(&[out])?;
                src += 2;
            }
            c => {
                writer.write_all(&[c])?;
                src += 1;
            }
        }
    }
    Ok(())
}

/// Decode data from the buffer that has been URL percent encoded and write
/// it to the given buffer. If an error is returned the garbage may have been
/// written to the writer.
pub fn url_percent_decode<W: Write>(writer: &mut W, buf: &[u8]) -> Result<(), Error> {
    let mut src = 0;
    while src < buf.len() {
        match buf[src] {
            b'%' => {
                if src + 2 >= buf.len() {
                    return Err(Error::DecodeError);
                }
                let (Some(hi), Some(lo)) = (hex(buf[src + 1]), hex(buf[src + 2])) else {
                    return Err(Error::DecodeError);
                };
                writer.write_all(&[(hi << 4) | lo])?;
                src += 3;
            }
            c => {
                writer.write_all(&[c])?;
                src += 1;
            }
        }
    }
    Ok(())
}

fn hex(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

/// Is the given character valid in URI percent encoding?
fn is_valid_char(c: u8) -> bool {
    match c {
        b' ' | b';' | b'=' => false,
        _ => (0x20..=0x7e).contains(&c),
    }
}

/// Write data to the writer after URI percent encoding.
pub fn url_percent_encode<W: Write>(writer: &mut W, data: &[u8]) -> io::Result<()> {
    for &c in data {
        if is_valid_char(c) {
            writer.write_all(&[c])?;
        } else {
            write!(writer, "%{c:02X}")?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pq(s: &[u8]) -> Result<Vec<u8>, Error> {
        let mut w = Vec::new();
        printf_q_decode(&mut w, s).map(|()| w)
    }

    fn pd(s: &[u8]) -> Result<Vec<u8>, Error> {
        let mut w = Vec::new();
        url_percent_decode(&mut w, s).map(|()| w)
    }

    #[test]
    fn printf_q_1() {
        assert_eq!(pq(b"bobr\\ kurwa").unwrap(), b"bobr kurwa");
    }

    #[test]
    fn printf_q_2() {
        assert_eq!(pq(b"bobr\\nkurwa").unwrap(), b"bobr\nkurwa");
    }

    #[test]
    fn printf_q_3() {
        assert_eq!(pq(b"bobr\\dkurwa"), Err(Error::DecodeError));
    }

    #[test]
    fn printf_q_4() {
        assert_eq!(pq(b"bobr kurwa\\"), Err(Error::DecodeError));
    }

    #[test]
    fn printf_q_5() {
        assert_eq!(pq(b"$'bobr kurwa'").unwrap(), b"bobr kurwa");
    }

    #[test]
    fn printf_q_6() {
        assert_eq!(pq(b"'bobr kurwa'").unwrap(), b"bobr kurwa");
    }

    #[test]
    fn printf_q_7() {
        assert_eq!(pq(b"$'bobr kurwa"), Err(Error::DecodeError));
    }

    #[test]
    fn printf_q_8() {
        assert_eq!(pq(b"$'"), Err(Error::DecodeError));
    }

    #[test]
    fn printf_q_9() {
        assert_eq!(pq(b"'bobr kurwa"), Err(Error::DecodeError));
    }

    #[test]
    fn printf_q_10() {
        assert_eq!(pq(b"'"), Err(Error::DecodeError));
    }

    #[test]
    fn printf_q_escapes() {
        assert_eq!(
            pq(b"\\e\\r\\t\\v\\\\\\\"\\'\\$").unwrap(),
            b"\x1b\r\t\x0b\\\"'$"
        );
    }

    #[test]
    fn singles_percent() {
        for c in 0..255u32 {
            let lower = format!("%{c:02x}");
            let decoded = pd(lower.as_bytes()).unwrap();
            assert_eq!(decoded, [c as u8]);
            let upper = format!("%{c:02X}");
            let decoded = pd(upper.as_bytes()).unwrap();
            assert_eq!(decoded, [c as u8]);
        }
    }

    #[test]
    fn percent_1() {
        assert_eq!(pd(b"bobr%20kurwa").unwrap(), b"bobr kurwa");
    }

    #[test]
    fn percent_2() {
        assert_eq!(pd(b"bobr%2kurwa"), Err(Error::DecodeError));
    }

    #[test]
    fn percent_3() {
        assert_eq!(pd(b"bobr%kurwa"), Err(Error::DecodeError));
    }

    #[test]
    fn percent_4() {
        assert_eq!(pd(b"bobr%%kurwa"), Err(Error::DecodeError));
    }

    #[test]
    fn percent_5() {
        assert_eq!(pd(b"bobr%20kurwa%20").unwrap(), b"bobr kurwa ");
    }

    #[test]
    fn percent_6() {
        assert_eq!(pd(b"bobr%20kurwa%2"), Err(Error::DecodeError));
    }

    #[test]
    fn percent_7() {
        assert_eq!(pd(b"bobr%20kurwa%"), Err(Error::DecodeError));
    }

    #[test]
    fn percent_encode() {
        let mut w = Vec::new();
        url_percent_encode(&mut w, b"a b;c=d\n\xc3\xa9~").unwrap();
        assert_eq!(String::from_utf8(w).unwrap(), "a%20b%3Bc%3Dd%0A%C3%A9~");
    }
}
