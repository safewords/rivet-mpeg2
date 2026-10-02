//! Malformed input: arbitrary bytes, and valid streams with bits flipped,
//! bytes cut and garbage spliced, fed to the decoder in arbitrary chunks.
//! Errors are fine; panics, hangs and absurd allocations are not. Run in
//! debug too, so arithmetic overflow traps.

mod common;

use common::*;
use mpeg2::{Decoder, EncoderConfig};
use proptest::prelude::*;
use std::sync::OnceLock;

/// A small valid stream with I, P and B pictures, made once.
fn sample_stream() -> &'static [u8] {
    static S: OnceLock<Vec<u8>> = OnceLock::new();
    S.get_or_init(|| {
        let cfg = EncoderConfig { b_frames: 2, gop_size: 4, ..EncoderConfig::new(48, 32) };
        let frames: Vec<_> = (0..6).map(|t| synthetic(48, 32, t)).collect();
        encode(cfg, &frames)
    })
}

/// Decodes `data` in `chunk`-byte pieces, ignoring errors; returns the
/// frames it produced.
fn feed(data: &[u8], chunk: usize) -> usize {
    let mut dec = Decoder::new();
    let mut n = 0;
    for c in data.chunks(chunk.max(1)) {
        if let Ok(f) = dec.decode(c) {
            n += f.len();
        }
    }
    if let Ok(f) = dec.flush() {
        n += f.len();
    }
    n
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 256, ..ProptestConfig::default() })]

    #[test]
    fn arbitrary_bytes(data in proptest::collection::vec(any::<u8>(), 0..4096), chunk in 1usize..600) {
        feed(&data, chunk);
    }

    /// Arbitrary bytes after a valid sequence header, so the slice layer is
    /// reached.
    #[test]
    fn arbitrary_pictures(tail in proptest::collection::vec(any::<u8>(), 0..4096), chunk in 1usize..600) {
        let s = sample_stream();
        let first_picture = s.windows(4).position(|w| w == [0, 0, 1, 0]).unwrap();
        let mut data = s[..first_picture + 4].to_vec();
        data.extend(&tail);
        feed(&data, chunk);
    }

    #[test]
    fn flipped_bits(flips in proptest::collection::vec((any::<usize>(), 0u8..8), 1..20), chunk in 1usize..700) {
        let mut data = sample_stream().to_vec();
        for (pos, bit) in flips {
            let i = pos % data.len();
            data[i] ^= 1 << bit;
        }
        feed(&data, chunk);
    }

    #[test]
    fn cut_and_spliced(cuts in proptest::collection::vec((any::<usize>(), any::<usize>()), 1..6),
                       garbage in proptest::collection::vec(any::<u8>(), 0..64),
                       chunk in 1usize..700) {
        let mut data = sample_stream().to_vec();
        for (a, b) in cuts {
            if data.len() < 2 { break; }
            let a = a % data.len();
            let b = (a + b % 300).min(data.len());
            data.drain(a..b);
        }
        let at = garbage.len() % data.len().max(1);
        data.splice(at..at, garbage);
        feed(&data, chunk);
    }
}

/// The valid stream itself still decodes whole through every chunking.
#[test]
fn any_chunking_gives_the_same_frames() {
    let s = sample_stream();
    let whole = decode(s, s.len());
    for chunk in [1, 2, 3, 5, 64, 1000] {
        let frames = decode(s, chunk);
        assert_eq!(frames, whole, "chunk {chunk}");
    }
}

/// A decoder recovers after garbage: the stream that follows decodes.
#[test]
fn recovers_after_garbage() {
    let s = sample_stream();
    let mut dec = Decoder::new();
    let _ = dec.decode(&[0, 0, 1, 0xb3, 0xff, 0xff, 0, 0, 1, 1, 0xde, 0xad]);
    let mut n = 0;
    match dec.decode(s) {
        Ok(f) => n += f.len(),
        Err(_) => n += dec.decode(&[]).map(|f| f.len()).unwrap_or(0),
    }
    n += dec.flush().unwrap().len();
    assert_eq!(n, 6);
}
