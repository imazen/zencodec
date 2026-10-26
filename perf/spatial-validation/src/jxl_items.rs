//! Qualifying the shared selection concepts against a native JXL decoder.
//! This path retains the input and scans visible presentations, not raw layers.
use super::{
    collection::{Collection, CollectionError, located},
    image_items::{Composition, ImageId, ImageRequest, Origin, RegionRequest, SelectionError},
    tiff_region::Failure,
};
use std::io::{Error, ErrorKind};
use zenjxl_decoder::api::{
    JxlDataFormat, JxlDecoder, JxlDecoderOptions, JxlOutputBuffer, JxlPixelFormat,
    ProcessingResult, VisibleFrameInfo, states,
};

pub struct JxlItems<'a> {
    data: &'a [u8],
    remaining: &'a [u8],
    scanner: Option<JxlDecoder<states::WithImageInfo>>,
    origin: Origin,
    visible: Vec<VisibleFrameInfo>,
    limit: usize,
    failed: bool,
}

fn complete<T, U>(result: ProcessingResult<T, U>) -> Result<T, Failure> {
    match result {
        ProcessingResult::Complete { result } => Ok(result),
        ProcessingResult::NeedsMoreInput { .. } => {
            Err(Error::new(ErrorKind::UnexpectedEof, "incomplete JXL input").into())
        }
    }
}

fn options() -> JxlDecoderOptions {
    JxlDecoderOptions::default()
        .with_adjust_orientation(false)
        .with_dither_u8(false)
}

pub struct Presentation {
    pub image: ImageId,
    pub native: VisibleFrameInfo,
}

/// Raw native channel bytes, with explicit row layout. f32 samples are compared
/// bitwise, including NaNs; this is not a color conversion or display encoding.
#[derive(Debug, PartialEq, Eq)]
pub struct Plane {
    pub bytes: Vec<u8>,
    pub width: usize,
    pub height: usize,
    pub channels: usize,
    pub stride: usize,
}

impl<'a> JxlItems<'a> {
    /// The view is fixed before discovery: flattened presentations and exposed
    /// layers have different cardinalities, so changing it at decode is unsound.
    pub fn open(data: &'a [u8], composition: Composition, limit: usize) -> Result<Self, Failure> {
        if composition != Composition::Composited {
            return Err(SelectionError::UnsupportedComposition.into());
        }
        let mut remaining = data;
        let decoder = JxlDecoder::new(options().with_scan_frames_only(true));
        let scanner = complete(decoder.process(&mut remaining)?)?;
        Ok(Self {
            data,
            remaining,
            scanner: Some(scanner),
            origin: Origin::default(),
            visible: Vec::new(),
            limit,
            failed: false,
        })
    }

    pub fn next(&mut self) -> Result<Option<Presentation>, Failure> {
        if self.failed {
            return Err(Error::other("JXL discovery failed; open a new operation").into());
        }
        let result = self.next_inner();
        if result.is_err() {
            self.failed = true;
            self.scanner = None;
        }
        result
    }

    fn next_inner(&mut self) -> Result<Option<Presentation>, Failure> {
        let Some(scanner) = self.scanner.as_ref() else {
            return Ok(None);
        };
        if !scanner.has_more_frames() {
            self.scanner = None;
            return Ok(None);
        }
        if self.visible.len() >= self.limit {
            return Err(SelectionError::DiscoveryLimit.into());
        }
        self.visible.try_reserve_exact(1)?;
        let scanner = self.scanner.take().expect("checked above");
        let frame = complete(scanner.process(&mut self.remaining)?)?;
        let scanner = complete(frame.skip_frame(&mut self.remaining)?)?;
        let native = scanner
            .scanned_frames()
            .last()
            .ok_or_else(|| Error::other("native scan returned no visible presentation"))?
            .clone();
        if native.index != self.visible.len() {
            return Err(Error::other("native visible index is not sequential").into());
        }
        let image = self.origin.image(native.index as u64);
        self.visible.push(native.clone());
        self.scanner = Some(scanner);
        Ok(Some(Presentation { image, native }))
    }

    pub fn decode(&self, image: &ImageId, padding: usize) -> Result<Vec<Plane>, Failure> {
        let (decoder, mut input) = self.selected_decoder(image)?;
        let (_, planes) = decode_next(decoder, &mut input, padding)?;
        Ok(planes)
    }

    pub fn decode_image(
        &self,
        image: &ImageId,
    ) -> Result<super::image_output::ImageOutput, Failure> {
        let (mut decoder, mut input) = self.selected_decoder(image)?;
        let index = usize::try_from(self.origin.key(image)?)?;
        let selected = &self.visible[index];
        let info = decoder.basic_info().clone();
        let embedded_color = decoder.embedded_color_profile().clone();
        let mut extra_details = Vec::new();
        extra_details.try_reserve_exact(info.extra_channels.len())?;
        for index in 0..info.extra_channels.len() {
            extra_details.push(
                decoder
                    .extra_channel_details(index)
                    .ok_or_else(|| Error::other("native extra-channel details unavailable"))?,
            );
        }
        let mut format = decoder.current_pixel_format().clone();
        if info
            .extra_channels
            .iter()
            .any(|ec| ec.ec_type == zenjxl_decoder::api::ExtraChannel::Alpha)
            && !format.color_type.has_alpha()
        {
            format.color_type = format
                .color_type
                .add_alpha()
                .ok_or_else(|| Error::other("packed color cannot carry required alpha"))?;
            decoder.set_pixel_format(format.clone());
        }
        let profile = decoder.output_color_profile().clone();
        // PixelSlice requires whole-pixel strides, not just sample alignment.
        // We own these buffers, so request one pixel of padding. The native
        // 13-byte padding stress path above remains a separate regression test.
        let padding = format.color_type.samples_per_pixel() * 4;
        let decoded = decode_next_details(decoder, &mut input, padding)?;
        let timing = if let Some(animation) = &info.animation {
            let ticks = self.visible[..index].iter().try_fold(0i64, |sum, frame| {
                sum.checked_add(i64::from(frame.duration_ticks))
                    .ok_or_else(|| Error::other("JXL timestamp overflow"))
            })?;
            let base =
                zencodec::time::TimeBase::new(animation.tps_denominator, animation.tps_numerator)?;
            Some(
                zencodec::frame::FrameTiming::at(zencodec::time::Timestamp::new(ticks, base))
                    .with_duration_ticks(u64::from(self.visible[index].duration_ticks)),
            )
        } else {
            None
        };
        super::jxl_output::adopt(
            decoded.planes,
            super::jxl_output::JxlSourceFacts {
                header: info,
                embedded_color,
                frame_index: index,
                frame_name: selected.name.clone(),
                is_last: selected.is_last,
                frame_timecode: decoded.timecode,
                extra_details,
            },
            &format,
            &profile,
            timing,
        )
    }

    fn selected_decoder(
        &self,
        image: &ImageId,
    ) -> Result<(JxlDecoder<states::WithImageInfo>, &[u8]), Failure> {
        let key = self.origin.key(image)?;
        if self.failed {
            return Err(Error::other("JXL operation failed").into());
        }
        let index = usize::try_from(key)?;
        let frame = self
            .visible
            .get(index)
            .ok_or_else(|| Error::other("unknown image"))?;
        let target = frame.seek_target.ok_or_else(|| {
            Error::new(
                ErrorKind::Unsupported,
                "native source has no direct seek target",
            )
        })?;
        let (mut decoder, _) = pixel_decoder(self.data, options())?;
        decoder.start_new_frame(target);
        let start = usize::try_from(target.decode_start_file_offset)?;
        let input = self
            .data
            .get(start..)
            .ok_or_else(|| Error::other("native seek outside input"))?;
        Ok((decoder, input))
    }
}

fn pixel_decoder(
    data: &[u8],
    options: JxlDecoderOptions,
) -> Result<(JxlDecoder<states::WithImageInfo>, &[u8]), Failure> {
    let mut input = data;
    let mut decoder = complete(JxlDecoder::new(options).process(&mut input)?)?;
    let format = decoder.current_pixel_format().clone();
    decoder.set_pixel_format(JxlPixelFormat {
        color_type: format.color_type,
        color_data_format: Some(JxlDataFormat::f32()),
        extra_channel_format: format
            .extra_channel_format
            .iter()
            .map(|_| Some(JxlDataFormat::f32()))
            .collect(),
    });
    Ok((decoder, input))
}

fn decode_next(
    decoder: JxlDecoder<states::WithImageInfo>,
    input: &mut &[u8],
    padding: usize,
) -> Result<(JxlDecoder<states::WithImageInfo>, Vec<Plane>), Failure> {
    let decoded = decode_next_details(decoder, input, padding)?;
    Ok((decoded.decoder, decoded.planes))
}

struct DecodedNext {
    decoder: JxlDecoder<states::WithImageInfo>,
    planes: Vec<Plane>,
    timecode: Option<u32>,
}

fn decode_next_details(
    decoder: JxlDecoder<states::WithImageInfo>,
    input: &mut &[u8],
    padding: usize,
) -> Result<DecodedNext, Failure> {
    let (width, height) = decoder.basic_info().size;
    let format = decoder.current_pixel_format();
    let mut planes = Vec::new();
    planes.try_reserve_exact(1 + format.extra_channel_format.len())?;
    for channels in std::iter::once(format.color_type.samples_per_pixel()).chain(
        format
            .extra_channel_format
            .iter()
            .filter(|v| v.is_some())
            .map(|_| 1),
    ) {
        let stride = width
            .checked_mul(channels)
            .and_then(|v| v.checked_mul(4))
            .and_then(|v| v.checked_add(padding))
            .ok_or_else(|| Error::other("row overflow"))?;
        let size = stride
            .checked_mul(height)
            .ok_or_else(|| Error::other("plane overflow"))?;
        let mut bytes = Vec::new();
        bytes.try_reserve_exact(size)?;
        bytes.resize(size, 0xa5);
        planes.push(Plane {
            bytes,
            width,
            height,
            channels,
            stride,
        });
    }
    let frame = complete(decoder.process(input)?)?;
    let timecode = frame.frame_timecode();
    let mut buffers: Vec<_> = planes
        .iter_mut()
        .map(|p| {
            JxlOutputBuffer::new_with_stride(
                &mut p.bytes,
                p.height,
                p.width * p.channels * 4,
                p.stride,
            )
        })
        .collect();
    let decoder = complete(frame.process(input, &mut buffers)?)?;
    Ok(DecodedNext {
        decoder,
        planes,
        timecode,
    })
}

pub fn sequential(data: &[u8], coalescing: bool) -> Result<Vec<Vec<Plane>>, Failure> {
    let (mut decoder, mut input) = pixel_decoder(data, options().with_coalescing(coalescing))?;
    let mut frames = Vec::new();
    while decoder.has_more_frames() {
        let (next, planes) = decode_next(decoder, &mut input, 0)?;
        frames.try_reserve_exact(1)?;
        frames.push(planes);
        decoder = next;
    }
    Ok(frames)
}

pub struct JxlCursor {
    owner: ImageId,
    index: usize,
}

impl Collection for JxlItems<'_> {
    type Cursor = JxlCursor;
    fn view(&self) -> Composition {
        Composition::Composited
    }
    fn images(&mut self) -> Result<JxlCursor, zencodec::At<CollectionError>> {
        Ok(JxlCursor {
            owner: self.origin.image(u64::MAX),
            index: 0,
        })
    }
    fn children(&mut self, image: &ImageId) -> Result<JxlCursor, zencodec::At<CollectionError>> {
        self.origin.key(image).map_err(|e| located(e.into()))?;
        Err(located(SelectionError::UnsupportedChildren.into()))
    }
    fn next(
        &mut self,
        cursor: &mut JxlCursor,
    ) -> Result<Option<ImageId>, zencodec::At<CollectionError>> {
        self.origin
            .key(&cursor.owner)
            .map_err(|_| located(SelectionError::ForeignCursor.into()))?;
        if self.failed {
            return Err(located(Error::other("JXL operation failed").into()));
        }
        let image = if cursor.index < self.visible.len() {
            Some(self.origin.image(cursor.index as u64))
        } else {
            self.next()
                .map(|item| item.map(|v| v.image))
                .map_err(located)?
        };
        if image.is_some() {
            cursor.index += 1;
        }
        Ok(image)
    }
    fn decode(
        &mut self,
        image: &ImageId,
        request: &ImageRequest,
    ) -> Result<super::image_output::ImageOutput, zencodec::At<CollectionError>> {
        self.origin.key(image).map_err(|e| located(e.into()))?;
        if request.region != RegionRequest::Full {
            return Err(located(SelectionError::UnsupportedRegion.into()));
        }
        self.decode_image(image).map_err(located)
    }
}
