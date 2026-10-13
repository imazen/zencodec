//! Strict framing preflight for the existing salvaging EXIF reader. We use the
//! shared reader/writer for interpretation and pruning; this prevents damaged
//! orientation or pointer entries from being silently salvaged away first.
use super::*;
use alloc::collections::BTreeSet;

pub(super) fn validate(req: &Context<'_>, bytes: &[u8]) -> Result<(), Error> {
    let mut expanded = if bytes.starts_with(b"Exif\0\0") {
        14usize
    } else {
        8
    };
    let bytes = bytes.strip_prefix(b"Exif\0\0").unwrap_or(bytes);
    let le = match bytes.get(..2) {
        Some(b"II") => true,
        Some(b"MM") => false,
        _ => return Err(Error::Malformed("EXIF byte order")),
    };
    let u16at = |p: usize| -> Result<u16, Error> {
        let b: [u8; 2] = bytes
            .get(p..p.checked_add(2).ok_or(Error::Malformed("EXIF offset"))?)
            .ok_or(Error::Malformed("EXIF bounds"))?
            .try_into()
            .unwrap();
        Ok(if le {
            u16::from_le_bytes(b)
        } else {
            u16::from_be_bytes(b)
        })
    };
    let u32at = |p: usize| -> Result<u32, Error> {
        let b: [u8; 4] = bytes
            .get(p..p.checked_add(4).ok_or(Error::Malformed("EXIF offset"))?)
            .ok_or(Error::Malformed("EXIF bounds"))?
            .try_into()
            .unwrap();
        Ok(if le {
            u32::from_le_bytes(b)
        } else {
            u32::from_be_bytes(b)
        })
    };
    if u16at(2)? != 42 {
        return Err(Error::Malformed("EXIF TIFF magic"));
    }
    let first = u32at(4)? as usize;
    if first == 0 {
        return Err(Error::Malformed("EXIF missing IFD0"));
    }
    let mut pending = alloc::vec![first];
    let mut visited = BTreeSet::new();
    let mut entries = 0usize;
    while let Some(p) = pending.pop() {
        req.check()?;
        if p < 8 || !visited.insert(p) || visited.len() > 64 {
            return Err(Error::Malformed("EXIF cyclic/excessive IFDs"));
        }
        let count = u16at(p)? as usize;
        expanded = expanded
            .checked_add(6 + 12 * count)
            .ok_or(Error::Limit("metadata bytes"))?;
        req.packet(expanded)?;
        let mut external_values = BTreeSet::new();
        let (mut thumbnail_offset, mut thumbnail_size) = (None, None);
        entries = entries
            .checked_add(count)
            .ok_or(Error::Limit("EXIF entries"))?;
        if entries > req.max_carriers() {
            return Err(Error::Limit("EXIF entries"));
        }
        let end = p
            .checked_add(2 + 12 * count)
            .filter(|&n| n <= bytes.len())
            .ok_or(Error::Malformed("EXIF truncated IFD"))?;
        let next = u32at(end)? as usize;
        if next != 0 {
            pending.push(next);
        }
        let mut tags = BTreeSet::new();
        for i in 0..count {
            req.check()?;
            let e = p + 2 + 12 * i;
            let tag = u16at(e)?;
            let ty = u16at(e + 2)?;
            let n = u32at(e + 4)? as usize;
            let width = match ty {
                1 | 2 | 6 | 7 => 1,
                3 | 8 => 2,
                4 | 9 | 11 | 13 => 4,
                5 | 10 | 12 | 16 | 17 | 18 => 8,
                _ => return Err(Error::Malformed("EXIF field type")),
            };
            let size = n
                .checked_mul(width)
                .ok_or(Error::Malformed("EXIF value length"))?;
            let value = if size <= 4 {
                e + 8
            } else {
                u32at(e + 8)? as usize
            };
            if value.checked_add(size).is_none_or(|v| v > bytes.len()) {
                return Err(Error::Malformed("EXIF value bounds"));
            }
            if size > 4 && external_values.insert((value, size)) {
                expanded = expanded
                    .checked_add(size)
                    .and_then(|n| n.checked_add(size & 1))
                    .ok_or(Error::Limit("metadata bytes"))?;
                req.packet(expanded)?;
            }
            if n == 1 && matches!(ty, 3 | 4) && matches!(tag, 0x0201 | 0x0202) {
                let scalar = if ty == 3 {
                    usize::from(u16at(value)?)
                } else {
                    u32at(value)? as usize
                };
                if tag == 0x0201 {
                    thumbnail_offset = Some(scalar);
                } else {
                    thumbnail_size = Some(scalar);
                }
            }
            if matches!(tag, 0x0112 | 0x8769 | 0x8825 | 0xa005) && !tags.insert(tag) {
                return Err(Error::Malformed("duplicate EXIF orientation/pointer"));
            }
            if tag == 0x0112 {
                if n != 1 || !matches!(ty, 3 | 4) {
                    return Err(Error::Malformed("EXIF orientation type"));
                }
                let v = if ty == 3 {
                    u16at(value)? as u32
                } else {
                    u32at(value)?
                };
                if !(1..=8).contains(&v) {
                    return Err(Error::Malformed("EXIF orientation value"));
                }
            }
            if matches!(tag, 0x8769 | 0x8825 | 0xa005) {
                if n != 1 || !matches!(ty, 4 | 13) {
                    return Err(Error::Malformed("EXIF IFD pointer type"));
                }
                let child = u32at(value)? as usize;
                if child != 0 {
                    pending.push(child);
                }
            }
        }
        if let (Some(offset), Some(size)) = (thumbnail_offset, thumbnail_size) {
            if offset.checked_add(size).is_none_or(|end| end > bytes.len()) {
                return Err(Error::Malformed("EXIF thumbnail bounds"));
            }
            expanded = expanded
                .checked_add(size)
                .ok_or(Error::Limit("metadata bytes"))?;
            req.packet(expanded)?;
        }
    }
    Ok(())
}
