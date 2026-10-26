//! Replace packed color without copying or detaching auxiliary mappings.
use super::*;

impl ImageOutput {
    /// Replace packed color while retaining the original auxiliary planes.
    ///
    /// The adapter must supply color for the same stored coordinates. This is
    /// for color/format conversion, not cropping, orientation, resampling or
    /// auxiliary rendering. Geometry, placement, timing, orientation, original
    /// auxiliary interpretation and opaque source facts remain unchanged. The
    /// new color's descriptor and context describe its actual output samples.
    /// Only `owner.pixels()` is used; auxiliary methods on `owner` are not adopted.
    ///
    /// Both owners keep their original storage charges. When auxiliaries exist,
    /// the original owner can also retain its old color allocation; this method
    /// cannot split a backend allocation. Further replacements release previous
    /// replacement colors instead of accumulating a chain. Without auxiliaries,
    /// the original storage is released unless another image retains it.
    ///
    /// Mapping guards keep the original auxiliary views alive and preserve typed
    /// native failures. Construction and later mapping check the frozen layout
    /// and color contracts. Failure drops this consumed image and the supplied
    /// owner; independent clones remain valid. This neither runs a transform nor
    /// grants publication/privacy permission, and it does not bound workspace.
    pub fn replace_packed_color<P: PackedImageOwner + 'static>(
        self,
        owner: P,
    ) -> Result<Self, At<OutputError>> {
        let facts = self.storage.facts();
        if facts.packed.is_none() {
            return Err(fail(OutputError::NotPackedColor));
        }
        let color = Self::from_packed_owner(owner, Vec::new(), (*self.interpretation).clone())?;
        let storage = if facts.auxiliaries == 0 {
            color.storage
        } else {
            let auxiliaries = self
                .storage
                .auxiliary_source()
                .unwrap_or_else(|| self.storage.clone());
            // Validate the currently held native mapping before publishing the
            // composition. The temporary guard never escapes construction.
            drop(auxiliaries.map()?);
            Arc::new(ColorReplacement {
                color: color.storage,
                auxiliaries,
            })
        };
        Ok(Self {
            storage,
            interpretation: self.interpretation,
        })
    }
}

struct ColorReplacement {
    color: Arc<dyn ImageStorage>,
    auxiliaries: Arc<dyn ImageStorage>,
}
impl ImageStorage for ColorReplacement {
    fn facts(&self) -> SampleFacts {
        let mut facts = self.auxiliaries.facts();
        facts.packed = self.color.facts().packed;
        facts
    }
    fn map(&self) -> Result<Box<dyn MappedImage + '_>, Failure> {
        Ok(Box::new(ReplacementMap {
            color: self.color.map()?,
            auxiliaries: self.auxiliaries.map()?,
        }))
    }
    fn auxiliary_source(&self) -> Option<Arc<dyn ImageStorage>> {
        Some(self.auxiliaries.clone())
    }
}
struct ReplacementMap<'a> {
    color: Box<dyn MappedImage + 'a>,
    auxiliaries: Box<dyn MappedImage + 'a>,
}
impl MappedImage for ReplacementMap<'_> {
    fn plane_count(&self) -> usize {
        self.auxiliaries.plane_count()
    }
    fn plane(&self, index: usize) -> Result<Option<Plane<'_>>, At<OutputError>> {
        if index == 0 {
            self.color.plane(0)
        } else {
            self.auxiliaries.plane(index)
        }
    }
}
