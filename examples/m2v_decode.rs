//! Decodes an MPEG-1 / MPEG-2 video elementary stream and prints one line
//! per frame; with a second argument, writes the frames there as raw planar
//! YUV (Y, Cb, Cr per frame, the layout of the conformance suite's
//! `.decoded` files).
//!
//! `cargo run --release --example m2v_decode -- in.m2v [out.yuv]`

use std::io::Write;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let Some(input) = args.get(1) else {
        eprintln!("usage: m2v_decode <stream> [out.yuv]");
        std::process::exit(2);
    };
    let data = std::fs::read(input).expect("read input");
    let mut out = args
        .get(2)
        .map(|p| std::io::BufWriter::new(std::fs::File::create(p).expect("create output")));
    let mut dec = mpeg2::Decoder::new();
    let mut frames = Vec::new();
    let mut errors = 0;
    for chunk in data.chunks(65536) {
        match dec.decode(chunk) {
            Ok(f) => frames.extend(f),
            Err(e) => {
                errors += 1;
                eprintln!("error: {e}");
            }
        }
    }
    match dec.flush() {
        Ok(f) => frames.extend(f),
        Err(e) => {
            errors += 1;
            eprintln!("error: {e}");
        }
    }
    if let Some(s) = dec.sequence() {
        println!(
            "{}x{} {:?} rate {:?} mpeg1 {} progressive_sequence {} profile/level {:#04x}",
            s.width,
            s.height,
            s.chroma,
            s.frame_rate,
            s.mpeg1,
            s.progressive_sequence,
            s.profile_and_level_indication
        );
    }
    for (i, f) in frames.iter().enumerate() {
        println!(
            "frame {i}: {:?} tr {} decode {} fields {} prog {} tff {} rff {}",
            f.picture_type,
            f.temporal_reference,
            f.decode_index,
            f.field_pictures,
            f.progressive_frame,
            f.top_field_first,
            f.repeat_first_field
        );
        if let Some(o) = &mut out {
            o.write_all(f.packed()).expect("write");
        }
    }
    println!("{} frames, {} errors", frames.len(), errors);
}
