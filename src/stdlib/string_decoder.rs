//! Node `string_decoder` core module: `new StringDecoder(encoding)` with
//! `.write(buffer)` / `.end([buffer])`. A StringDecoder turns byte chunks into a
//! string, holding back an incomplete trailing multibyte sequence until the next
//! chunk completes it.
//!
//! Every encoding that has a chunk boundary buffers across it: UTF-8 holds an
//! incomplete trailing sequence, UTF-16LE holds an odd byte and a trailing high
//! surrogate, and base64 holds up to two bytes so it only ever emits whole
//! 3-byte groups. The single-byte encodings consume everything.

use crate::host::{with_host, JsObj};
use fusevm::Value;
use indexmap::IndexMap;

/// The methods a `StringDecoder` instance carries. Also the method set its real
/// prototype object is built from, so a read (`sd.write`), a call (`sd.write(b)`)
/// and a prototype lookup (`StringDecoder.prototype.write`) cannot disagree.
pub const INSTANCE_METHODS: &[&str] = &["write", "end"];

/// Node's `normalizeEncoding`: the `encoding` property reports the CANONICAL
/// name, not the spelling that was passed — `new StringDecoder('ucs2').encoding`
/// is `'utf16le'` and `new StringDecoder('UTF-8').encoding` is `'utf8'`. Code
/// that branches on `decoder.encoding` (iconv-lite does) sees the canonical set.
fn normalize_encoding(enc: &str) -> String {
    match enc.to_ascii_lowercase().as_str() {
        "utf8" | "utf-8" => "utf8",
        "ucs2" | "ucs-2" | "utf16le" | "utf-16le" => "utf16le",
        "latin1" | "binary" => "latin1",
        other => return other.to_string(),
    }
    .to_string()
}

/// `new StringDecoder([encoding])`.
pub fn construct(args: &[Value]) -> Result<Value, String> {
    let enc = if args.is_empty() {
        "utf8".to_string()
    } else {
        super::arg_str(args, 0)
    };
    Ok(with_host(|h| {
        let mut m = IndexMap::new();
        m.insert("@@native".into(), h.new_str("StringDecoder"));
        m.insert("encoding".into(), h.new_str(normalize_encoding(&enc)));
        // Held-back bytes from a UTF-8 sequence split across chunks.
        let empty = h.new_array(Vec::new());
        m.insert("@@pending".into(), empty);
        h.new_object(m)
    }))
}

/// The byte content of a Buffer / typed array / array argument.
fn bytes_of(v: &Value) -> Vec<u8> {
    with_host(|h| match h.get(v) {
        Some(JsObj::Object(p)) => {
            if p.contains_key("@@buffer") {
                return crate::stdlib::typedarray::elems_with_host(h, v)
                    .iter()
                    .map(|x| h.to_number(x) as u8)
                    .collect();
            }
            match p.get("@@bytes").and_then(|a| h.get(a)) {
                Some(JsObj::Array(items)) => items.iter().map(|x| h.to_number(x) as u8).collect(),
                _ => Vec::new(),
            }
        }
        Some(JsObj::Array(items)) => items.iter().map(|x| h.to_number(x) as u8).collect(),
        _ => Vec::new(),
    })
}

fn encoding_of(recv: &Value) -> String {
    with_host(|h| match h.get(recv) {
        Some(JsObj::Object(p)) => p
            .get("encoding")
            .map(|v| h.str_of(v))
            .unwrap_or_else(|| "utf8".into()),
        _ => "utf8".into(),
    })
}

fn pending_of(recv: &Value) -> Vec<u8> {
    with_host(|h| match h.get(recv) {
        Some(JsObj::Object(p)) => match p.get("@@pending").and_then(|a| h.get(a)) {
            Some(JsObj::Array(items)) => items.iter().map(|x| h.to_number(x) as u8).collect(),
            _ => Vec::new(),
        },
        _ => Vec::new(),
    })
}

fn set_pending(recv: &Value, bytes: &[u8]) {
    with_host(|h| {
        let arr = h.new_array(bytes.iter().map(|b| Value::Float(*b as f64)).collect());
        if let Some(JsObj::Object(p)) = h.get_mut(recv) {
            p.insert("@@pending".into(), arr);
        }
    });
}

pub fn instance_call(recv: &Value, method: &str, args: &[Value]) -> Result<Value, String> {
    let enc = encoding_of(recv);
    match method {
        "write" => {
            let mut buf = pending_of(recv);
            buf.extend(bytes_of(&args.first().cloned().unwrap_or(Value::Undef)));
            let (decoded, tail) = decode(&enc, &buf);
            set_pending(recv, &tail);
            Ok(with_host(|h| h.new_str(decoded)))
        }
        "end" => {
            let mut buf = pending_of(recv);
            if let Some(v) = args.first() {
                buf.extend(bytes_of(v));
            }
            set_pending(recv, &[]);
            let (mut decoded, tail) = decode(&enc, &buf);
            decoded.push_str(&flush(&enc, &tail));
            Ok(with_host(|h| h.new_str(decoded)))
        }
        _ => Err(crate::host::type_error(&format!(
            "{method} is not a function"
        ))),
    }
}

/// What `end()` emits for bytes still held when the stream closes. Each encoding
/// resolves its own remainder, so this is not one blanket replacement char:
///
/// * UTF-8 — one `U+FFFD` for the whole truncated sequence (not one per byte).
/// * UTF-16LE — a held HIGH SURROGATE is emitted as a code unit; a dangling odd
///   byte is dropped silently, with no replacement char (measured on node
///   v26.7.0: `d.write(Buffer.from([0x61])); d.end()` is `''`). The lone
///   surrogate itself becomes `U+FFFD` here — the `utf16` storage boundary.
/// * base64 — the short group is padded and emitted (`AQ==`), which is the whole
///   reason the bytes were held rather than encoded early.
fn flush(enc: &str, tail: &[u8]) -> String {
    if tail.is_empty() {
        return String::new();
    }
    match enc {
        "base64" => super::to_base64(tail),
        "base64url" => super::to_base64url(tail),
        "utf16le" => {
            let units: Vec<u16> = tail
                .chunks_exact(2)
                .map(|c| u16::from_le_bytes([c[0], c[1]]))
                .collect();
            crate::utf16::to_string_lossy(&units)
        }
        _ => "\u{FFFD}".to_string(),
    }
}

/// Decode `buf` in `enc`, returning (decoded string, held-back trailing bytes).
/// Single-byte encodings consume everything; the rest hold back the partial tail
/// that only the next chunk can complete.
fn decode(enc: &str, buf: &[u8]) -> (String, Vec<u8>) {
    match enc {
        // `ascii` masks the high bit, `latin1`/`binary` keep the whole byte.
        "ascii" => (
            buf.iter().map(|b| (*b & 0x7f) as char).collect(),
            Vec::new(),
        ),
        "latin1" => (buf.iter().map(|b| *b as char).collect(), Vec::new()),
        "hex" => (super::to_hex(buf), Vec::new()),
        // Base64 is 3 bytes → 4 characters. Emitting a short group would pad it
        // mid-stream (`AQ==` then `AgM=` instead of `AQID`), so the remainder is
        // held until the group closes or `end()` pads it.
        "base64" | "base64url" => {
            let keep = buf.len() % 3;
            let (head, tail) = buf.split_at(buf.len() - keep);
            let s = if enc == "base64url" {
                super::to_base64url(head)
            } else {
                super::to_base64(head)
            };
            (s, tail.to_vec())
        }
        // UTF-16LE: an odd trailing byte is half a code unit, and a trailing HIGH
        // surrogate is half a code point — both wait for the next chunk.
        "utf16le" => {
            let mut keep = buf.len() % 2;
            let whole = buf.len() - keep;
            if whole >= 2 {
                let last = u16::from_le_bytes([buf[whole - 2], buf[whole - 1]]);
                if (0xD800..0xDC00).contains(&last) {
                    keep += 2;
                }
            }
            let (head, tail) = buf.split_at(buf.len() - keep);
            let units: Vec<u16> = head
                .chunks_exact(2)
                .map(|c| u16::from_le_bytes([c[0], c[1]]))
                .collect();
            (crate::utf16::to_string_lossy(&units), tail.to_vec())
        }
        // utf8 / utf-8 (and anything else): keep a split multibyte tail pending.
        _ => {
            let split = incomplete_utf8_tail(buf);
            let (head, tail) = buf.split_at(buf.len() - split);
            (String::from_utf8_lossy(head).into_owned(), tail.to_vec())
        }
    }
}

/// Number of trailing bytes that form an incomplete UTF-8 sequence (0..=3).
fn incomplete_utf8_tail(buf: &[u8]) -> usize {
    // Walk back over continuation bytes (10xxxxxx) to the lead byte.
    let mut i = buf.len();
    let mut cont = 0;
    while i > 0 && buf[i - 1] & 0b1100_0000 == 0b1000_0000 && cont < 3 {
        i -= 1;
        cont += 1;
    }
    if i == 0 {
        return 0;
    }
    let lead = buf[i - 1];
    let needed = if lead & 0b1000_0000 == 0 {
        1
    } else if lead & 0b1110_0000 == 0b1100_0000 {
        2
    } else if lead & 0b1111_0000 == 0b1110_0000 {
        3
    } else if lead & 0b1111_1000 == 0b1111_0000 {
        4
    } else {
        1
    };
    // If the lead + its continuations are all present, nothing is pending.
    if cont + 1 >= needed {
        0
    } else {
        cont + 1
    }
}
