//! Built-in format definition statics and shared detection helpers.

use super::{ImageFormat, ImageFormatDefinition};

// ------- ISOBMFF helpers (shared by AVIF + HEIC) -------

pub(super) const HEIC_BRANDS: &[&[u8; 4]] = &[
    b"heic", b"heix", b"hevc", b"hevx", b"heim", b"heis", b"hevm", b"hevs",
];

fn has_ftyp(data: &[u8]) -> bool {
    data.len() >= 12 && &data[4..8] == b"ftyp"
}

fn scan_compat_brands(data: &[u8], target: &[&[u8; 4]]) -> bool {
    let box_size = u32::from_be_bytes([data[0], data[1], data[2], data[3]]) as usize;
    let end = box_size.min(data.len());
    let mut offset = 16;
    while offset + 4 <= end {
        let compat = &data[offset..offset + 4];
        if target.iter().any(|b| compat[..4] == b[..]) {
            return true;
        }
        offset += 4;
    }
    false
}

fn detect_avif(data: &[u8]) -> bool {
    if !has_ftyp(data) {
        return false;
    }
    let major = &data[8..12];
    if major == b"avif" || major == b"avis" {
        return true;
    }
    if major == b"mif1" || major == b"msf1" {
        scan_compat_brands(data, &[b"avif", b"avis"])
    } else {
        false
    }
}

fn detect_heic(data: &[u8]) -> bool {
    if !has_ftyp(data) {
        return false;
    }
    let major = &data[8..12];
    if HEIC_BRANDS.iter().any(|b| major == &b[..]) {
        return true;
    }
    if major == b"mif1" || major == b"msf1" {
        scan_compat_brands(data, HEIC_BRANDS)
    } else {
        false
    }
}

fn detect_jxl(data: &[u8]) -> bool {
    // Codestream: FF 0A
    if data.len() >= 2 && data[0] == 0xFF && data[1] == 0x0A {
        return true;
    }
    // Container: 00 00 00 0C 4A 58 4C 20 0D 0A 87 0A
    data.len() >= 12
        && data[..4] == [0x00, 0x00, 0x00, 0x0C]
        && data[4..8] == [b'J', b'X', b'L', b' ']
        && data[8..12] == [0x0D, 0x0A, 0x87, 0x0A]
}

// ------- Built-in format definitions -------

pub static JPEG: ImageFormatDefinition = ImageFormatDefinition {
    name: "jpeg",
    image_format: Some(ImageFormat::Jpeg),
    display_name: "JPEG",
    preferred_extension: "jpg",
    extensions: &["jpg", "jpeg", "jpe", "jfif"],
    preferred_mime_type: "image/jpeg",
    mime_types: &["image/jpeg"],
    supports_alpha: false,
    supports_animation: false,
    supports_lossless: false,
    supports_lossy: true,
    magic_bytes_needed: 2048,
    detect: |data| data.len() >= 3 && data[0] == 0xFF && data[1] == 0xD8 && data[2] == 0xFF,
};

pub static PNG: ImageFormatDefinition = ImageFormatDefinition {
    name: "png",
    image_format: Some(ImageFormat::Png),
    display_name: "PNG",
    preferred_extension: "png",
    extensions: &["png"],
    preferred_mime_type: "image/png",
    mime_types: &["image/png"],
    supports_alpha: true,
    supports_animation: true,
    supports_lossless: true,
    supports_lossy: false,
    magic_bytes_needed: 33,
    detect: |data| data.len() >= 8 && data[..8] == [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A],
};

pub static GIF: ImageFormatDefinition = ImageFormatDefinition {
    name: "gif",
    image_format: Some(ImageFormat::Gif),
    display_name: "GIF",
    preferred_extension: "gif",
    extensions: &["gif"],
    preferred_mime_type: "image/gif",
    mime_types: &["image/gif"],
    supports_alpha: true,
    supports_animation: true,
    supports_lossless: true,
    supports_lossy: false,
    magic_bytes_needed: 13,
    detect: |data| {
        data.len() >= 6
            && data[..3] == *b"GIF"
            && data[3] == b'8'
            && (data[4] == b'7' || data[4] == b'9')
            && data[5] == b'a'
    },
};

pub static WEBP: ImageFormatDefinition = ImageFormatDefinition {
    name: "webp",
    image_format: Some(ImageFormat::WebP),
    display_name: "WebP",
    preferred_extension: "webp",
    extensions: &["webp"],
    preferred_mime_type: "image/webp",
    mime_types: &["image/webp"],
    supports_alpha: true,
    supports_animation: true,
    supports_lossless: true,
    supports_lossy: true,
    magic_bytes_needed: 30,
    detect: |data| data.len() >= 12 && data[..4] == *b"RIFF" && data[8..12] == *b"WEBP",
};

pub static AVIF: ImageFormatDefinition = ImageFormatDefinition {
    name: "avif",
    image_format: Some(ImageFormat::Avif),
    display_name: "AVIF",
    preferred_extension: "avif",
    extensions: &["avif"],
    preferred_mime_type: "image/avif",
    mime_types: &["image/avif"],
    supports_alpha: true,
    supports_animation: true,
    supports_lossless: true,
    supports_lossy: true,
    magic_bytes_needed: 512,
    detect: detect_avif,
};

pub static JXL: ImageFormatDefinition = ImageFormatDefinition {
    name: "jxl",
    image_format: Some(ImageFormat::Jxl),
    display_name: "JPEG XL",
    preferred_extension: "jxl",
    extensions: &["jxl"],
    preferred_mime_type: "image/jxl",
    mime_types: &["image/jxl"],
    supports_alpha: true,
    supports_animation: true,
    supports_lossless: true,
    supports_lossy: true,
    magic_bytes_needed: 256,
    detect: detect_jxl,
};

pub static HEIC: ImageFormatDefinition = ImageFormatDefinition {
    name: "heic",
    image_format: Some(ImageFormat::Heic),
    display_name: "HEIC",
    preferred_extension: "heif",
    extensions: &["heic", "heif", "hif"],
    preferred_mime_type: "image/heif",
    mime_types: &["image/heif", "image/heic"],
    supports_alpha: true,
    supports_animation: false,
    supports_lossless: false,
    supports_lossy: true,
    magic_bytes_needed: 512,
    detect: detect_heic,
};

pub static BMP: ImageFormatDefinition = ImageFormatDefinition {
    name: "bmp",
    image_format: Some(ImageFormat::Bmp),
    display_name: "BMP",
    preferred_extension: "bmp",
    extensions: &["bmp"],
    preferred_mime_type: "image/bmp",
    mime_types: &["image/bmp", "image/x-bmp"],
    supports_alpha: true,
    supports_animation: false,
    supports_lossless: true,
    supports_lossy: false,
    magic_bytes_needed: 54,
    detect: |data| data.len() >= 2 && data[0] == b'B' && data[1] == b'M',
};

pub static FARBFELD: ImageFormatDefinition = ImageFormatDefinition {
    name: "farbfeld",
    image_format: Some(ImageFormat::Farbfeld),
    display_name: "farbfeld",
    preferred_extension: "ff",
    extensions: &["ff"],
    preferred_mime_type: "image/x-farbfeld",
    mime_types: &["image/x-farbfeld"],
    supports_alpha: true,
    supports_animation: false,
    supports_lossless: true,
    supports_lossy: false,
    magic_bytes_needed: 16,
    detect: |data| data.len() >= 8 && data[..8] == *b"farbfeld",
};

pub static PNM: ImageFormatDefinition = ImageFormatDefinition {
    name: "pnm",
    image_format: Some(ImageFormat::Pnm),
    display_name: "PNM",
    preferred_extension: "pnm",
    extensions: &["pnm", "ppm", "pgm", "pbm", "pam", "pfm"],
    preferred_mime_type: "image/x-portable-anymap",
    mime_types: &[
        "image/x-portable-anymap",
        "image/x-portable-pixmap",
        "image/x-portable-graymap",
        "image/x-portable-bitmap",
    ],
    supports_alpha: true,
    supports_animation: false,
    supports_lossless: true,
    supports_lossy: false,
    magic_bytes_needed: 20,
    detect: |data| {
        data.len() >= 2 && data[0] == b'P' && matches!(data[1], b'1'..=b'7' | b'F' | b'f')
    },
};

pub static TIFF: ImageFormatDefinition = ImageFormatDefinition {
    name: "tiff",
    image_format: Some(ImageFormat::Tiff),
    display_name: "TIFF",
    preferred_extension: "tiff",
    extensions: &["tiff", "tif"],
    preferred_mime_type: "image/tiff",
    mime_types: &["image/tiff"],
    supports_alpha: true,
    supports_animation: false,
    supports_lossless: true,
    supports_lossy: false,
    magic_bytes_needed: 8,
    detect: |data| {
        data.len() >= 4
            && ((data[0] == b'I' && data[1] == b'I' && data[2] == 42 && data[3] == 0)
                || (data[0] == b'M' && data[1] == b'M' && data[2] == 0 && data[3] == 42))
    },
};

pub static ICO: ImageFormatDefinition = ImageFormatDefinition {
    name: "ico",
    image_format: Some(ImageFormat::Ico),
    display_name: "ICO",
    preferred_extension: "ico",
    extensions: &["ico", "cur"],
    preferred_mime_type: "image/x-icon",
    mime_types: &["image/x-icon", "image/vnd.microsoft.icon"],
    supports_alpha: true,
    supports_animation: false,
    supports_lossless: true,
    supports_lossy: false,
    magic_bytes_needed: 22,
    detect: |data| {
        data.len() >= 4
            && data[0] == 0
            && data[1] == 0
            && (data[2] == 1 || data[2] == 2)
            && data[3] == 0
    },
};

pub static QOI: ImageFormatDefinition = ImageFormatDefinition {
    name: "qoi",
    image_format: Some(ImageFormat::Qoi),
    display_name: "QOI",
    preferred_extension: "qoi",
    extensions: &["qoi"],
    preferred_mime_type: "image/x-qoi",
    mime_types: &["image/x-qoi"],
    supports_alpha: true,
    supports_animation: false,
    supports_lossless: true,
    supports_lossy: false,
    magic_bytes_needed: 14,
    detect: |data| data.len() >= 4 && data[..4] == *b"qoif",
};

/// All built-in definitions in detection priority order.
///
/// Order matters: JPEG first (most common), AVIF before HEIC
/// (for ambiguous mif1/msf1 containers, AVIF takes priority).
pub static ALL: &[&ImageFormatDefinition] = &[
    &JPEG, &PNG, &GIF, &WEBP, &AVIF, &JXL, &HEIC, &BMP, &FARBFELD, &PNM, &TIFF, &ICO, &QOI,
];
