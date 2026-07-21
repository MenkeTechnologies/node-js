//! Node `string_decoder` core module: `new StringDecoder(encoding)` with
//! `.write(buffer)` / `.end([buffer])`. A StringDecoder turns byte chunks into a
//! string, holding back an incomplete trailing multibyte sequence until the next
//! chunk completes it.
//!
//! node-js decodes each chunk whole (buffering a split UTF-8 tail): enough for the
//! iconv-lite `internal` codec that requires this module, and correct for any
//! single-chunk decode.

use crate::host::{with_host, JsObj};
use fusevm::Value;
use indexmap::IndexMap;

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
        m.insert("encoding".into(), h.new_str(enc.to_ascii_lowercase()));
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
            let field = p.get("@@bytes").or_else(|| p.get("@@elems"));
            match field.and_then(|a| h.get(a)) {
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
            // Flush the completed head; a dangling incomplete multibyte sequence
            // becomes a single U+FFFD replacement char (matching Node, which emits
            // one replacement for the whole held-back sequence, not one per byte).
            let (mut decoded, tail) = decode(&enc, &buf);
            if !tail.is_empty() {
                decoded.push('\u{FFFD}');
            }
            Ok(with_host(|h| h.new_str(decoded)))
        }
        _ => Err(crate::host::type_error(&format!(
            "{method} is not a function"
        ))),
    }
}

/// Decode `buf` in `enc`, returning (decoded string, held-back trailing bytes).
/// Only UTF-8 holds back an incomplete trailing sequence; single-byte encodings
/// consume everything.
fn decode(enc: &str, buf: &[u8]) -> (String, Vec<u8>) {
    match enc {
        "ascii" | "latin1" | "binary" => (buf.iter().map(|b| *b as char).collect(), Vec::new()),
        "hex" => (super::to_hex(buf), Vec::new()),
        "base64" | "base64url" => (super::to_base64(buf), Vec::new()),
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
