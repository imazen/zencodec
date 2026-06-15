use zencodec::exif::Exif;
fn main() {
    let path = std::env::args().nth(1).unwrap();
    let data = std::fs::read(&path).unwrap();
    let x = Exif::parse(&data).unwrap();
    let bytes = x.to_bytes();
    println!(
        "x.has_gps={} -> to_bytes {} bytes",
        x.has_gps(),
        bytes.len()
    );
    // hexdump
    for (i, c) in bytes.chunks(16).enumerate() {
        print!("{:04x}: ", i * 16);
        for b in c {
            print!("{:02x} ", b);
        }
        println!();
    }
    let y = Exif::parse(&bytes);
    println!(
        "reparse: {:?}",
        y.as_ref().map(|y| (y.has_gps(), y.has_thumbnail()))
    );
}
