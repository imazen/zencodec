use std::io::Write;
use std::process::{Command, Stdio};
use zencodec::encode::EncoderConfig;
use zenjpeg::JpegEncoderConfig;
use zenjpeg::container::{MarkerKind, marker};
use zenpixels::{PixelDescriptor, PixelSlice};

pub const WIDTH: u32 = 97;
pub const HEIGHT: u32 = 65;
pub const STRIDE: usize = WIDTH as usize * 3 + 21;
pub const PRIVATE: &[u8] = b"PRIVATE-fixture@example.test";

pub fn raw() -> Vec<u8> {
    let mut bytes = vec![0xa5; STRIDE * HEIGHT as usize];
    let mut state = 0x12345678u32;
    for y in 0..HEIGHT as usize {
        for b in &mut bytes[y * STRIDE..y * STRIDE + WIDTH as usize * 3] {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            *b = state as u8;
        }
    }
    bytes
}
pub fn pixels(bytes: &[u8]) -> PixelSlice<'_> {
    PixelSlice::new(bytes, WIDTH, HEIGHT, STRIDE, PixelDescriptor::RGB8_SRGB).unwrap()
}
pub fn config(progressive: bool, optimized: bool) -> JpegEncoderConfig {
    JpegEncoderConfig::new()
        .with_calibrated_quality(87.0)
        .with_progressive(progressive)
        .with_huffman(optimized)
}
pub fn base(progressive: bool) -> Vec<u8> {
    config(progressive, true)
        .encode(pixels(&raw()))
        .unwrap()
        .data()
        .to_vec()
}
pub fn profile() -> Vec<u8> {
    use zenpixels_convert::icc_profiles::{SynthesizedIcc, synthesize_icc_for_cicp};
    match synthesize_icc_for_cicp(zencodec::Cicp::new(12, 13, 0, true)) {
        SynthesizedIcc::Profile(bytes) => bytes.into_owned(),
        other => panic!("expected an actual Display P3 profile: {other:?}"),
    }
}

// Fixture framing follows zenjpeg tests/metadata_rewrite.rs at a2a128ab. Parsing,
// encoding and rewriting are delegated to existing implementations below.
pub fn segment(marker: u8, payload: &[u8]) -> Vec<u8> {
    let mut bytes = vec![0xff, marker];
    bytes.extend_from_slice(&u16::try_from(payload.len() + 2).unwrap().to_be_bytes());
    bytes.extend_from_slice(payload);
    bytes
}
pub fn icc_part(profile: &[u8], part: u8, total: u8) -> Vec<u8> {
    let mut payload = b"ICC_PROFILE\0".to_vec();
    payload.extend_from_slice(&[part, total]);
    payload.extend_from_slice(profile);
    segment(0xe2, &payload)
}
pub fn descriptive() -> Vec<u8> {
    let mut exif = zencodec::exif::Exif::new(zencodec::exif::TextEncoding::Ascii);
    exif.set_orientation(zencodec::Orientation::Rotate90);
    exif.set_artist(core::str::from_utf8(PRIVATE).unwrap());
    let tiff = zenmeta::exif::serialize(&exif);
    let mut payload = b"Exif\0\0".to_vec();
    payload.extend_from_slice(&tiff);
    let mut bytes = segment(0xe1, &payload);
    let mut xmp = b"http://ns.adobe.com/xap/1.0/\0".to_vec();
    xmp.extend_from_slice(b"<x xmlns='urn:private'>PRIVATE-fixture@example.test</x>");
    bytes.extend(segment(0xe1, &xmp));
    bytes.extend(segment(0xfe, PRIVATE));
    bytes
}
#[derive(Clone, Copy, Debug)]
pub enum Placement {
    Header,
    AfterFirstScan,
    BeforeEoi,
    SplitIcc,
}
pub fn fixture(progressive: bool, placement: Placement) -> Vec<u8> {
    let jpeg = base(progressive);
    let spans: Vec<_> = marker::iter(&jpeg).collect();
    let first_scan = spans.iter().find(|s| s.kind == MarkerKind::Sos).unwrap();
    let eoi = spans
        .iter()
        .find(|s| s.kind == MarkerKind::Eoi)
        .unwrap()
        .offset;
    let profile = profile();
    let mid = profile.len() / 2;
    let first = icc_part(&profile[..mid], 1, 2);
    let second = icc_part(&profile[mid..], 2, 2);
    let insertion = match placement {
        Placement::Header => 2,
        Placement::AfterFirstScan | Placement::SplitIcc => first_scan.offset + first_scan.length,
        Placement::BeforeEoi => eoi,
    };
    let mut out = Vec::new();
    if matches!(placement, Placement::SplitIcc) {
        out.extend_from_slice(&jpeg[..2]);
        out.extend(first);
        out.extend_from_slice(&jpeg[2..insertion]);
    } else {
        out.extend_from_slice(&jpeg[..insertion]);
        out.extend(first);
    }
    out.extend(second);
    out.extend(descriptive());
    out.extend_from_slice(&jpeg[insertion..]);
    out
}
pub fn scans(bytes: &[u8]) -> Vec<Vec<u8>> {
    marker::iter(bytes)
        .filter(|s| s.kind == MarkerKind::Sos)
        .map(|s| bytes[s.offset..s.offset + s.length].to_vec())
        .collect()
}
pub fn oracle(bytes: &[u8]) -> Vec<u8> {
    let mut child = Command::new("djpeg")
        .arg("-ppm")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("djpeg is required by the caller; install libjpeg-turbo-progs");
    let mut stdin = child.stdin.take().unwrap();
    let input = bytes.to_vec();
    let writer = std::thread::spawn(move || stdin.write_all(&input));
    let result = child.wait_with_output().unwrap();
    writer.join().unwrap().unwrap();
    assert!(
        result.status.success(),
        "djpeg: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(result.stdout.starts_with(b"P6\n"));
    result.stdout
}

pub struct OracleIcc {
    pub bytes: Vec<u8>,
    pub status: i32,
    pub diagnostic: String,
}
pub fn oracle_icc(bytes: &[u8]) -> OracleIcc {
    // Explicit caller-provided disk location; never use std::env::temp_dir().
    let root = std::env::var_os("JPEG_POLICY_ARTIFACTS")
        .expect("set JPEG_POLICY_ARTIFACTS to a disk-backed artifact directory");
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::path::PathBuf::from(root).join(format!("{}-{nonce}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let profile = dir.join("profile.icc");
    let mut child = Command::new("djpeg")
        .arg("-icc")
        .arg(&profile)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("djpeg is required by the caller; install libjpeg-turbo-progs");
    child.stdin.take().unwrap().write_all(bytes).unwrap();
    let output = child.wait_with_output().unwrap();
    std::fs::write(dir.join("stderr.txt"), &output.stderr).unwrap();
    // djpeg creates an empty file when no profile was found. An error status
    // still matters: the caller distinguishes no profile from bad chunks.
    OracleIcc {
        bytes: std::fs::read(profile).expect("djpeg must create its requested ICC output"),
        status: output
            .status
            .code()
            .expect("oracle must not terminate by signal"),
        diagnostic: String::from_utf8(output.stderr).unwrap(),
    }
}
