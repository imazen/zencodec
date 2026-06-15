use zencodec::Orientation;
use zencodec::exif::{Exif, TextEncoding};
fn take_string(data: &mut &[u8]) -> String {
    let Some((&len, rest)) = data.split_first() else {
        return String::new();
    };
    let len = (len as usize).min(rest.len());
    let (s, tail) = rest.split_at(len);
    *data = tail;
    String::from_utf8_lossy(s).into_owned()
}
fn main() {
    let bytes = std::fs::read(std::env::args().nth(1).unwrap()).unwrap();
    let mut data = &bytes[..];
    let (&cfg, rest) = data.split_first().unwrap();
    data = rest;
    let (&o, rest) = data.split_first().unwrap();
    data = rest;
    let copyright = take_string(&mut data);
    let artist = take_string(&mut data);
    let encoding = if cfg & 1 != 0 {
        TextEncoding::Utf8
    } else {
        TextEncoding::Ascii
    };
    let mut x = if cfg & 2 != 0 {
        match Exif::parse(data) {
            Some(x) => x,
            None => {
                println!("rest unparseable");
                return;
            }
        }
    } else {
        Exif::new(encoding)
    };
    println!("cfg={cfg:#x} o={o} from_parse={}", cfg & 2 != 0);
    let orient = Orientation::from_exif((o % 8) + 1).unwrap();
    if cfg & 4 != 0 {
        x.set_orientation(orient);
    }
    if cfg & 8 != 0 {
        x.set_copyright(&copyright);
    }
    if cfg & 0x10 != 0 {
        x.set_artist(&artist);
    }
    let out = x.to_bytes();
    let y = Exif::parse(&out).expect("must parse");
    let out2 = y.to_bytes();
    println!(
        "fixpoint={} (out {} bytes, out2 {} bytes)",
        out == out2,
        out.len(),
        out2.len()
    );
    if out != out2 {
        // find first differing offset
        let n = out.iter().zip(&out2).position(|(a, b)| a != b);
        println!("first diff at {:?}", n);
        // dump around the diff
        if let Some(p) = n {
            let s = p.saturating_sub(4);
            println!("out  @{s}: {:02x?}", &out[s..(s + 16).min(out.len())]);
            println!("out2 @{s}: {:02x?}", &out2[s..(s + 16).min(out2.len())]);
        }
    }
}
