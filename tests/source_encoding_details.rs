//! Consumer witnesses for inherent methods on all supported trait-object forms.
//! rustdoc JSON currently omits these methods; the site supplements its catalog
//! from their declarations, and this test checks that callers can use them.

use zencodec::SourceEncodingDetails;

struct Probe {
    quality: f32,
    quantizers: [u8; 4],
}

impl SourceEncodingDetails for Probe {
    fn source_generic_quality(&self) -> Option<f32> {
        Some(self.quality)
    }
}

struct OtherProbe;

impl SourceEncodingDetails for OtherProbe {
    fn source_generic_quality(&self) -> Option<f32> {
        None
    }
}

#[test]
fn all_object_forms_borrow_the_original_codec_details() {
    let probe = Probe {
        quality: 81.0,
        quantizers: [3, 7, 11, 19],
    };
    let plain: &dyn SourceEncodingDetails = &probe;
    let send: &(dyn SourceEncodingDetails + Send) = &probe;
    let shared: &(dyn SourceEncodingDetails + Send + Sync) = &probe;

    let plain_details = plain.codec_details::<Probe>().unwrap();
    let send_details = send.codec_details::<Probe>().unwrap();
    let shared_details = shared.codec_details::<Probe>().unwrap();
    for details in [plain_details, send_details, shared_details] {
        assert!(core::ptr::eq(details, &probe));
        assert_eq!(details.quantizers, probe.quantizers);
        assert_eq!(details.source_generic_quality(), Some(probe.quality));
    }
}

#[test]
fn every_object_form_refuses_a_different_probe_type() {
    let probe = OtherProbe;
    let plain: &dyn SourceEncodingDetails = &probe;
    let send: &(dyn SourceEncodingDetails + Send) = &probe;
    let shared: &(dyn SourceEncodingDetails + Send + Sync) = &probe;

    assert!(plain.codec_details::<Probe>().is_none());
    assert!(send.codec_details::<Probe>().is_none());
    assert!(shared.codec_details::<Probe>().is_none());
}
