//! Inspect a bare C2PA Manifest Store (one complete root JUMBF box).
//! cargo run -p zenmeta --example audit_c2pa --features c2pa,std -- store.jumbf
//!
//! Extract image-container carriers with the owning codec before using this tool.
use std::{error::Error, io::Read};
use zenmeta::audit::Report;

fn main() -> Result<(), Box<dyn Error>> {
    let mut paths = std::env::args().skip(1).peekable();
    if paths.peek().is_none() {
        return Err("usage: audit_c2pa store.jumbf... (raw stores, not image files)".into());
    }
    for path in paths {
        const MAX_STORE: u64 = 16 * 1024 * 1024;
        let mut store = Vec::new();
        std::fs::File::open(&path)?
            .take(MAX_STORE + 1)
            .read_to_end(&mut store)?;
        if store.len() as u64 > MAX_STORE {
            return Err(format!("{path}: manifest store exceeds 16 MiB").into());
        }
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
