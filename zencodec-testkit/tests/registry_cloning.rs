//! Registry snapshots share templates; execution still owns a fresh config.
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use zencodec::{
    CodecSet, ImageFormat, decode::DecoderConfig, encode::EncoderConfig, operation::DecodeRequest,
    storage::StorageBudget,
};
use zencodec_testkit::{ReferenceEncoderConfig, ReferenceZcrDecoderConfig};
use zenpixels::PixelDescriptor;

struct CountedDecoder(Arc<AtomicUsize>);
impl Clone for CountedDecoder {
    fn clone(&self) -> Self {
        self.0.fetch_add(1, Ordering::SeqCst);
        Self(self.0.clone())
    }
}
impl DecoderConfig for CountedDecoder {
    type Error = <ReferenceZcrDecoderConfig as DecoderConfig>::Error;
    type Job<'a> = <ReferenceZcrDecoderConfig as DecoderConfig>::Job<'a>;
    fn formats() -> &'static [ImageFormat] {
        ReferenceZcrDecoderConfig::formats()
    }
    fn supported_descriptors() -> &'static [PixelDescriptor] {
        ReferenceZcrDecoderConfig::supported_descriptors()
    }
    fn job<'a>(self) -> Self::Job<'a> {
        ReferenceZcrDecoderConfig.job()
    }
}
struct CountedEncoder(Arc<AtomicUsize>);
impl Clone for CountedEncoder {
    fn clone(&self) -> Self {
        self.0.fetch_add(1, Ordering::SeqCst);
        Self(self.0.clone())
    }
}
impl EncoderConfig for CountedEncoder {
    type Error = <ReferenceEncoderConfig as EncoderConfig>::Error;
    type Job = <ReferenceEncoderConfig as EncoderConfig>::Job;
    fn format() -> ImageFormat {
        ReferenceEncoderConfig::format()
    }
    fn supported_descriptors() -> &'static [PixelDescriptor] {
        ReferenceEncoderConfig::supported_descriptors()
    }
    fn job(self) -> Self::Job {
        ReferenceEncoderConfig::new().job()
    }
}

#[test]
fn registry_clone_and_controlled_start_do_not_clone_unselected_templates() {
    let decodes = Arc::new(AtomicUsize::new(0));
    let encodes = Arc::new(AtomicUsize::new(0));
    let mut base = CodecSet::new();
    for _ in 0..4 {
        base = base
            .with_decoder(CountedDecoder(decodes.clone()))
            .with_encoder(CountedEncoder(encodes.clone()))
            .with_send_encoder(CountedEncoder(encodes.clone()));
    }
    let copy = base.clone();
    assert_eq!(decodes.load(Ordering::SeqCst), 0);
    assert_eq!(encodes.load(Ordering::SeqCst), 0);
    // Unsupported controlled execution still selects only one native template.
    assert!(
        DecodeRequest::new()
            .with_format(CountedDecoder::formats()[0])
            .start_controlled(&copy, StorageBudget::new(4096, 4))
            .is_err()
    );
    assert_eq!(decodes.load(Ordering::SeqCst), 1);
    assert_eq!(encodes.load(Ordering::SeqCst), 0);
    // Each requested operation continues to receive a distinct owned config.
    drop(base.encode_job(ImageFormat::Pnm).unwrap());
    drop(copy.encode_job(ImageFormat::Pnm).unwrap());
    assert_eq!(encodes.load(Ordering::SeqCst), 2);
    assert_eq!(
        base.decodable_formats().count(),
        copy.decodable_formats().count()
    );
}
