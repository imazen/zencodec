//! Tracks: what a container says about each stream.
//!
//! Codec identity ([`CodecId`]) is separate from the container: AV1 lives
//! in IVF, WebM, MP4 and AVIF alike, and each container crate keeps its own
//! mapping from fourccs and codec ids to `CodecId`. [`TrackInfo`] has
//! private fields and a validating builder so new fields can be added
//! without breaking anyone.

use alloc::boxed::Box;
use alloc::string::String;
use alloc::sync::Arc;
use core::fmt;

use zencodec::time::{FrameDuration, TimeBase};
use zenpixels::Cicp;

/// What a track carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum TrackKind {
    /// Pictures.
    Video,
    /// Sound.
    Audio,
    /// Timed text.
    Subtitle,
    /// Anything else (timed metadata, attachments).
    Data,
}

/// A codec identifier read from a file that no [`CodecId`] variant names: a
/// fourcc, a Matroska `CodecID` string. Owned, so it can hold whatever the
/// file says.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct CodecTag(Box<str>);

impl CodecTag {
    /// A tag holding `tag`.
    pub fn new(tag: &str) -> Self {
        Self(String::from(tag).into_boxed_str())
    }

    /// The tag text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A codec, independent of the container that carries it.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum CodecId {
    /// AV1.
    Av1,
    /// VP9.
    Vp9,
    /// VP8.
    Vp8,
    /// H.264 / AVC.
    H264,
    /// H.265 / HEVC.
    Hevc,
    /// AAC.
    Aac,
    /// Opus.
    Opus,
    /// FLAC.
    Flac,
    /// Uncompressed PCM.
    Pcm,
    /// A codec the container named but this crate does not.
    Other(CodecTag),
}

/// What a container declares about a video track.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct VideoTrackInfo {
    /// Coded width in pixels.
    pub coded_width: u32,
    /// Coded height in pixels.
    pub coded_height: u32,
    /// Intended display size, when the container declares one.
    pub display: Option<(u32, u32)>,
    /// The container's colour claim (MP4 `colr`, WebM `Colour`). The
    /// bitstream's claim travels on each frame; the two are kept apart.
    pub color: Option<Cicp>,
}

impl VideoTrackInfo {
    /// A track of `coded_width` x `coded_height` pictures.
    pub const fn new(coded_width: u32, coded_height: u32) -> Self {
        Self {
            coded_width,
            coded_height,
            display: None,
            color: None,
        }
    }

    /// The declared display size.
    pub const fn with_display(mut self, width: u32, height: u32) -> Self {
        self.display = Some((width, height));
        self
    }

    /// The container's colour claim.
    pub const fn with_color(mut self, color: Cicp) -> Self {
        self.color = Some(color);
        self
    }
}

/// Audio channels: a count, and the layout when the container gives one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ChannelLayout {
    /// Number of channels.
    pub count: u16,
    /// A channel mask (WAVE / ITU order) when the container gives one.
    pub mask: Option<u64>,
}

impl ChannelLayout {
    /// `count` channels with no declared layout.
    pub const fn new(count: u16) -> Self {
        Self { count, mask: None }
    }

    /// The declared channel mask.
    pub const fn with_mask(mut self, mask: u64) -> Self {
        self.mask = Some(mask);
        self
    }
}

/// What a container declares about an audio track.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct AudioTrackInfo {
    /// Samples per second per channel.
    pub sample_rate: u32,
    /// The channels.
    pub channels: ChannelLayout,
}

impl AudioTrackInfo {
    /// A track at `sample_rate` with `channels`.
    pub const fn new(sample_rate: u32, channels: ChannelLayout) -> Self {
        Self {
            sample_rate,
            channels,
        }
    }
}

/// One track of a container: its codec, clock, configuration and what the
/// container declares about it. Built with [`TrackInfo::video`],
/// [`TrackInfo::audio`] or [`TrackInfo::other`].
///
/// Delays and edit offsets are carried untouched, so a stream copy preserves
/// them. They are applied exactly once, by the decoder adapter configured
/// from this track; the session never trims.
#[derive(Clone, Debug)]
pub struct TrackInfo {
    index: u32,
    kind: TrackKind,
    codec: CodecId,
    time_base: TimeBase,
    codec_private: Option<Arc<[u8]>>,
    config_epoch: u32,
    video: Option<VideoTrackInfo>,
    audio: Option<AudioTrackInfo>,
    codec_delay: Option<FrameDuration>,
    seek_preroll: Option<FrameDuration>,
    edit_delay_ticks: Option<i64>,
    declared_packets: Option<u64>,
    declared_duration_ticks: Option<u64>,
}

/// An invalid track description.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum TrackError {
    /// A video track with zero width or height.
    ZeroDimension,
    /// An audio track with a zero sample rate.
    ZeroSampleRate,
    /// An audio track with no channels.
    NoChannels,
}

impl fmt::Display for TrackError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::ZeroDimension => "video track has a zero dimension",
            Self::ZeroSampleRate => "audio track has a zero sample rate",
            Self::NoChannels => "audio track has no channels",
        })
    }
}

impl core::error::Error for TrackError {}

/// Builds a [`TrackInfo`]; [`build`](Self::build) validates it.
#[derive(Clone, Debug)]
pub struct TrackInfoBuilder(TrackInfo);

impl TrackInfo {
    fn base(index: u32, kind: TrackKind, codec: CodecId, time_base: TimeBase) -> TrackInfo {
        TrackInfo {
            index,
            kind,
            codec,
            time_base,
            codec_private: None,
            config_epoch: 0,
            video: None,
            audio: None,
            codec_delay: None,
            seek_preroll: None,
            edit_delay_ticks: None,
            declared_packets: None,
            declared_duration_ticks: None,
        }
    }

    /// A video track.
    pub fn video(
        index: u32,
        codec: CodecId,
        time_base: TimeBase,
        video: VideoTrackInfo,
    ) -> TrackInfoBuilder {
        let mut t = Self::base(index, TrackKind::Video, codec, time_base);
        t.video = Some(video);
        TrackInfoBuilder(t)
    }

    /// An audio track.
    pub fn audio(
        index: u32,
        codec: CodecId,
        time_base: TimeBase,
        audio: AudioTrackInfo,
    ) -> TrackInfoBuilder {
        let mut t = Self::base(index, TrackKind::Audio, codec, time_base);
        t.audio = Some(audio);
        TrackInfoBuilder(t)
    }

    /// A subtitle or data track.
    pub fn other(
        index: u32,
        kind: TrackKind,
        codec: CodecId,
        time_base: TimeBase,
    ) -> TrackInfoBuilder {
        TrackInfoBuilder(Self::base(index, kind, codec, time_base))
    }

    /// The track's index in its container's track list.
    pub fn index(&self) -> u32 {
        self.index
    }

    /// What the track carries.
    pub fn kind(&self) -> TrackKind {
        self.kind
    }

    /// The codec.
    pub fn codec(&self) -> &CodecId {
        &self.codec
    }

    /// The clock its timestamps use.
    pub fn time_base(&self) -> TimeBase {
        self.time_base
    }

    /// Codec configuration bytes from the container (`av1C`, `avcC`,
    /// `OpusHead`), as stored.
    pub fn codec_private(&self) -> Option<&Arc<[u8]>> {
        self.codec_private.as_ref()
    }

    /// The configuration epoch the current `codec_private` belongs to.
    pub fn config_epoch(&self) -> u32 {
        self.config_epoch
    }

    /// Video declarations, for a video track.
    pub fn video_info(&self) -> Option<&VideoTrackInfo> {
        self.video.as_ref()
    }

    /// Audio declarations, for an audio track.
    pub fn audio_info(&self) -> Option<&AudioTrackInfo> {
        self.audio.as_ref()
    }

    /// Decoder delay the container records (Opus pre-skip, AAC priming).
    pub fn codec_delay(&self) -> Option<FrameDuration> {
        self.codec_delay
    }

    /// How much must be decoded before a seek target to output it
    /// correctly (Opus in WebM: 80 ms).
    pub fn seek_preroll(&self) -> Option<FrameDuration> {
        self.seek_preroll
    }

    /// The edit-list offset of the first presentation, in track ticks.
    pub fn edit_delay_ticks(&self) -> Option<i64> {
        self.edit_delay_ticks
    }

    /// How many packets the container says the track has.
    pub fn declared_packets(&self) -> Option<u64> {
        self.declared_packets
    }

    /// The track length the container declares, in track ticks.
    pub fn declared_duration_ticks(&self) -> Option<u64> {
        self.declared_duration_ticks
    }
}

impl TrackInfoBuilder {
    /// Codec configuration bytes.
    pub fn with_codec_private(mut self, bytes: Arc<[u8]>) -> Self {
        self.0.codec_private = Some(bytes);
        self
    }

    /// The configuration epoch.
    pub fn with_config_epoch(mut self, epoch: u32) -> Self {
        self.0.config_epoch = epoch;
        self
    }

    /// Decoder delay.
    pub fn with_codec_delay(mut self, d: FrameDuration) -> Self {
        self.0.codec_delay = Some(d);
        self
    }

    /// Seek pre-roll.
    pub fn with_seek_preroll(mut self, d: FrameDuration) -> Self {
        self.0.seek_preroll = Some(d);
        self
    }

    /// Edit-list offset in track ticks.
    pub fn with_edit_delay_ticks(mut self, ticks: i64) -> Self {
        self.0.edit_delay_ticks = Some(ticks);
        self
    }

    /// Declared packet count.
    pub fn with_declared_packets(mut self, n: u64) -> Self {
        self.0.declared_packets = Some(n);
        self
    }

    /// Declared length in track ticks.
    pub fn with_declared_duration_ticks(mut self, n: u64) -> Self {
        self.0.declared_duration_ticks = Some(n);
        self
    }

    /// Validate and finish.
    ///
    /// # Errors
    ///
    /// A [`TrackError`] for a zero video dimension, a zero sample rate, or
    /// no audio channels.
    pub fn build(self) -> Result<TrackInfo, TrackError> {
        let t = self.0;
        if let Some(v) = &t.video
            && (v.coded_width == 0 || v.coded_height == 0)
        {
            return Err(TrackError::ZeroDimension);
        }
        if let Some(a) = &t.audio {
            if a.sample_rate == 0 {
                return Err(TrackError::ZeroSampleRate);
            }
            if a.channels.count == 0 {
                return Err(TrackError::NoChannels);
            }
        }
        Ok(t)
    }
}
