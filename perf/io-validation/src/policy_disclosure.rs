//! Private disclosure qualification, deliberately scoped to one XMP field.
//! Uses zenmeta's bounded XML/RDF reader, not an XML parser invented for a test.
//! No claim about image samples, arbitrary ICC, all XMP semantics or whole files.
#![allow(dead_code)]
use crate::policy::*;
use zencodec::{At, enough::Stop};

const TIFF: &str = "http://ns.adobe.com/tiff/1.0/";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Privacy {
    PreserveUninspected,
    /// Retain only the canonical, numeric TIFF orientation field. This is a
    /// named metadata allowlist, not an anonymity promise about pixels or text.
    OrientationOnlyV1,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Obligation {
    OptionalRemovable,
    Orientation,
    ExactBytes,
    UnknownCritical,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TranscodeRoute {
    Copy,
    Rewrite,
    Coefficients,
    Pixels,
    Media,
}

/// Inspection owns its regenerated packet and borrows the exact original.
/// A successful decision cannot be detached and applied to different bytes.
pub(crate) struct Prepared<'a> {
    original: &'a [u8],
    canonical: Option<Vec<u8>>,
    inspection: Inspection,
    carrier: Carrier,
    obligation: Obligation,
    privacy: Privacy,
    route: TranscodeRoute,
}

pub(crate) struct Approved<'a> {
    content: Content<'a>,
    pub action: BoundaryAction,
    pub inspection: Inspection,
    pub privacy: Privacy,
    pub route: TranscodeRoute,
    pub record: Record,
}
enum Content<'a> {
    Exact(&'a [u8]),
    Fresh(Vec<u8>),
    Omitted,
}
impl Approved<'_> {
    pub fn bytes(&self) -> &[u8] {
        match &self.content {
            Content::Exact(bytes) => bytes,
            Content::Fresh(bytes) => bytes,
            Content::Omitted => &[],
        }
    }
}

pub(crate) fn orientation_packet(orientation: u8) -> Vec<u8> {
    // The value is checked by inspection before this serialization is used.
    format!(
        "<rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\"><rdf:Description rdf:about=\"\" xmlns:tiff=\"{TIFF}\" tiff:Orientation=\"{orientation}\"/></rdf:RDF>"
    )
    .into_bytes()
}

impl<'a> Prepared<'a> {
    pub fn opaque(
        bytes: &'a [u8],
        carrier: Carrier,
        obligation: Obligation,
        privacy: Privacy,
        route: TranscodeRoute,
    ) -> Self {
        Self {
            original: bytes,
            canonical: None,
            inspection: Inspection::Unavailable,
            carrier,
            obligation,
            privacy,
            route,
        }
    }

    pub fn xmp(
        bytes: &'a [u8],
        service_available: bool,
        byte_limit: usize,
        obligation: Obligation,
        privacy: Privacy,
        route: TranscodeRoute,
    ) -> Self {
        let mut prepared = Self::opaque(bytes, Carrier::Xmp, obligation, privacy, route);
        if !service_available {
            return prepared;
        }
        if bytes.len() > byte_limit {
            prepared.inspection = Inspection::Limit;
            return prepared;
        }
        let Ok(xml) = core::str::from_utf8(bytes) else {
            prepared.inspection = Inspection::Malformed;
            return prepared;
        };
        let packet = match zenmeta::xmp::Packet::parse(xml) {
            Ok(packet) => packet,
            Err(zenmeta::xmp::Error::Limit) => {
                prepared.inspection = Inspection::Limit;
                return prepared;
            }
            Err(_) => {
                prepared.inspection = Inspection::Malformed;
                return prepared;
            }
        };
        prepared.inspection = Inspection::SyntaxOnly;
        // Syntax success, unknown namespaces, duplicate/unsupported RDF shapes
        // and free text are NOT evidence of compliance with the allowlist.
        let Ok(Some(values)) = packet.property(TIFF, "Orientation") else {
            return prepared;
        };
        if values.len() != 1 {
            return prepared;
        }
        let Some(value) = values[0].parse::<u8>().ok().filter(|v| (1..=8).contains(v)) else {
            return prepared;
        };
        let canonical = orientation_packet(value);
        prepared.inspection = if canonical == bytes {
            Inspection::Canonical
        } else {
            Inspection::SupportedSubset
        };
        prepared.canonical = Some(canonical);
        prepared
    }

    pub fn inspection(&self) -> Inspection {
        self.inspection
    }

    pub fn resolve<R: PolicyResolver + ?Sized>(
        self,
        engine: &mut Engine,
        context: Context,
        resolver: &mut R,
        stop: &dyn zencodec::Pulse,
    ) -> Result<Approved<'a>, At<Failure<R::Error>>> {
        let strict = self.privacy == Privacy::OrientationOnlyV1;
        // Byte, parser-node and depth limits are not a proven whole-operation
        // allocation bound. This experiment offers no hard native RAM limit.
        let offer = |action| Alternative {
            action: Action::Boundary(action),
            ram: Evidence::Unknown,
            disk: Evidence::Upper(0),
            accounting: Accounting::Driver,
            effects: Effects(0),
            needs_replay: false,
            preserves_published: true,
        };
        let mut entries = [offer(BoundaryAction::Drop); 3];
        let mut count = 0;
        let mut add = |action| {
            entries[count] = offer(action);
            count += 1;
        };
        if (!strict || self.inspection == Inspection::Canonical)
            && (self.obligation != Obligation::Orientation || self.canonical.is_some())
        {
            add(BoundaryAction::Keep);
        }
        if self.canonical.is_some()
            && matches!(
                self.obligation,
                Obligation::OptionalRemovable | Obligation::Orientation
            )
        {
            add(BoundaryAction::Regenerate);
        }
        if self.obligation == Obligation::OptionalRemovable {
            add(BoundaryAction::Drop);
        }
        let selected = engine.resolve(
            Facts::Boundary(BoundaryFacts {
                trigger: BoundaryTrigger::Disclosure {
                    carrier: self.carrier,
                    inspection: self.inspection,
                    bytes: self.original.len() as u64,
                    required: self.obligation != Obligation::OptionalRemovable,
                    strict,
                },
                commitments: Commitments {
                    body_bytes: context.published_bytes,
                    ..Commitments::default()
                },
            }),
            &entries[..count],
            Requirements {
                ram: None,
                disk: Some(0),
                accounting: Accounting::Driver,
                effects: Effects(0),
            },
            context,
            resolver,
            stop,
        )?;
        let Action::Boundary(action) = selected.alternative.action else {
            unreachable!("engine validates the question domain")
        };
        let content = match action {
            BoundaryAction::Keep => Content::Exact(self.original),
            BoundaryAction::Regenerate => Content::Fresh(self.canonical.expect("qualified subset")),
            BoundaryAction::Drop => Content::Omitted,
            _ => unreachable!("unoffered action"),
        };
        Ok(Approved {
            content,
            action,
            inspection: self.inspection,
            privacy: self.privacy,
            route: self.route,
            record: engine
                .records()
                .last()
                .expect("successful decision recorded"),
        })
    }
}
