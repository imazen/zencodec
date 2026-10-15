//! Inspect the C2PA manifest store in a PNG (`caBX` chunk), a JPEG (APP11 packets) or a
//! bare JUMBF file, and print every finding and entry.
//! cargo run --example audit_c2pa --features c2pa,std -- file.png [file.jpg ...]
//!
//! The container framing here is example code: in a codec it belongs to the decoder.
use std::error::Error;
use zenmeta::audit::Report;

/// The PNG `caBX` chunk's payload.
fn from_png(d: &[u8]) -> Option<Vec<u8>> {
    let mut pos = 8;
    while pos + 12 <= d.len() {
        let len = u32::from_be_bytes(d[pos..pos + 4].try_into().ok()?) as usize;
        let ty = &d[pos + 4..pos + 8];
        let body = d.get(pos + 8..pos + 8 + len)?;
        if ty == b"caBX" {
            return Some(body.to_vec());
        }
        if ty == b"IEND" {
            return None;
        }
        pos += 12 + len;
    }
    None
}

/// APP11 packets: `JP`, box instance (2 bytes), sequence number (4 bytes), then the
/// box. Continuation packets repeat the box header (LBox+TBox, plus XLBox when LBox
/// is 1), which is dropped when the payloads are joined.
fn from_jpeg(d: &[u8]) -> Option<Vec<u8>> {
    let mut packets: Vec<(u16, u32, &[u8])> = Vec::new();
    let mut pos = 2;
    while pos + 4 <= d.len() && d[pos] == 0xff {
        let marker = d[pos + 1];
        if marker == 0xda || marker == 0xd9 {
            break;
        }
        let len = u16::from_be_bytes([d[pos + 2], d[pos + 3]]) as usize;
        let seg = d.get(pos + 4..pos + 2 + len)?;
        if marker == 0xeb && seg.len() > 8 && &seg[..2] == b"JP" {
            let en = u16::from_be_bytes([seg[2], seg[3]]);
            let z = u32::from_be_bytes(seg[4..8].try_into().ok()?);
            packets.push((en, z, &seg[8..]));
        }
        pos += 2 + len;
    }
    let en = packets.first()?.0;
    let mut mine: Vec<_> = packets.into_iter().filter(|p| p.0 == en).collect();
    mine.sort_by_key(|p| p.1);
    let mut out = mine[0].2.to_vec();
    for (_, _, body) in &mine[1..] {
        let header = if body.get(..4) == Some(&[0u8, 0, 0, 1][..]) {
            16
        } else {
            8
        };
        out.extend_from_slice(body.get(header..)?);
    }
    Some(out)
}

fn main() -> Result<(), Box<dyn Error>> {
    let paths: Vec<String> = std::env::args().skip(1).collect();
    if paths.is_empty() {
        return Err("usage: audit_c2pa file...".into());
    }
    for path in paths {
        let d = std::fs::read(&path)?;
        let store = if d.starts_with(b"\x89PNG") {
            from_png(&d)
        } else if d.starts_with(&[0xff, 0xd8]) {
            from_jpeg(&d)
        } else {
            Some(d)
        };
        let Some(store) = store else {
            println!("{path}: no C2PA manifest store");
            continue;
        };
        let report = Report::c2pa(&store);
        println!(
            "{path}: {} entries, {} findings",
            report.entries().len(),
            report.findings().len()
        );
        for f in report.findings() {
            println!("finding: {f:?}");
        }
        for e in report.entries() {
            println!("{} [{}] {}", e.path(), e.kind(), e.value());
        }
    }
    Ok(())
}
