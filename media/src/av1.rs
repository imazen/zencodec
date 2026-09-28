//! Native AV1 ownership and mapping, with a sequential IVF decoder.

use crate::{
    color::{ChromaLocation, ColorError, Subsampling, YuvView},
    ivf::{IvfInfo, IvfPacket, IvfReader},
    plane::{Plane, PlaneError, Samples},
    time::{TimeBase, Timestamp},
};
use rav1d_safe::{Frame, PlaneView8, PlaneView16, Planes, ReceiveStatus, SendStatus};
use std::{
    fmt,
    io::{self, Read},
    sync::Arc,
};
use zenpixels::{ChannelType, Cicp, sample::SampleEncoding};

/// Owned backend picture. Cloning retains the allocation without copying pixels.
#[derive(Clone)]
pub struct Av1Frame {
    inner: Frame,
    time_base: TimeBase,
}

impl Av1Frame {
    pub fn is_keyframe(&self) -> bool {
        self.inner.is_keyframe()
    }
    pub fn is_show_existing(&self) -> bool {
        self.inner.is_show_existing()
    }
    pub fn width(&self) -> u32 {
        self.inner.width()
    }
    pub fn height(&self) -> u32 {
        self.inner.height()
    }
    pub fn bit_depth(&self) -> u8 {
        self.inner.bit_depth()
    }
    pub fn render_size(&self) -> (u32, u32) {
        self.inner.render_size()
    }
    pub fn timestamp(&self) -> Option<Timestamp> {
        let ticks = self.inner.timestamp();
        (ticks != i64::MIN).then(|| Timestamp::new(ticks, self.time_base))
    }
    pub fn input_offset(&self) -> Option<u64> {
        u64::try_from(self.inner.input_offset()).ok()
    }

    /// Bitstream color claim; unspecified and reserved codes remain unchanged.
    pub fn color(&self) -> Cicp {
        let c = self.inner.raw_color_info();
        Cicp::new(
            c.primaries,
            c.transfer_characteristics,
            c.matrix_coefficients,
            c.full_range,
        )
    }

    /// Raw AV1 position: 0 unknown, 1 vertical, 2 colocated, 3 reserved.
    pub fn chroma_sample_position(&self) -> u8 {
        self.inner.raw_color_info().chroma_sample_position
    }

    /// Retain the backend's borrow guards. Returned views borrow this mapping,
    /// so no self-referential slices or copies are needed. Other frames may be
    /// decoded while this frame and its mapping remain alive.
    pub fn map(&self) -> MappedAv1<'_> {
        let planes = match self.inner.planes() {
            Planes::Depth8(p) => MappedPlanes::U8([Some(p.y()), p.u(), p.v()]),
            Planes::Depth16(p) => MappedPlanes::U16([Some(p.y()), p.u(), p.v()]),
        };
        let storage = if self.bit_depth() == 8 {
            ChannelType::U8
        } else {
            ChannelType::U16
        };
        let encoding = SampleEncoding::new(storage, self.bit_depth(), 0)
            .expect("AV1 uses 8/10/12-bit unsigned codes");
        let subsampling = match self.inner.pixel_layout() {
            rav1d_safe::PixelLayout::I400 => Subsampling::Monochrome,
            rav1d_safe::PixelLayout::I420 => Subsampling::Yuv420,
            rav1d_safe::PixelLayout::I422 => Subsampling::Yuv422,
            rav1d_safe::PixelLayout::I444 => Subsampling::Yuv444,
        };
        let location = match self.chroma_sample_position() {
            1 => ChromaLocation::Left,
            2 => ChromaLocation::TopLeft,
            _ => ChromaLocation::Unknown,
        };
        MappedAv1 {
            planes,
            encoding,
            subsampling,
            location,
            color: self.color(),
        }
    }
}

enum MappedPlanes<'a> {
    U8([Option<PlaneView8<'a>>; 3]),
    U16([Option<PlaneView16<'a>>; 3]),
}

pub struct MappedAv1<'a> {
    planes: MappedPlanes<'a>,
    encoding: SampleEncoding,
    subsampling: Subsampling,
    location: ChromaLocation,
    color: Cicp,
}

impl MappedAv1<'_> {
    /// Borrow a checked native view without losing the bitstream's matrix,
    /// range or sample precision. Unknown/reserved chroma positions stay unknown;
    /// reconstruction requires an explicit interpretation for subsampled data.
    pub fn yuv_view(&self) -> Result<YuvView<'_>, ColorError> {
        let luma = self.plane(0)?.ok_or(ColorError::ComponentLayout)?;
        let chroma = match (self.plane(1)?, self.plane(2)?) {
            (Some(cb), Some(cr)) => Some([cb, cr]),
            (None, None) => None,
            _ => return Err(ColorError::ComponentLayout),
        };
        YuvView::new(luma, chroma, self.subsampling, self.location, self.color)
    }

    /// AV1 component index: 0 Y, 1 Cb, 2 Cr. Identity-matrix streams instead
    /// use G, B, R in these slots. Monochrome has only component 0.
    pub fn plane(&self, component: usize) -> Result<Option<Plane<'_>>, PlaneError> {
        match &self.planes {
            MappedPlanes::U8(planes) => planes
                .get(component)
                .ok_or(PlaneError::OutsidePlane)?
                .as_ref()
                .map(|p| {
                    Plane::new(
                        Samples::U8(p.as_slice()),
                        p.width(),
                        p.height(),
                        p.stride(),
                        self.encoding,
                    )
                })
                .transpose(),
            MappedPlanes::U16(planes) => planes
                .get(component)
                .ok_or(PlaneError::OutsidePlane)?
                .as_ref()
                .map(|p| {
                    let stride = p
                        .stride()
                        .checked_mul(2)
                        .ok_or(PlaneError::GeometryOverflow)?;
                    Plane::new(
                        Samples::U16(p.as_slice()),
                        p.width(),
                        p.height(),
                        stride,
                        self.encoding,
                    )
                })
                .transpose(),
        }
    }
}

/// One-packet-at-a-time decoder over a sequential source, including network reads.
/// This holds at most one unaccepted compressed packet in addition to backend
/// references. The caller chooses how many returned frames to retain.
pub struct Av1IvfDecoder<R> {
    input: IvfReader<R>,
    decoder: rav1d_safe::Decoder,
    pending: Option<rav1d_safe::Packet>,
    failed: bool,
    stop: Option<Arc<dyn enough::Stop>>,
}

impl<R: Read> Av1IvfDecoder<R> {
    pub fn new(
        reader: R,
        max_packet_bytes: usize,
        settings: rav1d_safe::Settings,
    ) -> Result<Self, DecodeError> {
        let input = IvfReader::new(reader, max_packet_bytes)?;
        Self::from_input(input, settings, None)
    }

    pub(crate) fn from_input(
        input: IvfReader<R>,
        settings: rav1d_safe::Settings,
        sequence: Option<&[u8]>,
    ) -> Result<Self, DecodeError> {
        let mut decoder =
            rav1d_safe::Decoder::with_settings(settings).map_err(DecodeError::Codec)?;
        if let Some(sequence) = sequence {
            let mut bytes = Vec::new();
            bytes.try_reserve_exact(sequence.len()).map_err(|_| {
                io::Error::new(
                    io::ErrorKind::OutOfMemory,
                    "sequence-header allocation failed",
                )
            })?;
            bytes.extend_from_slice(sequence);
            let mut packet = rav1d_safe::Packet::new(bytes).map_err(DecodeError::Codec)?;
            if decoder
                .send_packet(&mut packet)
                .map_err(DecodeError::Codec)?
                != SendStatus::Accepted
            {
                return Err(DecodeError::Protocol(
                    "fresh decoder rejected sequence initialization",
                ));
            }
        }
        Ok(Self {
            input,
            decoder,
            pending: None,
            failed: false,
            stop: None,
        })
    }

    /// Cooperatively cancel codec work and the boundaries between input reads.
    /// Blocking `Read` itself must supply a timeout/cancellation mechanism; this
    /// token cannot interrupt an arbitrary reader while its `read` is blocked.
    pub fn set_stop(&mut self, stop: Option<Arc<dyn enough::Stop>>) {
        self.decoder.set_stop(stop.clone());
        self.stop = stop;
    }

    pub fn info(&self) -> IvfInfo {
        self.input.info()
    }

    pub fn into_inner(self) -> R {
        self.input.into_inner()
    }

    /// Return one owned presentation frame. Clean EOF is distinct from malformed
    /// or truncated input; errors poison the stream instead of silently resuming.
    pub fn next_frame(&mut self) -> Result<Option<Av1Frame>, DecodeError> {
        self.next_frame_observed(&mut |_| Ok(()))
    }

    pub(crate) fn next_frame_observed(
        &mut self,
        observe: &mut impl FnMut(&IvfPacket) -> io::Result<()>,
    ) -> Result<Option<Av1Frame>, DecodeError> {
        if self.failed {
            return Err(DecodeError::FailedStream);
        }
        let result = self.next_frame_inner(observe);
        if result.is_err() {
            self.failed = true;
        }
        result
    }

    fn next_frame_inner(
        &mut self,
        observe: &mut impl FnMut(&IvfPacket) -> io::Result<()>,
    ) -> Result<Option<Av1Frame>, DecodeError> {
        loop {
            if self.stop.as_ref().is_some_and(|s| s.should_stop()) {
                return Err(DecodeError::Codec(whereat::at(
                    rav1d_safe::Error::Cancelled,
                )));
            }
            match self.decoder.receive().map_err(DecodeError::Codec)? {
                ReceiveStatus::Frame(inner) => {
                    return Ok(Some(Av1Frame {
                        inner,
                        time_base: self.input.info().time_base(),
                    }));
                }
                ReceiveStatus::EndOfStream => return Ok(None),
                ReceiveStatus::NeedInput => {}
                _ => {
                    return Err(DecodeError::Protocol("unrecognized decoder receive state"));
                }
            }
            if self.pending.is_none() {
                let Some(packet) = self.input.next_packet()? else {
                    self.decoder.end_input();
                    continue;
                };
                observe(&packet)?;
                let offset = i64::try_from(packet.byte_offset)
                    .map_err(|_| DecodeError::InputOffsetOverflow)?;
                if packet.timestamp.ticks() == i64::MIN {
                    return Err(DecodeError::ReservedTimestamp);
                }
                self.pending = Some(
                    rav1d_safe::Packet::new(packet.data)
                        .map_err(DecodeError::Codec)?
                        .with_timestamp(packet.timestamp.ticks())
                        .with_offset(offset),
                );
            }
            match self
                .decoder
                .send_packet(self.pending.as_mut().expect("pending packet"))
                .map_err(DecodeError::Codec)?
            {
                SendStatus::Accepted => self.pending = None,
                SendStatus::ReceivePending => {}
                _ => return Err(DecodeError::Protocol("unrecognized decoder send state")),
            }
        }
    }
}

#[derive(Debug)]
#[non_exhaustive]
pub enum DecodeError {
    Io(io::Error),
    Codec(whereat::At<rav1d_safe::Error>),
    Protocol(&'static str),
    InputOffsetOverflow,
    ReservedTimestamp,
    FailedStream,
}
impl From<io::Error> for DecodeError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}
impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => e.fmt(f),
            Self::Codec(e) => write!(f, "AV1 decoding failed: {e}"),
            Self::Protocol(message) => f.write_str(message),
            Self::InputOffsetOverflow => {
                f.write_str("input offset exceeds backend metadata capacity")
            }
            Self::ReservedTimestamp => {
                f.write_str("known timestamp collides with backend unknown sentinel")
            }
            Self::FailedStream => f.write_str("decoder cannot resume after a stream error"),
        }
    }
}
impl std::error::Error for DecodeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            Self::Codec(e) => Some(e.error()),
            _ => None,
        }
    }
}
