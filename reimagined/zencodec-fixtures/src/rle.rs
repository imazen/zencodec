//! `ZWH1`: run-length coding of a whole image.
//!
//! The entire codec is the two trait impls below. The state machine, gates,
//! format negotiation and refusals come from [`zencodec::simple::WholeImage`].
//!
//! ```text
//! "ZWH1"  width:u32le  height:u32le  layout:u8   then (count:u8 ≥ 1, value:u8)*
//! ```

use crate::{FixtureError, PACKED, le32, packed_from_byte, packed_to_byte};
use zencodec::encode::EncodeSpec;
use zencodec::simple::{WholeImage, WholeImageDecode, WholeImageEncode};
use zencodec::{
    Bound, CodecError, ErrorKind, Fidelity, FormatInfo, ImageFormat, ImageInfo, Layout,
    MetadataKinds, Result, error,
};
use zenpixels::{PixelBuffer, PixelDescriptor, PixelSlice};

const MAGIC: &[u8; 4] = b"ZWH1";
const HEADER: usize = 13;

static INFO: FormatInfo = FormatInfo::new("zwh", "image/x-zwh", "zwh", 4, |b| b.starts_with(MAGIC));

/// The `ZWH1` format.
pub const FORMAT: ImageFormat = ImageFormat::new(&INFO);

/// The run-length codec. Wrap it: `WholeImage(RleCodec)`.
#[derive(Clone, Copy, Debug, Default)]
pub struct RleCodec;

impl RleCodec {
    /// The codec as a contract codec, for both directions.
    pub fn codec() -> WholeImage<Self> {
        WholeImage(Self)
    }
}

fn bad(e: FixtureError) -> zencodec::Error {
    CodecError::from_native(e).at()
}

impl WholeImageDecode for RleCodec {
    fn name(&self) -> &'static str {
        "zwh"
    }

    fn formats(&self) -> &[ImageFormat] {
        &[FORMAT]
    }

    fn probe(&self, bytes: &[u8]) -> Result<Option<ImageInfo>> {
        if bytes.len() >= 4 && !bytes.starts_with(MAGIC) {
            return Err(bad(FixtureError::BadMagic));
        }
        if bytes.len() < HEADER {
            return Ok(None);
        }
        let descriptor = packed_from_byte(bytes[12]).map_err(bad)?;
        let info = ImageInfo::new(
            FORMAT,
            le32(bytes, 4),
            le32(bytes, 8),
            Layout::Packed(descriptor),
        )
        .with_lossless(true)
        .with_metadata(Default::default(), MetadataKinds::ALL);
        Ok(Some(info))
    }

    fn memory_bound(&self, info: &ImageInfo) -> Bound {
        let bpp = info.native.descriptor().map_or(4, |d| d.bytes_per_pixel()) as u64;
        Bound::AtMost(u64::from(info.width) * u64::from(info.height) * bpp)
    }

    fn decode(
        &self,
        bytes: &[u8],
        info: &ImageInfo,
        output: PixelDescriptor,
    ) -> Result<PixelBuffer> {
        let len = (info.width as usize)
            .checked_mul(info.height as usize)
            .and_then(|n| n.checked_mul(output.bytes_per_pixel()))
            .ok_or_else(|| bad(FixtureError::BadHeader("size overflows")))?;
        let mut pixels = Vec::new();
        pixels
            .try_reserve_exact(len)
            .map_err(|_| error(ErrorKind::OutOfMemory))?;
        let runs = &bytes[HEADER..];
        for run in runs.chunks_exact(2) {
            let (count, value) = (run[0] as usize, run[1]);
            if count == 0 || pixels.len() + count > len {
                return Err(bad(FixtureError::BadStream("run overflows the image")));
            }
            pixels.resize(pixels.len() + count, value);
        }
        if pixels.len() != len {
            return Err(CodecError::new(ErrorKind::Truncated)
                .with_codec("zwh")
                .at());
        }
        PixelBuffer::from_vec(pixels, info.width, info.height, output)
            .map_err(|_| error(ErrorKind::Internal))
    }
}

impl WholeImageEncode for RleCodec {
    fn name(&self) -> &'static str {
        "zwh"
    }

    fn format(&self) -> ImageFormat {
        FORMAT
    }

    fn accepts(&self) -> &[PixelDescriptor] {
        &PACKED
    }

    fn encode(
        &self,
        pixels: PixelSlice<'_>,
        _fidelity: Fidelity,
        _spec: &EncodeSpec,
    ) -> Result<Vec<u8>> {
        let layout = packed_to_byte(pixels.descriptor()).ok_or_else(|| error(ErrorKind::Internal))?;
        let mut out = Vec::with_capacity(HEADER + 64);
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&pixels.width().to_le_bytes());
        out.extend_from_slice(&pixels.rows().to_le_bytes());
        out.push(layout);
        let row_bytes = pixels.width() as usize * pixels.descriptor().bytes_per_pixel();
        let mut run: Option<(u8, u8)> = None;
        for y in 0..pixels.rows() {
            for &byte in &pixels.row(y)[..row_bytes] {
                run = match run {
                    Some((count, value)) if value == byte && count < u8::MAX => {
                        Some((count + 1, value))
                    }
                    Some((count, value)) => {
                        out.extend_from_slice(&[count, value]);
                        Some((1, byte))
                    }
                    None => Some((1, byte)),
                };
            }
        }
        if let Some((count, value)) = run {
            out.extend_from_slice(&[count, value]);
        }
        Ok(out)
    }
}
