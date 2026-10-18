//! Bounded CBOR (RFC 8949) reader for C2PA claims, assertions and COSE structures.
//!
//! Depth and item budgets are checked before descending, so hostile nesting fails
//! with an error instead of exhausting the stack. Indefinite-length strings are
//! concatenated; text must be valid UTF-8.
use super::budget::Budget;
use alloc::{string::String, vec::Vec};

/// Deepest nesting accepted (arrays, maps and tags count one level each).
pub(crate) const MAX_DEPTH: usize = 32;

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Value {
    Uint(u64),
    /// `-1 - n`.
    Nint(u64),
    Bytes(Vec<u8>),
    Text(String),
    Array(Vec<Value>),
    Map(Vec<(Value, Value)>),
    // Exactly one child; Vec permits fallible allocation on stable Rust.
    Tag(u64, Vec<Value>),
    Bool(bool),
    Null,
    Undefined,
    Simple(u8),
    Float(f64),
}

impl Value {
    pub(crate) fn as_text(&self) -> Option<&str> {
        match self {
            Value::Text(t) => Some(t),
            _ => None,
        }
    }
    pub(crate) fn as_bytes(&self) -> Option<&[u8]> {
        match self {
            Value::Bytes(b) => Some(b),
            _ => None,
        }
    }
    /// A map's value under a text key.
    pub(crate) fn get_text(&self, key: &str) -> Option<&Value> {
        let Value::Map(pairs) = self else { return None };
        pairs
            .iter()
            .find(|(k, _)| k.as_text() == Some(key))
            .map(|(_, v)| v)
    }
    pub(crate) fn as_i64(&self) -> Option<i64> {
        match *self {
            Value::Uint(n) => i64::try_from(n).ok(),
            Value::Nint(n) => i64::try_from(n).ok().map(|n| -1 - n),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Error {
    Truncated,
    Malformed(&'static str),
    Depth,
    Budget,
    /// Bytes left after the top-level item.
    Trailing(usize),
}

struct Reader<'a, 'b> {
    data: &'a [u8],
    pos: usize,
    budget: &'b mut Budget,
}

/// Parse exactly one CBOR data item spanning all of `data`.
#[cfg(test)]
pub(crate) fn parse(data: &[u8]) -> Result<Value, Error> {
    parse_with_budget(data, &mut Budget::new())
}

pub(super) fn parse_with_budget(data: &[u8], budget: &mut Budget) -> Result<Value, Error> {
    let mut r = Reader {
        data,
        pos: 0,
        budget,
    };
    let v = r.item(0)?;
    if r.pos != data.len() {
        return Err(Error::Trailing(data.len() - r.pos));
    }
    Ok(v)
}

enum Head {
    Value(u64),
    Indefinite,
}

impl<'a> Reader<'a, '_> {
    fn byte(&mut self) -> Result<u8, Error> {
        let b = *self.data.get(self.pos).ok_or(Error::Truncated)?;
        self.pos += 1;
        Ok(b)
    }
    fn take(&mut self, n: u64) -> Result<&'a [u8], Error> {
        let n = usize::try_from(n).map_err(|_| Error::Truncated)?;
        let end = self.pos.checked_add(n).ok_or(Error::Truncated)?;
        let s = self.data.get(self.pos..end).ok_or(Error::Truncated)?;
        self.pos = end;
        Ok(s)
    }
    fn uint(&mut self, info: u8) -> Result<Head, Error> {
        let n = match info {
            0..=23 => u64::from(info),
            24 => u64::from(self.byte()?),
            25 => u64::from(u16::from_be_bytes(self.take(2)?.try_into().unwrap())),
            26 => u64::from(u32::from_be_bytes(self.take(4)?.try_into().unwrap())),
            27 => u64::from_be_bytes(self.take(8)?.try_into().unwrap()),
            31 => return Ok(Head::Indefinite),
            _ => return Err(Error::Malformed("reserved additional information")),
        };
        Ok(Head::Value(n))
    }
    fn count(&mut self) -> Result<(), Error> {
        self.budget.item().map_err(|_| Error::Budget)
    }
    /// Length-prefixed counts can claim far more items than the input holds; every
    /// item takes at least one byte, so a count above the remaining bytes is false.
    fn check_len(&self, n: u64) -> Result<usize, Error> {
        let rest = (self.data.len() - self.pos) as u64;
        if n > rest {
            return Err(Error::Truncated);
        }
        Ok(n as usize)
    }
    fn string(&mut self, major: u8, info: u8) -> Result<Vec<u8>, Error> {
        match self.uint(info)? {
            Head::Value(n) => {
                let bytes = self.take(n)?;
                self.budget.copy(bytes).map_err(|_| Error::Budget)
            }
            Head::Indefinite => {
                let mut out = Vec::new();
                loop {
                    let b = self.byte()?;
                    if b == 0xff {
                        return Ok(out);
                    }
                    if b >> 5 != major || b & 0x1f == 31 {
                        return Err(Error::Malformed("bad indefinite-length string chunk"));
                    }
                    self.count()?;
                    match self.uint(b & 0x1f)? {
                        Head::Value(n) => {
                            let bytes = self.take(n)?;
                            if major == 3 {
                                core::str::from_utf8(bytes)
                                    .map_err(|_| Error::Malformed("text is not UTF-8"))?;
                            }
                            self.budget
                                .reserve(&mut out, bytes.len())
                                .map_err(|_| Error::Budget)?;
                            out.extend_from_slice(bytes);
                        }
                        Head::Indefinite => unreachable!(),
                    }
                }
            }
        }
    }
    fn item(&mut self, depth: usize) -> Result<Value, Error> {
        if depth > MAX_DEPTH {
            return Err(Error::Depth);
        }
        self.count()?;
        let b = self.byte()?;
        let (major, info) = (b >> 5, b & 0x1f);
        Ok(match major {
            0 | 1 => match self.uint(info)? {
                Head::Value(n) if major == 0 => Value::Uint(n),
                Head::Value(n) => Value::Nint(n),
                Head::Indefinite => return Err(Error::Malformed("indefinite integer")),
            },
            2 => Value::Bytes(self.string(2, info)?),
            3 => Value::Text(
                String::from_utf8(self.string(3, info)?)
                    .map_err(|_| Error::Malformed("text is not UTF-8"))?,
            ),
            4 => {
                let mut items = Vec::new();
                match self.uint(info)? {
                    Head::Value(n) => {
                        for _ in 0..self.check_len(n)? {
                            let item = self.item(depth + 1)?;
                            self.budget
                                .push(&mut items, item)
                                .map_err(|_| Error::Budget)?;
                        }
                    }
                    Head::Indefinite => loop {
                        if self.data.get(self.pos) == Some(&0xff) {
                            self.pos += 1;
                            break;
                        }
                        let item = self.item(depth + 1)?;
                        self.budget
                            .push(&mut items, item)
                            .map_err(|_| Error::Budget)?;
                    },
                }
                Value::Array(items)
            }
            5 => {
                let mut pairs = Vec::new();
                match self.uint(info)? {
                    Head::Value(n) => {
                        let n = self.check_len(n)?;
                        for _ in 0..n {
                            let k = self.item(depth + 1)?;
                            let v = self.item(depth + 1)?;
                            self.budget
                                .push(&mut pairs, (k, v))
                                .map_err(|_| Error::Budget)?;
                        }
                    }
                    Head::Indefinite => loop {
                        if self.data.get(self.pos) == Some(&0xff) {
                            self.pos += 1;
                            break;
                        }
                        let k = self.item(depth + 1)?;
                        let v = self.item(depth + 1)?;
                        self.budget
                            .push(&mut pairs, (k, v))
                            .map_err(|_| Error::Budget)?;
                    },
                }
                Value::Map(pairs)
            }
            6 => match self.uint(info)? {
                Head::Value(tag) => {
                    let item = self.item(depth + 1)?;
                    let mut child = Vec::new();
                    self.budget
                        .push(&mut child, item)
                        .map_err(|_| Error::Budget)?;
                    Value::Tag(tag, child)
                }
                Head::Indefinite => return Err(Error::Malformed("indefinite tag")),
            },
            _ => match info {
                20 => Value::Bool(false),
                21 => Value::Bool(true),
                22 => Value::Null,
                23 => Value::Undefined,
                24 => match self.byte()? {
                    n if n < 32 => return Err(Error::Malformed("two-byte simple value below 32")),
                    n => Value::Simple(n),
                },
                25 => Value::Float(half(u16::from_be_bytes(self.take(2)?.try_into().unwrap()))),
                26 => Value::Float(f64::from(f32::from_bits(u32::from_be_bytes(
                    self.take(4)?.try_into().unwrap(),
                )))),
                27 => Value::Float(f64::from_bits(u64::from_be_bytes(
                    self.take(8)?.try_into().unwrap(),
                ))),
                0..=19 => Value::Simple(info),
                31 => return Err(Error::Malformed("unexpected break")),
                _ => return Err(Error::Malformed("reserved simple value")),
            },
        })
    }
}

/// IEEE 754 binary16 to f64.
fn half(h: u16) -> f64 {
    let sign = if h & 0x8000 != 0 { -1.0 } else { 1.0 };
    let exp = i32::from((h >> 10) & 0x1f);
    let frac = f64::from(h & 0x3ff);
    sign * match exp {
        0 => frac * 2f64.powi(-24),
        31 if frac == 0.0 => f64::INFINITY,
        31 => f64::NAN,
        e => (1.0 + frac / 1024.0) * 2f64.powi(e - 15),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    #[test]
    fn rfc8949_appendix_a_examples() {
        let cases: &[(&[u8], Value)] = &[
            (&[0x00], Value::Uint(0)),
            (&[0x18, 0x64], Value::Uint(100)),
            (
                &[0x1b, 0, 0, 0, 0xe8, 0xd4, 0xa5, 0x10, 0],
                Value::Uint(1_000_000_000_000),
            ),
            (&[0x38, 0x63], Value::Nint(99)),
            (&[0xf9, 0x3c, 0x00], Value::Float(1.0)),
            (&[0xf9, 0x00, 0x01], Value::Float(5.960464477539063e-8)),
            (&[0xfa, 0x47, 0xc3, 0x50, 0x00], Value::Float(100000.0)),
            (&[0xf4], Value::Bool(false)),
            (&[0xf6], Value::Null),
            (&[0x44, 1, 2, 3, 4], Value::Bytes(vec![1, 2, 3, 4])),
            (&[0x64, 0x49, 0x45, 0x54, 0x46], Value::Text("IETF".into())),
            (
                &[0x5f, 0x42, 1, 2, 0x43, 3, 4, 5, 0xff],
                Value::Bytes(vec![1, 2, 3, 4, 5]),
            ),
            (
                &[0x83, 0x01, 0x82, 0x02, 0x03, 0x82, 0x04, 0x05],
                Value::Array(vec![
                    Value::Uint(1),
                    Value::Array(vec![Value::Uint(2), Value::Uint(3)]),
                    Value::Array(vec![Value::Uint(4), Value::Uint(5)]),
                ]),
            ),
            (
                &[0xbf, 0x61, 0x61, 0x01, 0xff],
                Value::Map(vec![(Value::Text("a".into()), Value::Uint(1))]),
            ),
            (
                &[0xc1, 0x1a, 0x51, 0x4b, 0x67, 0xb0],
                Value::Tag(1, vec![Value::Uint(1_363_896_240)]),
            ),
        ];
        for (bytes, want) in cases {
            assert_eq!(&parse(bytes).unwrap(), want, "{bytes:02x?}");
        }
        assert!(matches!(parse(&[0xf9, 0x7c, 0x00]).unwrap(), Value::Float(f) if f.is_infinite()));
    }

    #[test]
    fn hostile_inputs_fail_without_recursing_or_allocating() {
        // 100k nested arrays: rejected at MAX_DEPTH, long before the stack.
        let deep = vec![0x81u8; 100_000];
        assert_eq!(parse(&deep), Err(Error::Depth));
        // An array claiming 2^64-1 items in five bytes.
        assert_eq!(
            parse(&[0x9b, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff]),
            Err(Error::Truncated)
        );
        // A byte string claiming 4 GiB.
        assert_eq!(
            parse(&[0x5a, 0xff, 0xff, 0xff, 0xff]),
            Err(Error::Truncated)
        );
        assert_eq!(
            parse(&[0x62, 0xff, 0xfe]),
            Err(Error::Malformed("text is not UTF-8"))
        );
        assert_eq!(parse(&[0x01, 0x02]), Err(Error::Trailing(1)));
        assert_eq!(
            parse(&[0x1c]),
            Err(Error::Malformed("reserved additional information"))
        );
        assert_eq!(
            parse(&[0x5f, 0x61, 0x41, 0xff]),
            Err(Error::Malformed("bad indefinite-length string chunk"))
        );
    }
}
