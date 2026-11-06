#[cfg(test)]
mod tests {
    use zencodec::encode::EncoderConfig;
    #[test]
    fn png_shared_incremental_contract() {
        for image in [zencodec_testkit::TestImage::rgb8_gradient(97,65),zencodec_testkit::TestImage::rgba8_gradient(97,65)] {
            let bytes=zenpng::PngEncoderConfig::new().encode(image.as_slice()).unwrap();
            zencodec_testkit::check_incremental_contract(zenpng::PngDecoderConfig::new(),bytes.data()).unwrap();
        }
    }
}
