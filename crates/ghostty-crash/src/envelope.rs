//! The Sentry Envelope format (port of `crash/sentry_envelope.zig`):
//! <https://develop.sentry.dev/sdk/envelopes/>
//!
//! The envelope is our primary crash report format. It is an open,
//! publicly documented format so reports can be consumed by any system.
//!
//! This implements reading and writing the envelope format. It is
//! incomplete; only what Ghostty needs is implemented.

use std::io::{self, BufRead, Read, Write};

use serde_json::{Map, Value};

/// A JSON object preserving insertion order (Zig: `std.json.ObjectMap`).
pub type ObjectMap = Map<String, Value>;

/// Limit of header lines (1MB, arbitrary choice upstream).
const LINE_LIMIT: u64 = 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("io: {0}")]
    Io(#[from] io::Error),
    #[error("invalid json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("EnvelopeMalformedHeaders")]
    MalformedHeaders,
    #[error("EnvelopeItemMalformedHeaders")]
    ItemMalformedHeaders,
    #[error("EnvelopeItemTypeMissing")]
    ItemTypeMissing,
    #[error("EnvelopeItemLengthMalformed")]
    ItemLengthMalformed,
    #[error("EnvelopeItemPayloadTooShort")]
    ItemPayloadTooShort,
    #[error("EnvelopeItemPayloadNoNewline")]
    ItemPayloadNoNewline,
    /// The header line exceeded the limit (Zig: `StreamTooLong`).
    #[error("StreamTooLong")]
    StreamTooLong,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum DecodeError {
    #[error("MissingRequiredField")]
    MissingRequiredField,
    #[error("InvalidFieldType")]
    InvalidFieldType,
    #[error("UnsupportedType")]
    UnsupportedType,
}

/// The various item types that can be in an envelope. Unknown types take
/// the `Unknown` value.
///
/// <https://develop.sentry.dev/sdk/envelopes/#data-model>
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemType {
    Unknown,
    Event,
    Transaction,
    Attachment,
    Session,
    Sessions,
    Statsd,
    MetricMeta,
    UserFeedback,
    ClientReport,
    ReplayEvent,
    ReplayRecording,
    Profile,
    CheckIn,
}

impl ItemType {
    /// Parse the wire name (Zig `std.meta.stringToEnum`, so the snake_case
    /// field names, including "unknown").
    #[must_use]
    pub fn from_name(s: &str) -> Option<ItemType> {
        Some(match s {
            "unknown" => ItemType::Unknown,
            "event" => ItemType::Event,
            "transaction" => ItemType::Transaction,
            "attachment" => ItemType::Attachment,
            "session" => ItemType::Session,
            "sessions" => ItemType::Sessions,
            "statsd" => ItemType::Statsd,
            "metric_meta" => ItemType::MetricMeta,
            "user_feedback" => ItemType::UserFeedback,
            "client_report" => ItemType::ClientReport,
            "replay_event" => ItemType::ReplayEvent,
            "replay_recording" => ItemType::ReplayRecording,
            "profile" => ItemType::Profile,
            "check_in" => ItemType::CheckIn,
            _ => return None,
        })
    }
}

/// An encoded item: payload is bytes, headers are decoded to a JSON object.
#[derive(Debug, Clone, PartialEq)]
pub struct EncodedItem {
    pub headers: ObjectMap,
    pub item_type: ItemType,
    pub payload: Vec<u8>,
}

/// An arbitrary file attachment.
///
/// <https://develop.sentry.dev/sdk/envelopes/#attachment>
#[derive(Debug, Clone, PartialEq)]
pub struct Attachment {
    /// The name of the uploaded file without a path component.
    pub filename: String,
    /// A special "type" associated with the attachment (`attachment_type`).
    pub attachment_type: Option<String>,
    /// Additional headers for the attachment.
    pub headers_extra: ObjectMap,
    /// The data for the attachment.
    pub payload: Vec<u8>,
}

impl Attachment {
    /// A fresh attachment with no extra headers.
    #[must_use]
    pub fn new(filename: impl Into<String>, payload: Vec<u8>) -> Attachment {
        Attachment {
            filename: filename.into(),
            attachment_type: None,
            headers_extra: ObjectMap::new(),
            payload,
        }
    }

    pub fn decode(item: &EncodedItem) -> Result<Attachment, DecodeError> {
        let filename = match item.headers.get("filename") {
            Some(Value::String(s)) => s.clone(),
            Some(_) => return Err(DecodeError::InvalidFieldType),
            None => return Err(DecodeError::MissingRequiredField),
        };
        let attachment_type = match item.headers.get("attachment_type") {
            Some(Value::String(s)) => Some(s.clone()),
            Some(_) => return Err(DecodeError::InvalidFieldType),
            None => None,
        };
        Ok(Attachment {
            filename,
            attachment_type,
            headers_extra: item.headers.clone(),
            payload: item.payload.clone(),
        })
    }

    pub fn encode(&mut self) -> EncodedItem {
        // Deviation: upstream never writes the required "type" header for
        // attachments built in memory (only parsed ones carry it), which
        // yields an unparseable item. We add it when absent.
        if !self.headers_extra.contains_key("type") {
            self.headers_extra
                .insert("type".into(), Value::String("attachment".into()));
        }
        self.headers_extra
            .insert("filename".into(), Value::String(self.filename.clone()));
        if let Some(v) = &self.attachment_type {
            self.headers_extra
                .insert("attachment_type".into(), Value::String(v.clone()));
        } else {
            self.headers_extra.swap_remove("attachment_type");
        }
        EncodedItem {
            headers: self.headers_extra.clone(),
            item_type: ItemType::Attachment,
            payload: self.payload.clone(),
        }
    }
}

/// An item in the envelope, either encoded or decoded.
#[derive(Debug, Clone, PartialEq)]
pub enum Item {
    Encoded(EncodedItem),
    Attachment(Attachment),
}

impl Item {
    /// Convert the item to an encoded item, modifying it in place.
    pub fn encode(&mut self) -> &EncodedItem {
        if let Item::Attachment(a) = self {
            let enc = a.encode();
            *self = Item::Encoded(enc);
        }
        match self {
            Item::Encoded(e) => e,
            Item::Attachment(_) => unreachable!("just encoded"),
        }
    }

    /// The type of item, whether it is encoded or not.
    #[must_use]
    pub fn item_type(&self) -> ItemType {
        match self {
            Item::Encoded(e) => e.item_type,
            Item::Attachment(_) => ItemType::Attachment,
        }
    }

    /// Decode the item if it is encoded (modifies itself). On failure the
    /// encoded item remains unmodified.
    pub fn decode(&mut self) -> Result<(), DecodeError> {
        let Item::Encoded(enc) = self else {
            return Ok(());
        };
        match enc.item_type {
            ItemType::Attachment => {
                let a = Attachment::decode(enc)?;
                *self = Item::Attachment(a);
                Ok(())
            }
            _ => Err(DecodeError::UnsupportedType),
        }
    }
}

/// A parsed or constructed envelope.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Envelope {
    /// The headers of the envelope decoded into a JSON object.
    pub headers: ObjectMap,
    /// The items in the envelope in the order they're encoded.
    pub items: Vec<Item>,
}

/// Read up to the next `\n` (exclusive), limited to `LINE_LIMIT` bytes
/// (Zig: `streamDelimiterLimit` + `discardDelimiterInclusive`). Returns
/// `StreamTooLong` if the limit is hit before a delimiter or end of stream.
fn read_line<R: BufRead>(r: &mut R) -> Result<Vec<u8>, Error> {
    let mut buf = Vec::new();
    // Read one byte past the limit... Zig errors only if the limit is
    // reached and the next byte isn't the delimiter/EOF.
    r.by_ref().take(LINE_LIMIT).read_until(b'\n', &mut buf)?;
    if buf.last() == Some(&b'\n') {
        buf.pop();
        return Ok(buf);
    }
    if buf.len() as u64 == LINE_LIMIT {
        // Exactly at the limit: fine only if the stream ends or the next
        // byte is the delimiter.
        let next = r.fill_buf()?;
        match next.first() {
            None => {}
            Some(b'\n') => r.consume(1),
            Some(_) => return Err(Error::StreamTooLong),
        }
    }
    Ok(buf)
}

fn trim_ws(b: &[u8]) -> &[u8] {
    let is_ws = |c: &u8| *c == b' ' || *c == b'\t';
    let start = b.iter().position(|c| !is_ws(c)).unwrap_or(b.len());
    let end = b.iter().rposition(|c| !is_ws(c)).map_or(start, |i| i + 1);
    &b[start..end]
}

impl Envelope {
    /// Parse an envelope from a reader. The full envelope is read into memory.
    pub fn parse<R: BufRead>(reader: &mut R) -> Result<Envelope, Error> {
        let headers = Self::parse_header(reader)?;
        let items = Self::parse_items(reader)?;
        Ok(Envelope { headers, items })
    }

    /// Convenience for parsing from a byte slice.
    pub fn parse_slice(mut data: &[u8]) -> Result<Envelope, Error> {
        Self::parse(&mut data)
    }

    fn parse_header<R: BufRead>(reader: &mut R) -> Result<ObjectMap, Error> {
        let line = read_line(reader)?;
        match serde_json::from_slice::<Value>(&line)? {
            Value::Object(m) => Ok(m),
            _ => Err(Error::MalformedHeaders),
        }
    }

    fn parse_items<R: BufRead>(reader: &mut R) -> Result<Vec<Item>, Error> {
        let mut items = Vec::new();
        while let Some(item) = Self::parse_one_item(reader)? {
            items.push(item);
        }
        Ok(items)
    }

    fn parse_one_item<R: BufRead>(reader: &mut R) -> Result<Option<Item>, Error> {
        // The next item must start with a header.
        let line = match read_line(reader) {
            Ok(l) => l,
            Err(Error::StreamTooLong) => return Ok(None),
            Err(e) => return Err(e),
        };
        let line = trim_ws(&line);
        if line.is_empty() {
            return Ok(None);
        }
        let Value::Object(headers) = serde_json::from_slice::<Value>(line)? else {
            return Err(Error::ItemMalformedHeaders);
        };

        // Get the item type
        let typ = match headers.get("type") {
            Some(Value::String(s)) => ItemType::from_name(s).unwrap_or(ItemType::Unknown),
            _ => return Err(Error::ItemTypeMissing),
        };

        // The length is not required. If absent the payload is the next line.
        let len: Option<u64> = match headers.get("length") {
            Some(Value::Number(n)) => match n.as_i64() {
                Some(i) => Some(u64::try_from(i).map_err(|_| Error::ItemLengthMalformed)?),
                None => return Err(Error::ItemLengthMalformed),
            },
            Some(_) => return Err(Error::ItemLengthMalformed),
            None => None,
        };

        let payload = if let Some(len) = len {
            let mut payload = Vec::new();
            let n = reader.by_ref().take(len).read_to_end(&mut payload)?;
            if (n as u64) < len {
                return Err(Error::ItemPayloadTooShort);
            }
            // The next byte must be a newline (or end of stream).
            let next = reader.fill_buf()?;
            match next.first() {
                None => {}
                Some(b'\n') => reader.consume(1),
                Some(_) => return Err(Error::ItemPayloadNoNewline),
            }
            payload
        } else {
            match read_line(reader) {
                Ok(p) => p,
                Err(Error::StreamTooLong) => return Err(Error::ItemPayloadTooShort),
                Err(e) => return Err(e),
            }
        };

        Ok(Some(Item::Encoded(EncodedItem {
            headers,
            item_type: typ,
            payload,
        })))
    }

    /// Serialize the envelope to the given writer. This converts all decoded
    /// items to encoded items.
    pub fn serialize<W: Write>(&mut self, w: &mut W) -> Result<(), Error> {
        // Header line first. Minified: all JSON headers must be one line.
        serde_json::to_writer(&mut *w, &self.headers)?;
        w.write_all(b"\n")?;

        for (idx, item) in self.items.iter_mut().enumerate() {
            if idx > 0 {
                w.write_all(b"\n")?;
            }
            let enc = item.encode();
            serde_json::to_writer(&mut *w, &enc.headers)?;
            w.write_all(b"\n")?;
            w.write_all(&enc.payload)?;
        }
        Ok(())
    }

    /// Serialize into a byte vector.
    pub fn to_bytes(&mut self) -> Result<Vec<u8>, Error> {
        let mut out = Vec::new();
        self.serialize(&mut out)?;
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    const SESSION: &str = r#"{"init":true,"sid":"c148cc2f-5f9f-4231-575c-2e85504d6434","status":"abnormal","errors":0,"started":"2024-08-29T02:38:57.607016Z","duration":0.000343,"attrs":{"release":"0.1.0-HEAD+d37b7d09","environment":"production"}}"#;

    fn session_env() -> String {
        format!("{{}}\n{{\"type\":\"session\",\"length\":218}}\n{SESSION}")
    }

    fn encoded(item: &Item) -> &EncodedItem {
        match item {
            Item::Encoded(e) => e,
            Item::Attachment(_) => panic!("expected encoded"),
        }
    }

    #[test]
    fn envelope_parse() {
        Envelope::parse_slice(b"{}").unwrap();
    }

    #[test]
    fn envelope_parse_session() {
        let v = Envelope::parse_slice(session_env().as_bytes()).unwrap();
        assert_eq!(1, v.items.len());
        assert_eq!(ItemType::Session, encoded(&v.items[0]).item_type);
    }

    #[test]
    fn envelope_parse_multiple() {
        let s = format!(
            "{}\n{}\nABCD",
            session_env(),
            r#"{"type":"attachment","length":4,"filename":"test.txt"}"#
        );
        let v = Envelope::parse_slice(s.as_bytes()).unwrap();
        assert_eq!(2, v.items.len());
        assert_eq!(ItemType::Session, encoded(&v.items[0]).item_type);
        assert_eq!(ItemType::Attachment, encoded(&v.items[1]).item_type);
    }

    #[test]
    fn envelope_parse_multiple_no_length() {
        let s = "{}\n{\"type\":\"session\"}\n{}\n{\"type\":\"attachment\",\"length\":4,\"filename\":\"test.txt\"}\nABCD";
        let v = Envelope::parse_slice(s.as_bytes()).unwrap();
        assert_eq!(2, v.items.len());
        assert_eq!(ItemType::Session, encoded(&v.items[0]).item_type);
        assert_eq!(ItemType::Attachment, encoded(&v.items[1]).item_type);
    }

    #[test]
    fn envelope_parse_end_in_new_line() {
        let s = format!("{}\n", session_env());
        let v = Envelope::parse_slice(s.as_bytes()).unwrap();
        assert_eq!(1, v.items.len());
        assert_eq!(ItemType::Session, encoded(&v.items[0]).item_type);
    }

    #[test]
    fn envelope_parse_attachment() {
        let s = "{}\n{\"type\":\"attachment\",\"length\":4,\"filename\":\"test.txt\"}\nABCD";
        let mut v = Envelope::parse_slice(s.as_bytes()).unwrap();
        assert_eq!(1, v.items.len());
        assert_eq!(ItemType::Attachment, encoded(&v.items[0]).item_type);
        v.items[0].decode().unwrap();
        let Item::Attachment(a) = &v.items[0] else {
            panic!("not decoded");
        };
        assert_eq!("test.txt", a.filename);

        // Serialization test
        let out = v.to_bytes().unwrap();
        assert_eq!(s, String::from_utf8(out).unwrap().trim_matches('\n'));
    }

    #[test]
    fn envelope_serialize_empty() {
        let mut v = Envelope::parse_slice(b"{}").unwrap();
        let out = v.to_bytes().unwrap();
        assert_eq!("{}", String::from_utf8(out).unwrap().trim_matches('\n'));
    }

    #[test]
    fn envelope_serialize_session() {
        let s = session_env();
        let mut v = Envelope::parse_slice(s.as_bytes()).unwrap();
        let out = v.to_bytes().unwrap();
        assert_eq!(s, String::from_utf8(out).unwrap().trim_matches('\n'));
    }

    #[test]
    fn parse_errors() {
        assert!(matches!(
            Envelope::parse_slice(b"[]"),
            Err(Error::MalformedHeaders)
        ));
        assert!(matches!(
            Envelope::parse_slice(b"{}\n{\"length\":1}\nx"),
            Err(Error::ItemTypeMissing)
        ));
        assert!(matches!(
            Envelope::parse_slice(b"{}\n{\"type\":\"event\",\"length\":-1}\nx"),
            Err(Error::ItemLengthMalformed)
        ));
        assert!(matches!(
            Envelope::parse_slice(b"{}\n{\"type\":\"event\",\"length\":9}\nabc"),
            Err(Error::ItemPayloadTooShort)
        ));
        assert!(matches!(
            Envelope::parse_slice(b"{}\n{\"type\":\"event\",\"length\":1}\nabc"),
            Err(Error::ItemPayloadNoNewline)
        ));
    }

    #[test]
    fn unknown_type_and_decode_errors() {
        let mut v = Envelope::parse_slice(b"{}\n{\"type\":\"nope\"}\nx").unwrap();
        assert_eq!(ItemType::Unknown, v.items[0].item_type());
        assert_eq!(Err(DecodeError::UnsupportedType), v.items[0].decode());
        let mut v = Envelope::parse_slice(b"{}\n{\"type\":\"attachment\"}\nx").unwrap();
        assert_eq!(Err(DecodeError::MissingRequiredField), v.items[0].decode());
        // The failed decode leaves the encoded item untouched.
        assert!(matches!(v.items[0], Item::Encoded(_)));
    }

    #[test]
    fn fresh_attachment_round_trip() {
        let mut a = Attachment::new("log.txt", b"hello".to_vec());
        a.attachment_type = Some("event.attachment".into());
        let mut env = Envelope {
            items: vec![Item::Attachment(a)],
            ..Envelope::default()
        };
        let bytes = env.to_bytes().unwrap();
        let mut back = Envelope::parse_slice(&bytes).unwrap();
        back.items[0].decode().unwrap();
        let Item::Attachment(a) = &back.items[0] else {
            panic!()
        };
        assert_eq!("log.txt", a.filename);
        assert_eq!(Some("event.attachment"), a.attachment_type.as_deref());
        assert_eq!(b"hello", a.payload.as_slice());
    }
}
