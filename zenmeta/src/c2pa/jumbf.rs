//! JUMBF (ISO/IEC 19566-5) boxes: superboxes (`jumb`) with their description box
//! (`jumd`) and content boxes.
use alloc::{string::String, vec::Vec};

/// Deepest superbox nesting followed. A C2PA store nests store → manifest →
/// assertion store → assertion, so four levels plus ingredients' own stores.
pub(crate) const MAX_DEPTH: usize = 16;

#[derive(Clone, Copy, Debug)]
pub(crate) struct Box<'a> {
    pub ty: [u8; 4],
    pub payload: &'a [u8],
}

/// The boxes laid end to end in `data`. `LBox` 1 takes a 64-bit `XLBox`; 0 runs to
/// the end. An error names the first box that does not fit.
pub(crate) fn boxes(data: &[u8]) -> Result<Vec<Box<'_>>, &'static str> {
    let mut out = Vec::new();
    let mut pos = 0usize;
    while pos < data.len() {
        let head = data.get(pos..pos + 8).ok_or("truncated box header")?;
        let lbox = u32::from_be_bytes([head[0], head[1], head[2], head[3]]) as u64;
        let ty = [head[4], head[5], head[6], head[7]];
        let (header, len) = match lbox {
            0 => (8usize, (data.len() - pos) as u64),
            1 => {
                let x = data.get(pos + 8..pos + 16).ok_or("truncated XLBox")?;
                (16, u64::from_be_bytes(x.try_into().unwrap()))
            }
            2..=7 => return Err("box length below its header"),
            n => (8, n),
        };
        if len < header as u64 {
            return Err("box length below its header");
        }
        let end = usize::try_from(len)
            .ok()
            .and_then(|l| pos.checked_add(l))
            .filter(|&e| e <= data.len())
            .ok_or("box runs past its parent")?;
        out.push(Box {
            ty,
            payload: &data[pos + header..end],
        });
        pos = end;
    }
    Ok(out)
}

/// A superbox's description box.
#[derive(Clone, Debug, Default)]
pub(crate) struct Description {
    pub uuid: [u8; 16],
    pub label: Option<String>,
    pub id: Option<u32>,
    pub hash: Option<[u8; 32]>,
    /// The private box's type and payload length (C2PA puts a salt here).
    pub private: Option<([u8; 4], usize)>,
}

pub(crate) fn description(payload: &[u8]) -> Result<Description, &'static str> {
    let uuid: [u8; 16] = payload
        .get(..16)
        .ok_or("short description box")?
        .try_into()
        .unwrap();
    let toggles = *payload.get(16).ok_or("short description box")?;
    let mut d = Description {
        uuid,
        ..Description::default()
    };
    let mut rest = &payload[17..];
    if toggles & 0x02 != 0 {
        let end = rest
            .iter()
            .position(|&b| b == 0)
            .ok_or("unterminated label")?;
        d.label = Some(String::from_utf8(rest[..end].to_vec()).map_err(|_| "label is not UTF-8")?);
        rest = &rest[end + 1..];
    }
    if toggles & 0x04 != 0 {
        let id = rest.get(..4).ok_or("short ID field")?;
        d.id = Some(u32::from_be_bytes(id.try_into().unwrap()));
        rest = &rest[4..];
    }
    if toggles & 0x08 != 0 {
        d.hash = Some(
            rest.get(..32)
                .ok_or("short hash field")?
                .try_into()
                .unwrap(),
        );
        rest = &rest[32..];
    }
    if toggles & 0x10 != 0 {
        let b = boxes(rest)?;
        let first = b.first().ok_or("missing private box")?;
        d.private = Some((first.ty, first.payload.len()));
    }
    Ok(d)
}

/// The four-character code a C2PA type UUID starts with, when the other twelve bytes
/// are the ISO base suffix `0011 0010 8000 00AA 0038 9B71`.
pub(crate) fn c2pa_type(uuid: &[u8; 16]) -> Option<[u8; 4]> {
    const SUFFIX: [u8; 12] = [
        0x00, 0x11, 0x00, 0x10, 0x80, 0x00, 0x00, 0xaa, 0x00, 0x38, 0x9b, 0x71,
    ];
    (uuid[4..] == SUFFIX).then(|| [uuid[0], uuid[1], uuid[2], uuid[3]])
}

/// ISO/IEC 19566-5 embedded-file content type (`bfdb` + `bidb` boxes).
pub(crate) const EMBEDDED_FILE: [u8; 16] = [
    0x40, 0xcb, 0x0c, 0x32, 0xbb, 0x8a, 0x48, 0x9d, 0xa7, 0x0b, 0x2a, 0xd6, 0xf4, 0x7f, 0x43, 0x69,
];

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    #[test]
    fn box_lengths_are_bounded_by_the_parent() {
        assert!(boxes(&[0, 0, 0, 9, b'j', b'u', b'm', b'b']).is_err());
        assert!(boxes(&[0, 0, 0, 4, b'j', b'u', b'm', b'b']).is_err());
        let mut xl = vec![0, 0, 0, 1, b'c', b'b', b'o', b'r'];
        xl.extend_from_slice(&u64::MAX.to_be_bytes());
        assert!(boxes(&xl).is_err());
        let ok = boxes(&[0, 0, 0, 9, b'c', b'b', b'o', b'r', 0xf6]).unwrap();
        assert_eq!((ok.len(), ok[0].payload), (1, &[0xf6u8][..]));
        let to_end = boxes(&[0, 0, 0, 0, b'b', b'i', b'd', b'b', 1, 2, 3]).unwrap();
        assert_eq!(to_end[0].payload, &[1, 2, 3]);
    }

    #[test]
    fn description_fields_follow_the_toggles() {
        let mut p = b"c2pa\x00\x11\x00\x10\x80\x00\x00\xaa\x00\x38\x9b\x71".to_vec();
        p.push(0x03 | 0x04);
        p.extend_from_slice(b"c2pa\0");
        p.extend_from_slice(&7u32.to_be_bytes());
        let d = description(&p).unwrap();
        assert_eq!(
            (d.label.as_deref(), d.id, c2pa_type(&d.uuid)),
            (Some("c2pa"), Some(7), Some(*b"c2pa"))
        );
        assert!(description(&p[..20]).is_err());
    }
}
