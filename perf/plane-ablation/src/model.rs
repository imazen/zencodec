//! Private representation experiment, not a proposed public API.
use zenpixels::{ChannelType, sample::SampleEncoding};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Order {
    Native,
    Little,
    Big,
}
impl Order {
    pub(crate) fn native(self) -> bool {
        self == Self::Native
            || (self == Self::Little && cfg!(target_endian = "little"))
            || (self == Self::Big && cfg!(target_endian = "big"))
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Error {
    Geometry,
    Storage,
    Layout,
    Alignment,
    Endian,
    Bounds,
}

/// Storage only: no color, role, grid, normalization, historical precision or ownership.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Plane<'a> {
    bytes: &'a [u8],
    channel: ChannelType,
    order: Order,
    width: usize,
    height: usize,
    stride: usize,
    step: usize,
}
impl<'a> Plane<'a> {
    pub(crate) fn new(
        bytes: &'a [u8],
        channel: ChannelType,
        order: Order,
        width: usize,
        height: usize,
        stride: usize,
        step: usize,
    ) -> Result<Self, Error> {
        let size = channel.byte_size();
        if size == 0 || step < size {
            return Err(Error::Layout);
        }
        let end = if width == 0 || height == 0 {
            0
        } else {
            let span = (width - 1)
                .checked_mul(step)
                .and_then(|v| v.checked_add(size))
                .ok_or(Error::Geometry)?;
            if stride < span {
                return Err(Error::Layout);
            }
            (height - 1)
                .checked_mul(stride)
                .and_then(|v| v.checked_add(span))
                .ok_or(Error::Geometry)?
        };
        let bytes = bytes.get(..end).ok_or(Error::Storage)?;
        Ok(Self {
            bytes,
            channel,
            order,
            width,
            height,
            stride,
            step,
        })
    }
    pub(crate) fn row(self, y: usize) -> Result<Row<'a>, Error> {
        if y >= self.height {
            return Err(Error::Bounds);
        }
        let bytes = if self.width == 0 {
            &self.bytes[..0]
        } else {
            let start = y * self.stride;
            &self.bytes[start..start + (self.width - 1) * self.step + self.channel.byte_size()]
        };
        Ok(Row {
            bytes,
            channel: self.channel,
            order: self.order,
            width: self.width,
            step: self.step,
        })
    }
    pub(crate) fn codes(self, encoding: SampleEncoding) -> Result<Codes<'a>, Error> {
        if encoding.storage() != self.channel {
            return Err(Error::Storage);
        }
        Ok(Codes {
            plane: self,
            encoding,
        })
    }
}
#[derive(Clone, Copy, Debug)]
pub(crate) struct Row<'a> {
    bytes: &'a [u8],
    channel: ChannelType,
    order: Order,
    width: usize,
    step: usize,
}
impl<'a> Row<'a> {
    pub(crate) fn sample(self, x: usize) -> Result<&'a [u8], Error> {
        if x >= self.width {
            return Err(Error::Bounds);
        }
        Ok(&self.bytes[x * self.step..x * self.step + self.channel.byte_size()])
    }
    pub(crate) fn typed<T: Scalar>(self) -> Result<&'a [T], Error> {
        if self.channel != T::CHANNEL {
            return Err(Error::Storage);
        }
        if self.step != self.channel.byte_size() {
            return Err(Error::Layout);
        }
        if !self.order.native() && self.channel.byte_size() != 1 {
            return Err(Error::Endian);
        }
        if self.bytes.is_empty() {
            return Ok(&[]);
        }
        bytemuck::try_cast_slice(self.bytes).map_err(|_| Error::Alignment)
    }
}
// Private/sealed candidate: no caller-defined casts or layout promises.
pub(crate) trait Scalar: bytemuck::Pod {
    const CHANNEL: ChannelType;
}
macro_rules! scalars { ($($t:ty => $v:ident),*) => { $(impl Scalar for $t { const CHANNEL: ChannelType = ChannelType::$v; })* }; }
scalars!(u8=>U8,u16=>U16,f32=>F32,f64=>F64,i8=>I8,i16=>I16,u32=>U32,i32=>I32,u64=>U64,i64=>I64);

pub(crate) struct Codes<'a> {
    plane: Plane<'a>,
    encoding: SampleEncoding,
}
impl Codes<'_> {
    pub(crate) fn code(&self, x: usize, y: usize) -> Result<u16, Error> {
        let bytes = self.plane.row(y)?.sample(x)?;
        let word = match self.encoding.storage() {
            ChannelType::U8 => u16::from(bytes[0]),
            ChannelType::U16 => {
                let bytes = [bytes[0], bytes[1]];
                match self.plane.order {
                    Order::Native => u16::from_ne_bytes(bytes),
                    Order::Little => u16::from_le_bytes(bytes),
                    Order::Big => u16::from_be_bytes(bytes),
                }
            }
            _ => return Err(Error::Storage),
        };
        Ok(
            (word >> self.encoding.bit_shift())
                & (((1u32 << self.encoding.code_bits()) - 1) as u16),
        )
    }
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
