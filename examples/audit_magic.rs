//! Audit: validate magic byte detection against real-world image corpus.
//! Run from zencodec repo root:
//!   cargo run --release --example audit_magic -- /mnt/v/input

use rayon::prelude::*;
use std::collections::HashMap;
use std::io::Read;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::sync::Mutex;
use std::time::Instant;

fn ext_to_expected(ext: &str) -> Option<zencodec::ImageFormat> {
    match ext {
        "jpg" | "jpeg" | "jpe" => Some(zencodec::ImageFormat::Jpeg),
        "png" => Some(zencodec::ImageFormat::Png),
        "gif" => Some(zencodec::ImageFormat::Gif),
        "webp" => Some(zencodec::ImageFormat::WebP),
        "avif" => Some(zencodec::ImageFormat::Avif),
        "jxl" => Some(zencodec::ImageFormat::Jxl),
        "heic" | "heif" => Some(zencodec::ImageFormat::Heic),
        "bmp" => Some(zencodec::ImageFormat::Bmp),
        "tif" | "tiff" => Some(zencodec::ImageFormat::Tiff),
        "ico" => Some(zencodec::ImageFormat::Ico),
        "pnm" | "pbm" | "pgm" | "ppm" | "pam" | "pfm" => Some(zencodec::ImageFormat::Pnm),
        "ff" | "farbfeld" => Some(zencodec::ImageFormat::Farbfeld),
        "qoi" => Some(zencodec::ImageFormat::Qoi),
        _ => None,
    }
}

fn main() {
    let dir = std::env::args().nth(1).unwrap_or_else(|| "/mnt/v/input".into());

    // Phase 1: collect paths
    eprintln!("Scanning {dir} ...");
    let paths: Vec<(PathBuf, String, zencodec::ImageFormat)> = walkdir::WalkDir::new(&dir)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .filter_map(|e| {
            let ext = e.path().extension()?.to_str()?.to_lowercase();
            let expected = ext_to_expected(&ext)?;
            Some((e.into_path(), ext, expected))
        })
        .collect();
    let total_files = paths.len() as u64;
    eprintln!("{total_files} image files found, checking magic bytes ...");

    // Shared state
    let registry = zencodec::ImageFormatRegistry::common();
    let stats: Mutex<HashMap<String, [u64; 3]>> = Mutex::new(HashMap::new());
    let mismatches: Mutex<Vec<String>> = Mutex::new(Vec::new());
    let undetected: Mutex<Vec<String>> = Mutex::new(Vec::new());
    let ok_count = AtomicU64::new(0);
    let fail_count = AtomicU64::new(0);
    let done_count = AtomicU64::new(0);
    let start = Instant::now();
    let last_report = AtomicU64::new(0); // epoch seconds of last progress report

    // Phase 2: parallel check
    paths.par_iter().for_each(|(path, ext, expected)| {
        let mut buf = [0u8; 64];
        let n = match std::fs::File::open(path).and_then(|mut f| f.read(&mut buf)) {
            Ok(n) => n,
            Err(_) => {
                done_count.fetch_add(1, Relaxed);
                return;
            }
        };

        let detected = registry.detect(&buf[..n]);
        {
            let mut s = stats.lock().unwrap();
            let e = s.entry(ext.clone()).or_insert([0, 0, 0]);
            e[0] += 1;
            match detected {
                Some(fmt) if fmt == *expected => {
                    e[1] += 1;
                    ok_count.fetch_add(1, Relaxed);
                }
                Some(fmt) => {
                    e[2] += 1;
                    fail_count.fetch_add(1, Relaxed);
                    drop(s);
                    let mut m = mismatches.lock().unwrap();
                    if m.len() < 200 {
                        m.push(format!(
                            "  MISMATCH: {} — ext={ext}, detected={fmt:?}, expected={expected:?}",
                            path.display()
                        ));
                    }
                }
                None => {
                    e[2] += 1;
                    fail_count.fetch_add(1, Relaxed);
                    drop(s);
                    let mut u = undetected.lock().unwrap();
                    if u.len() < 200 {
                        u.push(format!("  UNDETECTED: {} (ext={ext})", path.display()));
                    }
                }
            }
        }
        let done = done_count.fetch_add(1, Relaxed) + 1;

        // Progress every 10 seconds
        let secs = start.elapsed().as_secs();
        let prev = last_report.load(Relaxed);
        if secs >= prev + 10 && last_report.compare_exchange(prev, secs, Relaxed, Relaxed).is_ok()
        {
            let ok = ok_count.load(Relaxed);
            let fail = fail_count.load(Relaxed);
            let rate = if secs > 0 { done / secs } else { 0 };
            eprintln!("[{secs:>4}s] {done:>8}/{total_files} ({rate}/s) — ok={ok}, fail={fail}");
        }
    });

    // Phase 3: report
    let ok = ok_count.load(Relaxed);
    let fail = fail_count.load(Relaxed);
    let elapsed = start.elapsed();
    let stats = stats.into_inner().unwrap();
    let mismatches = mismatches.into_inner().unwrap();
    let undetected = undetected.into_inner().unwrap();

    println!("=== Magic Byte Detection Audit ===");
    println!("Directory: {dir}");
    println!("Total files checked: {total_files}");
    println!("Elapsed: {:.1}s", elapsed.as_secs_f64());
    println!();

    let mut exts: Vec<_> = stats.iter().collect();
    exts.sort_by_key(|(_, v)| std::cmp::Reverse(v[0]));

    println!("{:<10} {:>8} {:>8} {:>8}", "ext", "total", "ok", "fail");
    println!("{:-<10} {:-<8} {:-<8} {:-<8}", "", "", "", "");
    for (ext, v) in &exts {
        println!("{:<10} {:>8} {:>8} {:>8}", ext, v[0], v[1], v[2]);
    }
    println!("{:-<10} {:-<8} {:-<8} {:-<8}", "", "", "", "");
    println!("{:<10} {:>8} {:>8} {:>8}", "TOTAL", total_files, ok, fail);

    if !mismatches.is_empty() {
        println!("\n=== Mismatches (detected as wrong format) ===");
        for m in &mismatches { println!("{m}"); }
        if mismatches.len() == 200 { println!("  ... (truncated at 200)"); }
    }

    if !undetected.is_empty() {
        println!("\n=== Undetected (extension suggests image, magic bytes didn't match) ===");
        for m in &undetected { println!("{m}"); }
        if undetected.len() == 200 { println!("  ... (truncated at 200)"); }
    }

    if fail == 0 {
        println!("\nAll {total_files} files detected correctly.");
    } else {
        println!("\n{fail} / {total_files} files had detection issues.");
        std::process::exit(1);
    }
}
