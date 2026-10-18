//! One cumulative allocation/work budget, including embedded-CBOR reparses.
use alloc::{string::String, vec::Vec};
use core::fmt::{self, Write};

pub(super) const LIMIT: usize = 16 * 1024 * 1024;
const MAX_ITEMS: usize = 1 << 20;

#[derive(Clone, Copy, Debug)]
pub(super) struct Limit;

pub(super) struct Budget {
    remaining: usize,
    items: usize,
}

impl Budget {
    pub(super) fn new() -> Self {
        Self {
            remaining: LIMIT,
            items: MAX_ITEMS,
        }
    }

    pub(super) fn charge(&mut self, bytes: usize) -> Result<(), Limit> {
        self.remaining = self.remaining.checked_sub(bytes).ok_or(Limit)?;
        Ok(())
    }

    pub(super) fn item(&mut self) -> Result<(), Limit> {
        self.items = self.items.checked_sub(1).ok_or(Limit)?;
        Ok(())
    }

    /// Charge requested capacity before allocation. Freed temporary storage does
    /// not refund the budget: repeated parsing/formatting must also be bounded.
    pub(super) fn reserve<T>(&mut self, out: &mut Vec<T>, extra: usize) -> Result<(), Limit> {
        let needed = out.len().checked_add(extra).ok_or(Limit)?;
        if needed > out.capacity() {
            let capacity = needed.max(out.capacity().saturating_mul(2)).max(4);
            self.charge(
                (capacity - out.capacity())
                    .checked_mul(core::mem::size_of::<T>())
                    .ok_or(Limit)?,
            )?;
            out.try_reserve_exact(capacity - out.len())
                .map_err(|_| Limit)?;
        }
        Ok(())
    }

    pub(super) fn push<T>(&mut self, out: &mut Vec<T>, value: T) -> Result<(), Limit> {
        self.reserve(out, 1)?;
        out.push(value);
        Ok(())
    }

    pub(super) fn copy(&mut self, bytes: &[u8]) -> Result<Vec<u8>, Limit> {
        let mut out = Vec::new();
        self.reserve(&mut out, bytes.len())?;
        out.extend_from_slice(bytes);
        Ok(out)
    }

    pub(super) fn text(&mut self, args: fmt::Arguments<'_>) -> Result<String, Limit> {
        let mut writer = Text {
            out: String::new(),
            budget: self,
        };
        writer.write_fmt(args).map_err(|_| Limit)?;
        Ok(writer.out)
    }
}

struct Text<'a> {
    out: String,
    budget: &'a mut Budget,
}

impl Write for Text<'_> {
    fn write_str(&mut self, value: &str) -> fmt::Result {
        let needed = self.out.len().checked_add(value.len()).ok_or(fmt::Error)?;
        if needed > self.out.capacity() {
            let capacity = needed.max(self.out.capacity().saturating_mul(2)).max(16);
            self.budget
                .charge(capacity - self.out.capacity())
                .map_err(|_| fmt::Error)?;
            self.out
                .try_reserve_exact(capacity - self.out.len())
                .map_err(|_| fmt::Error)?;
        }
        self.out.push_str(value);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formatting_is_admitted_before_expansion() {
        let mut budget = Budget::new();
        budget.remaining = 16;
        assert!(budget.text(format_args!("{:?}", "\0".repeat(100))).is_err());
    }

    #[test]
    fn released_temporaries_do_not_reset_the_budget() {
        let mut budget = Budget::new();
        budget.remaining = 16;
        drop(budget.copy(&[0; 16]).unwrap());
        assert!(budget.copy(&[0]).is_err());
    }
}
