//! Minimal BSON reader: enough for the document stream in UniFi's `db.gz`
//! (the same concatenated layout `mongodump` writes).
//!
//! Values map the way unifidump's `extract.py` reads them: ObjectId as a hex
//! string, datetime as epoch milliseconds. Binary values become null; in a
//! controller dump they hold key material, never anything the summary uses.

use serde_json::{Map, Number, Value};

struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
    /// Nesting so far; a crafted file can't recurse the stack away.
    depth: u32,
}

/// Far deeper than any controller document.
const MAX_DEPTH: u32 = 64;

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        let end = self
            .pos
            .checked_add(n)
            .filter(|&e| e <= self.buf.len())
            .ok_or_else(|| format!("BSON truncated at byte {}", self.pos))?;
        let s = &self.buf[self.pos..end];
        self.pos = end;
        Ok(s)
    }

    fn arr<const N: usize>(&mut self) -> Result<[u8; N], String> {
        Ok(self.take(N)?.try_into().unwrap())
    }

    fn len(&mut self) -> Result<usize, String> {
        let n = i32::from_le_bytes(self.arr()?);
        usize::try_from(n).map_err(|_| format!("BSON negative length at byte {}", self.pos))
    }

    fn cstr(&mut self) -> Result<String, String> {
        let rest = &self.buf[self.pos..];
        let n = rest
            .iter()
            .position(|&b| b == 0)
            .ok_or("BSON key not terminated")?;
        let s = String::from_utf8_lossy(&rest[..n]).into_owned();
        self.pos += n + 1;
        Ok(s)
    }

    fn string(&mut self) -> Result<String, String> {
        let n = self.len()?;
        let b = self.take(n)?;
        Ok(String::from_utf8_lossy(b.strip_suffix(&[0]).unwrap_or(b)).into_owned())
    }

    /// Walk one embedded document, handing each element to `f`.
    fn elements(&mut self, f: &mut dyn FnMut(String, Value)) -> Result<(), String> {
        if self.depth >= MAX_DEPTH {
            return Err(format!("BSON nested too deep at byte {}", self.pos));
        }
        self.depth += 1;
        let start = self.pos;
        let end = start + self.len()?;
        if end > self.buf.len() || end < start + 5 {
            return Err(format!("BSON bad document length at byte {start}"));
        }
        while self.pos < end - 1 {
            let t = self.take(1)?[0];
            let k = self.cstr()?;
            let v = self.value(t)?;
            f(k, v);
        }
        self.pos = end;
        self.depth -= 1;
        Ok(())
    }

    fn document(&mut self) -> Result<Map<String, Value>, String> {
        let mut m = Map::new();
        self.elements(&mut |k, v| {
            m.insert(k, v);
        })?;
        Ok(m)
    }

    fn value(&mut self, t: u8) -> Result<Value, String> {
        Ok(match t {
            0x01 => {
                Number::from_f64(f64::from_le_bytes(self.arr()?)).map_or(Value::Null, Value::Number)
            }
            0x02 | 0x0D | 0x0E => Value::String(self.string()?),
            0x03 => Value::Object(self.document()?),
            0x04 => {
                // Keys are "0", "1", …; keep stream order.
                let mut a = Vec::new();
                self.elements(&mut |_, v| a.push(v))?;
                Value::Array(a)
            }
            0x05 => {
                let n = self.len()?;
                self.take(n + 1)?;
                Value::Null
            }
            0x06 | 0x0A | 0x7F | 0xFF => Value::Null,
            0x07 => Value::String(hex(self.take(12)?)),
            0x08 => Value::Bool(self.take(1)?[0] != 0),
            0x09 | 0x12 => i64::from_le_bytes(self.arr()?).into(),
            0x0B => {
                let pattern = self.cstr()?;
                let options = self.cstr()?;
                serde_json::json!({ "$regex": pattern, "$options": options })
            }
            0x0C => {
                self.string()?;
                self.take(12)?;
                Value::Null
            }
            0x0F => {
                let n = self.len()?;
                self.take(n.saturating_sub(4))?;
                Value::Null
            }
            0x10 => i32::from_le_bytes(self.arr()?).into(),
            0x11 => u64::from_le_bytes(self.arr()?).into(),
            0x13 => Value::String(hex(self.take(16)?)),
            _ => {
                return Err(format!(
                    "BSON type 0x{t:02x} unsupported at byte {}",
                    self.pos
                ));
            }
        })
    }
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// Split a concatenated BSON stream into raw documents.
pub fn documents(buf: &[u8]) -> impl Iterator<Item = Result<&[u8], String>> {
    let mut pos = 0;
    std::iter::from_fn(move || {
        if pos >= buf.len() {
            return None;
        }
        let Some(head) = buf.get(pos..pos + 4) else {
            pos = buf.len();
            return Some(Err(format!("BSON stream truncated at byte {pos}")));
        };
        // Negative or oversized lengths must not wrap the bounds check.
        let n = usize::try_from(i32::from_le_bytes(head.try_into().unwrap())).unwrap_or(0);
        if n < 5 || n > buf.len() - pos {
            let at = pos;
            pos = buf.len();
            return Some(Err(format!("BSON bad document length at byte {at}")));
        }
        let doc = &buf[pos..pos + n];
        pos += n;
        Some(Ok(doc))
    })
}

pub fn decode(doc: &[u8]) -> Result<Map<String, Value>, String> {
    Reader {
        buf: doc,
        pos: 0,
        depth: 0,
    }
    .document()
}

/// Cheap test for a `{"__cmd": …}` collection marker, so skipped
/// collections never need a full decode.
pub fn has_cmd(doc: &[u8]) -> bool {
    doc.windows(6).any(|w| w == b"__cmd\0")
}

#[cfg(test)]
pub mod tests {
    use serde_json::Value;

    /// Tiny BSON writer for test fixtures.
    pub fn encode(v: &Value) -> Vec<u8> {
        fn doc(items: Vec<(String, &Value)>) -> Vec<u8> {
            let mut body = Vec::new();
            for (k, v) in items {
                let (t, data) = match v {
                    Value::Null => (0x0A, vec![]),
                    Value::Bool(b) => (0x08, vec![*b as u8]),
                    Value::Number(n) if n.is_i64() => {
                        let i = n.as_i64().unwrap();
                        match i32::try_from(i) {
                            Ok(i) => (0x10, i.to_le_bytes().to_vec()),
                            Err(_) => (0x12, i.to_le_bytes().to_vec()),
                        }
                    }
                    Value::Number(n) => (0x01, n.as_f64().unwrap().to_le_bytes().to_vec()),
                    Value::String(s) => {
                        let mut d = ((s.len() + 1) as i32).to_le_bytes().to_vec();
                        d.extend(s.as_bytes());
                        d.push(0);
                        (0x02, d)
                    }
                    Value::Array(a) => (
                        0x04,
                        doc(a
                            .iter()
                            .enumerate()
                            .map(|(i, v)| (i.to_string(), v))
                            .collect()),
                    ),
                    Value::Object(_) => (0x03, encode(v)),
                };
                body.push(t);
                body.extend(k.as_bytes());
                body.push(0);
                body.extend(data);
            }
            let mut out = ((body.len() + 5) as i32).to_le_bytes().to_vec();
            out.extend(body);
            out.push(0);
            out
        }
        doc(v
            .as_object()
            .unwrap()
            .iter()
            .map(|(k, v)| (k.clone(), v))
            .collect())
    }

    #[test]
    fn round_trip() {
        let v = serde_json::json!({
            "s": "x", "i": 5, "big": 1_i64 << 40, "f": 1.5, "b": true, "n": null,
            "a": [3, 2, 1, 0, 9, 8, 7, 6, 5, 4, 10], "o": {"k": "v"}
        });
        let enc = encode(&v);
        assert_eq!(Value::Object(super::decode(&enc).unwrap()), v);
    }

    #[test]
    fn object_id_and_datetime() {
        // {"_id": ObjectId(00..0b), "t": Date(1000)}
        let mut body = vec![0x07];
        body.extend(b"_id\0");
        body.extend(0u8..12);
        body.push(0x09);
        body.extend(b"t\0");
        body.extend(1000i64.to_le_bytes());
        let mut d = ((body.len() + 5) as i32).to_le_bytes().to_vec();
        d.extend(body);
        d.push(0);
        let m = super::decode(&d).unwrap();
        assert_eq!(m["_id"], "000102030405060708090a0b");
        assert_eq!(m["t"], 1000);
    }

    #[test]
    fn stream_and_truncation() {
        let a = encode(&serde_json::json!({"__cmd": "select", "collection": "device"}));
        let b = encode(&serde_json::json!({"name": "sw"}));
        let s = [a.clone(), b.clone()].concat();
        let docs: Vec<_> = super::documents(&s).collect::<Result<_, _>>().unwrap();
        assert_eq!(docs, [&a[..], &b[..]]);
        assert!(super::has_cmd(docs[0]) && !super::has_cmd(docs[1]));
        assert!(super::documents(&s[..s.len() - 1]).any(|d| d.is_err()));
        assert!(super::decode(&b[..b.len() - 2]).is_err());
        let negative = [(-8i32).to_le_bytes().to_vec(), vec![0; 8]].concat();
        assert!(super::documents(&negative).next().unwrap().is_err());
    }
}
