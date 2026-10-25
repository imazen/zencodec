//! Line protocol used by the independent Python Fraction oracle.
use std::io::{self, BufRead, Write};
use zencodec::{Resolution, ResolutionUnit};

fn unit(value: &str) -> ResolutionUnit {
    match value {
        "I" => ResolutionUnit::Inch,
        "C" => ResolutionUnit::Centimeter,
        "M" => ResolutionUnit::Meter,
        "U" => ResolutionUnit::Unknown,
        _ => panic!("invalid oracle unit"),
    }
}
fn resolution(fields: &[&str]) -> Resolution {
    Resolution {
        x: f64::from_bits(u64::from_str_radix(fields[0], 16).unwrap()),
        y: f64::from_bits(u64::from_str_radix(fields[1], 16).unwrap()),
        unit: unit(fields[2]),
    }
}
fn main() -> io::Result<()> {
    let mut out = io::BufWriter::new(io::stdout().lock());
    for line in io::stdin().lock().lines() {
        let line = line?;
        let fields: Vec<_> = line.split_whitespace().collect();
        assert_eq!(fields.len(), 8, "invalid oracle request");
        let source = resolution(&fields[..3]);
        let other = resolution(&fields[3..6]);
        let result = source.try_integer_density(unit(fields[6]), fields[7].parse().unwrap());
        write!(out, "{} ", source.is_equivalent_to(&other))?;
        match result {
            Ok((x, y)) => writeln!(out, "{x} {y}")?,
            Err(error) => writeln!(out, "{error:?}")?,
        }
    }
    out.flush()
}
