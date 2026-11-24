// Adapted from imazen/zenpixels fde59b96632557592cffa9b7301acd8db96ed078.
use crate::helpers::signal_range;
use core::fmt;
use zenpixels::{
    ChannelOrder as ChannelLayout, ImageSpec, PixelView, SampleType as ChannelType, SignalRange,
};

/// Proven exact code depth of the current samples of a canonical U8/U16 image.
///
/// `color_bits = N` states that every current color sample `v` survives
/// re-quantization to N-bit codes: narrowing it to N bits and widening back to
/// the storage depth reproduces `v` exactly, using the canonical rule for the
/// signal range (nearest full-scale rescaling for full range; power-of-two
/// anchor scaling for narrow range). `alpha_bits` states the same for alpha,
/// which is always full scale. Storage depth itself is always exact, so the
/// weakest evidence is `color_bits == alpha_bits == storage bits`.
///
/// This describes current values, never history. Decoding a ten-bit file does
/// not establish ten-bit evidence for RGB samples produced by a YCbCr matrix,
/// and resizing, filtering, color conversion or any mutable access ends it.
/// It is established only by the code that wrote the samples (for example an
/// exact widening of native codes), by an explicit sample scan
/// ([`measure`](Self::measure)), or by an operation that provably preserves it.
/// [`holds_for`](Self::holds_for) checks a statement against rows written
/// elsewhere, such as strips a row driver copied through a mutable view.
/// A statement built with [`new`](Self::new) and attached to a view is an
/// assertion; a consumer that relies on it should check the samples.
///
/// Exactness is defined for the canonical mapping only. A codec whose native
/// narrowing or reconstruction differs does not inherit it: truncating
/// canonical ten-bit level 577 gives code 9, which bit-replicated widening
/// returns as 576. Such a path needs its own qualification.
///
/// Higher depth is not automatically exact. Full-range N-bit grids nest only
/// when N divides the depth: canonical U16 samples holding eight-bit content
/// are exact at 8 and 16 bits but not at 10 or 12, where 252 and 240 of the 256
/// levels change. The original N-bit codes nevertheless remain recoverable at
/// every higher depth; [`color_codes_recoverable_at`](Self::color_codes_recoverable_at)
/// states that weaker guarantee.
///
/// ```
/// use zenpixels::{SampleType as ChannelType, SignalRange};
/// use zencodec::sample::ExactDepth;
/// let ten = ExactDepth::new(ChannelType::U16, SignalRange::Full, 10, 16).unwrap();
/// assert!(ten.color_exact_at(10) && ten.color_exact_at(16));
/// assert!(!ten.color_exact_at(12)); // 10 does not divide 12
/// assert!(ten.color_codes_recoverable_at(12));
/// let narrow = ExactDepth::new(ChannelType::U16, SignalRange::Narrow, 10, 16).unwrap();
/// assert!(narrow.color_exact_at(12)); // narrow grids nest by powers of two
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ExactDepth {
    storage: ChannelType,
    range: SignalRange,
    color_bits: u8,
    alpha_bits: u8,
}

/// An exact-depth statement is impossible for its storage or range.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ExactDepthError {
    /// Evidence is defined for canonical U8 and U16 integer storage only.
    UnsupportedStorage,
    /// Depth is zero or exceeds the storage width.
    InvalidBits,
    /// Narrow-range codes need at least eight bits.
    NarrowBelowEightBits,
    /// The evidence names a different storage type or signal range than the pixels.
    DescriptorMismatch,
    /// Scans cover gray, gray-alpha, RGB, RGBA and BGRA layouts only.
    UnsupportedLayout,
}

impl fmt::Display for ExactDepthError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::UnsupportedStorage => "exact-depth evidence requires U8 or U16 storage",
            Self::InvalidBits => "exact depth must be positive and fit its storage",
            Self::NarrowBelowEightBits => "narrow-range exact depth needs at least 8 bits",
            Self::DescriptorMismatch => "exact-depth evidence disagrees with the pixel descriptor",
            Self::UnsupportedLayout => "exact-depth scans need gray, gray-alpha, RGB, RGBA or BGRA",
        })
    }
}

impl core::error::Error for ExactDepthError {}

impl ExactDepth {
    /// State that current color and alpha samples are exact at the given depths.
    ///
    /// This is an assertion, checked only for consistency with the storage and
    /// range; it does not read samples. Use it where the caller wrote the samples
    /// itself, or after a scan. `alpha_bits` is ignored for layouts without alpha;
    /// pass the storage depth when nothing more is known.
    pub const fn new(
        storage: ChannelType,
        range: SignalRange,
        color_bits: u8,
        alpha_bits: u8,
    ) -> Result<Self, ExactDepthError> {
        let storage_bits = match storage {
            ChannelType::U8 => 8,
            ChannelType::U16 => 16,
            _ => return Err(ExactDepthError::UnsupportedStorage),
        };
        if color_bits == 0
            || color_bits > storage_bits
            || alpha_bits == 0
            || alpha_bits > storage_bits
        {
            return Err(ExactDepthError::InvalidBits);
        }
        match range {
            SignalRange::Full => {}
            SignalRange::Narrow if color_bits >= 8 => {}
            SignalRange::Narrow => return Err(ExactDepthError::NarrowBelowEightBits),
            _ => return Err(ExactDepthError::DescriptorMismatch),
        }
        Ok(Self {
            storage,
            range,
            color_bits,
            alpha_bits,
        })
    }

    /// Storage the statement applies to.
    pub const fn storage(self) -> ChannelType {
        self.storage
    }

    /// Signal range whose canonical rule defines the color grid.
    pub const fn range(self) -> SignalRange {
        self.range
    }

    /// Smallest proven exact color depth.
    pub const fn color_bits(self) -> u8 {
        self.color_bits
    }

    /// Smallest proven exact alpha depth.
    pub const fn alpha_bits(self) -> u8 {
        self.alpha_bits
    }

    /// Whether quantizing color to `bits` and widening back reproduces every sample.
    ///
    /// Canonical mapping only. A depth at or above the storage depth is
    /// trivially exact, so validate a coded depth against the codec first.
    pub const fn color_exact_at(self, bits: u8) -> bool {
        grid_nests(
            self.storage_bits(),
            self.color_bits,
            bits,
            matches!(self.range, SignalRange::Narrow),
        )
    }

    /// Whether quantizing alpha to `bits` and widening back reproduces every sample.
    ///
    /// Alpha is always full scale. `alpha_bits == 1` allows both transparent and
    /// opaque samples; it does not prove an opaque image.
    pub const fn alpha_exact_at(self, bits: u8) -> bool {
        grid_nests(self.storage_bits(), self.alpha_bits, bits, false)
    }

    /// Measure the smallest depths whose grids hold every current sample.
    ///
    /// Color and alpha are measured separately; without real alpha,
    /// `alpha_bits` is the storage depth. Reads only active samples (never row
    /// padding or an RGBX/BGRX padding slot) and stops once only the storage
    /// depth remains. Evidence the view already carries is not consulted. An
    /// empty view is exact at every depth.
    pub fn measure(pixels: &PixelView<'_>) -> Result<Self, ExactDepthError> {
        let descriptor = pixels.spec();
        let roles = Roles::of(descriptor)?;
        let mut color = GridScan::new(roles.storage_bits, roles.narrow);
        let mut alpha = GridScan::new(roles.storage_bits, false);
        let has_alpha = roles.alpha.is_some();
        roles.for_each(pixels, |is_alpha, v| {
            if is_alpha {
                alpha.push(v);
            } else {
                color.push(v);
            }
            !(color.settled() && (!has_alpha || alpha.settled()))
        });
        let alpha_bits = if has_alpha {
            alpha.bits()
        } else {
            roles.storage_bits
        };
        Self::new(
            descriptor.layout.sample,
            signal_range(descriptor),
            color.bits(),
            alpha_bits,
        )
    }

    /// Check the declared grid against storage, packing, range and channel roles.
    pub fn validate_spec(self, spec: &ImageSpec) -> Result<(), ExactDepthError> {
        Roles::of(spec)?;
        if self.storage != spec.layout.sample || self.range != signal_range(spec) {
            return Err(ExactDepthError::DescriptorMismatch);
        }
        Ok(())
    }

    /// Whether every current sample of `rows` lies on this statement's grids.
    ///
    /// Color uses the range's rule and alpha the full-scale rule, each at its
    /// own depth. Returns `Ok(false)` at the first sample off a grid. Reads only
    /// active samples, ignores evidence the view carries, and holds for an
    /// empty view. A view with different storage or signal range is an error.
    ///
    /// ```
    /// use zenpixels::{ImageSpec, PixelView};
    /// use zencodec::sample::ExactDepth;
    /// // Ten-bit codes 0, 1 and 1023 widened to canonical U16.
    /// let bytes: Vec<u8> = [0u16, 64, 65535].iter().flat_map(|w| w.to_ne_bytes()).collect();
    /// let rows = PixelView::new(&bytes, 3, 1, 6, ImageSpec::gray16_srgb())?;
    /// let ten = ExactDepth::measure(&rows)?;
    /// assert_eq!(ten.color_bits(), 10);
    /// assert!(ten.holds_for(&rows)?);
    /// // One edited sample leaves every grid below 16 bits.
    /// let edited: Vec<u8> = [0u16, 65, 65535].iter().flat_map(|w| w.to_ne_bytes()).collect();
    /// let edited = PixelView::new(&edited, 3, 1, 6, ImageSpec::gray16_srgb())?;
    /// assert!(!ten.holds_for(&edited)?);
    /// assert_eq!(ExactDepth::measure(&edited)?.color_bits(), 16);
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn holds_for(self, rows: &PixelView<'_>) -> Result<bool, ExactDepthError> {
        let descriptor = rows.spec();
        self.validate_spec(descriptor)?;
        let roles = Roles::of(descriptor)?;
        let storage_bits = roles.storage_bits;
        Ok(roles.for_each(rows, |is_alpha, v| {
            if is_alpha {
                on_full_grid(v, storage_bits, self.alpha_bits)
            } else if roles.narrow {
                on_narrow_grid(v, storage_bits, self.color_bits)
            } else {
                on_full_grid(v, storage_bits, self.color_bits)
            }
        }))
    }

    /// Whether the N-bit color codes survive a `bits`-deep encoding, recoverable
    /// by quantizing the decoded samples back to N bits. Weaker than exactness.
    pub const fn color_codes_recoverable_at(self, bits: u8) -> bool {
        bits >= self.color_bits
    }

    const fn storage_bits(self) -> u8 {
        match self.storage {
            ChannelType::U8 => 8,
            _ => 16,
        }
    }
}

/// Whether the `base`-bit grid in `storage_bits` storage is a subset of the
/// `depth`-bit grid. Widening to at least the storage depth always round-trips;
/// narrow (power-of-two) grids nest upward; full-range grids nest exactly when
/// `base` divides `depth`, because `2^base - 1` divides `2^depth - 1`.
const fn grid_nests(storage_bits: u8, base: u8, depth: u8, narrow: bool) -> bool {
    if depth == 0 {
        false
    } else if depth >= storage_bits {
        true
    } else if narrow {
        depth >= base
    } else {
        depth.is_multiple_of(base)
    }
}

/// Whether `v` lies on the full-scale `bits`-deep grid of `storage_bits`
/// storage: narrowing it with nearest rounding and widening back returns `v`.
const fn on_full_grid(v: u32, storage_bits: u8, bits: u8) -> bool {
    if bits >= storage_bits {
        return true;
    }
    let storage_max = (1u32 << storage_bits) - 1;
    let max = (1u32 << bits) - 1;
    let q = (v * max + storage_max / 2) / storage_max;
    (q * storage_max + max / 2) / max == v
}

/// Whether `v` lies on the narrow-range `bits`-deep grid of `storage_bits`
/// storage: narrow codes scale by powers of two, so the low bits are zero.
const fn on_narrow_grid(v: u32, storage_bits: u8, bits: u8) -> bool {
    bits >= storage_bits || v & ((1u32 << (storage_bits - bits)) - 1) == 0
}

/// Which samples of a view are color, which are alpha and which are padding.
struct Roles {
    storage_bits: u8,
    narrow: bool,
    channels: usize,
    /// Real alpha channel within a pixel.
    alpha: Option<usize>,
    /// Undefined RGBX/BGRX padding slot, never read.
    padding: Option<usize>,
}

impl Roles {
    fn of(descriptor: &ImageSpec) -> Result<Self, ExactDepthError> {
        if !descriptor.layout.packing.is_full() {
            return Err(ExactDepthError::UnsupportedLayout);
        }
        let storage_bits = match descriptor.layout.sample {
            ChannelType::U8 => 8,
            ChannelType::U16 => 16,
            _ => return Err(ExactDepthError::UnsupportedStorage),
        };
        let channels = descriptor.layout.channels();
        let last = match descriptor.layout.order {
            ChannelLayout::Gray | ChannelLayout::Rgb => None,
            ChannelLayout::GrayAlpha
            | ChannelLayout::Rgba
            | ChannelLayout::Bgra
            | ChannelLayout::Rgbx
            | ChannelLayout::Bgrx => Some(channels - 1),
            _ => return Err(ExactDepthError::UnsupportedLayout),
        };
        let (alpha, padding) = match last {
            Some(index) if descriptor.layout.order.has_alpha_slot() => (Some(index), None),
            Some(index) => (None, Some(index)),
            None => (None, None),
        };
        Ok(Self {
            storage_bits,
            narrow: matches!(signal_range(descriptor), SignalRange::Narrow),
            channels,
            alpha,
            padding,
        })
    }

    /// Visit active samples in order as `(is_alpha, value)` until `visit`
    /// returns false; returns whether every visited sample was accepted.
    fn for_each(&self, pixels: &PixelView<'_>, mut visit: impl FnMut(bool, u32) -> bool) -> bool {
        let samples = pixels.width() as usize * self.channels;
        for y in 0..pixels.rows() {
            let row = pixels.row(y);
            for i in 0..samples {
                let channel = i % self.channels;
                if Some(channel) == self.padding {
                    continue;
                }
                let v = if self.storage_bits == 8 {
                    u32::from(row[i])
                } else {
                    u32::from(u16::from_ne_bytes([row[2 * i], row[2 * i + 1]]))
                };
                if !visit(Some(channel) == self.alpha, v) {
                    return false;
                }
            }
        }
        true
    }
}

/// Running intersection of the depths whose grid holds every pushed sample.
struct GridScan {
    storage_bits: u8,
    narrow: bool,
    /// Full range: bit `d - 1` set while every sample lies on the d-bit grid.
    candidates: u32,
    /// Narrow range: OR of all samples (trailing zeros give the depth).
    or: u32,
}

impl GridScan {
    fn new(storage_bits: u8, narrow: bool) -> Self {
        Self {
            storage_bits,
            narrow,
            candidates: (1u32 << (storage_bits - 1)) - 1,
            or: 0,
        }
    }

    fn push(&mut self, v: u32) {
        if self.narrow {
            self.or |= v;
            return;
        }
        let mut remaining = self.candidates;
        while remaining != 0 {
            let d = remaining.trailing_zeros() as u8 + 1;
            remaining &= remaining - 1;
            if !on_full_grid(v, self.storage_bits, d) {
                self.candidates &= !(1u32 << (d - 1));
            }
        }
    }

    fn settled(&self) -> bool {
        if self.narrow {
            // Once the lowest set bit reaches the storage LSB nothing coarser remains.
            self.or & 1 != 0
        } else {
            self.candidates == 0
        }
    }

    fn bits(&self) -> u8 {
        if self.narrow {
            let zeros = if self.or == 0 {
                16
            } else {
                self.or.trailing_zeros() as u8
            };
            // Narrow U16 codes need at least eight bits; U8 storage is only eight.
            self.storage_bits
                .saturating_sub(zeros)
                .clamp(8, self.storage_bits)
        } else if self.candidates == 0 {
            self.storage_bits
        } else {
            self.candidates.trailing_zeros() as u8 + 1
        }
    }
}

impl ExactDepth {
    /// Read a common-grid claim carried by zenpixels, if it describes this view.
    /// The caller must still verify the actual samples before relying on it.
    pub fn from_view(view: &PixelView<'_>) -> Option<Self> {
        let spec = view.spec();
        if signal_range(spec) != SignalRange::Full {
            return None;
        }
        let bits = view.facts().exact.effective_bits;
        let alpha = if spec.layout.order.has_alpha_slot() {
            bits
        } else {
            spec.layout.sample.bits()
        };
        let claim = Self::new(spec.layout.sample, signal_range(spec), bits, alpha).ok()?;
        claim.validate_spec(spec).ok()?;
        Some(claim)
    }

    /// Publish the shared full-range grid of an already verified claim.
    ///
    /// The declaration retains separate color and alpha depths. The zenpixels
    /// view has a single depth, so use the smallest grid that contains both.
    pub fn attach_verified(self, view: PixelView<'_>) -> PixelView<'_> {
        if self.range != SignalRange::Full {
            return view;
        }
        let alpha = view.spec().layout.order.has_alpha_slot();
        let mut facts = view.facts();
        facts.exact.effective_bits = (self.color_bits..=self.storage_bits())
            .find(|&bits| self.color_exact_at(bits) && (!alpha || self.alpha_exact_at(bits)))
            .unwrap_or(self.storage_bits());
        view.with_facts(facts)
    }
}
