// Supplemental debugging check; not part of the frozen learning score.
#[test]
fn the_concrete_source_cause_survives() {
    #[derive(Debug)]
    struct SourceFault;
    impl std::fmt::Display for SourceFault {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("source fault")
        }
    }
    impl std::error::Error for SourceFault {}
    struct Broken;
    impl ReadAt for Broken {
        type Error = SourceFault;
        fn len(&mut self) -> Result<u64, Self::Error> {
            Err(SourceFault)
        }
        fn read_exact_at(&mut self, _: u64, _: &mut [u8]) -> Result<(), Self::Error> {
            Err(SourceFault)
        }
    }
    let error = candidate::decode_positioned(&mut Broken).err().unwrap();
    let mut cause: &(dyn std::error::Error + 'static) = error.as_ref();
    while !cause.is::<SourceFault>() {
        cause = cause.source().expect("original concrete source cause lost");
    }
}
