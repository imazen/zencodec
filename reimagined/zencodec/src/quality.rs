//! Fidelity targets, and the difference between asking and insisting.

/// A request that is either a preference or a requirement.
///
/// A codec that cannot honor [`Require`](Demand::Require) refuses when
/// asked for an encoder. One that cannot honor [`Prefer`](Demand::Prefer) proceeds and
/// states what it chose in its plan. There is no third, silent outcome.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Demand<T> {
    /// The codec's default.
    #[default]
    Default,
    /// Use this if possible.
    Prefer(T),
    /// Use this or refuse.
    Require(T),
}

impl<T> Demand<T> {
    /// The requested value, if any.
    pub const fn get(&self) -> Option<&T> {
        match self {
            Self::Default => None,
            Self::Prefer(t) | Self::Require(t) => Some(t),
        }
    }

    /// Whether refusal is the only alternative.
    pub const fn is_required(&self) -> bool {
        matches!(self, Self::Require(_))
    }

    /// Settle the request against what a codec can do.
    ///
    /// `achieve` maps a requested value to what the codec would actually do,
    /// or `None` when it cannot honor it. Returns the value to use, or the
    /// unmet requirement.
    pub fn resolve(self, default: T, achieve: impl FnOnce(&T) -> Option<T>) -> Result<T, T> {
        match self {
            Self::Default => Ok(default),
            Self::Prefer(want) => Ok(achieve(&want).unwrap_or(default)),
            Self::Require(want) => achieve(&want).ok_or(want),
        }
    }
}

/// How closely the encoded image must match its source.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum Fidelity {
    /// Bit-exact samples.
    Lossless,
    /// A lossy target.
    Lossy(LossyTarget),
}

/// A lossy target. One-shot: the codec maps it to its own dial without
/// re-encoding.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum LossyTarget {
    /// An approximate SSIMULACRA2 score.
    ApproxSsim2(f32),
    /// An approximate butteraugli distance.
    ApproxButteraugli(f32),
    /// An approximate zensim A score.
    ApproxZensimA(f32),
    /// The codec's own quality dial.
    CodecSpecificQuality(f32),
}

impl Fidelity {
    /// Whether this is [`Lossless`](Self::Lossless).
    pub const fn is_lossless(&self) -> bool {
        matches!(self, Self::Lossless)
    }
}

/// How hard to work for the same fidelity, on the codec's declared scale.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Effort(pub i32);
