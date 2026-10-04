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

/// Decodes `data` in `chunk`-byte pieces on `threads` threads: every
/// call's frames or error.
fn outcomes(data: &[u8], chunk: usize, threads: usize) -> Vec<Result<Vec<mpeg2::Frame>, String>> {
    let mut dec = Decoder::new();
    dec.set_threads(threads);
    let mut out: Vec<_> = data.chunks(chunk.max(1)).map(|c| dec.decode(c).map_err(|e| e.to_string())).collect();
    out.push(dec.flush().map_err(|e| e.to_string()));
    out
}

/// Slice threads change nothing, even on damaged streams (slices running
/// into rows other threads own, slices out of order, errors mid-picture):
/// the same frames and the same errors from the same calls.
fn same_on_threads(data: &[u8], chunk: usize) {
    let one = outcomes(data, chunk, 1);
    for t in [2, 3, 8] {
        assert!(outcomes(data, chunk, t) == one, "{t} threads differ from one");
    }
}

/// 256 cases unless PROPTEST_CASES says otherwise (e.g. 20000 for a long
/// local run in release with overflow checks).
fn config() -> ProptestConfig {
    let cases = std::env::var("PROPTEST_CASES").ok().and_then(|v| v.parse().ok()).unwrap_or(256);
    ProptestConfig { cases, ..ProptestConfig::default() }
}

/// A start-code value biased toward the ones that change decoder state:
/// headers, extensions, the first and last slice codes.
fn start_code_value() -> impl Strategy<Value = u8> {
    prop_oneof![
        Just(0x00u8),
        Just(0xb3),
        Just(0xb5),
        Just(0xb7),
        Just(0xb8),
        Just(0x01),
        Just(0xaf),
        any::<u8>(),
    ]
}

proptest! {
    #![proptest_config(config())]

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
        same_on_threads(&data, chunk);
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
        same_on_threads(&data, chunk);
    }

    /// Whole start-code units (a header, an extension, a slice) of random
    /// content inserted anywhere in a valid stream, and units copied from
    /// elsewhere in it: out-of-place headers and extensions mid-picture.
    #[test]
    fn spliced_units(units in proptest::collection::vec(
                         (any::<usize>(), start_code_value(), proptest::collection::vec(any::<u8>(), 0..24)), 1..8),
                     copies in proptest::collection::vec((any::<usize>(), any::<usize>(), 1usize..40), 0..4),
                     chunk in 1usize..700) {
        let mut data = sample_stream().to_vec();
        for (at, code, body) in units {
            let at = at % (data.len() + 1);
            let mut u = vec![0, 0, 1, code];
            u.extend(body);
            data.splice(at..at, u);
        }
        for (from, to, len) in copies {
            let from = from % data.len();
            let piece = data[from..(from + len).min(data.len())].to_vec();
            let to = to % (data.len() + 1);
            data.splice(to..to, piece);
        }
        feed(&data, chunk);
        same_on_threads(&data, chunk);
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

/// A damaged slice in the middle of a stream fed in one call: the call
/// reports the error, and the frames before and after it still come out.
#[test]
fn an_error_mid_stream_loses_no_frames() {
    let mut data = sample_stream().to_vec();
    // Garbage in the first slice of the third picture.
    let pics: Vec<usize> = data.windows(4).enumerate().filter(|(_, w)| *w == [0, 0, 1, 0]).map(|(i, _)| i).collect();
    let slice = pics[2] + data[pics[2]..].windows(4).position(|w| w == [0, 0, 1, 1]).unwrap();
    for b in &mut data[slice + 5..slice + 12] {
        *b = 0xff;
    }
    let mut dec = Decoder::new();
    let first = dec.decode(&data);
    assert!(first.is_err(), "the damaged slice is reported");
    let mut n = 0;
    n += dec.decode(&[]).expect("the rest decodes").len();
    n += dec.flush().expect("flush").len();
    assert_eq!(n, 6);
}

/// Applies `cut_and_spliced`'s edit to the sample stream.
fn cut_and_splice(cuts: &[(usize, usize)], garbage: &[u8]) -> Vec<u8> {
    let mut data = sample_stream().to_vec();
    for &(a, b) in cuts {
        if data.len() < 2 {
            break;
        }
        let a = a % data.len();
        let b = (a + b % 300).min(data.len());
        data.drain(a..b);
    }
    let at = garbage.len() % data.len().max(1);
    data.splice(at..at, garbage.iter().copied());
    data
}

/// Regression: a proptest case from CI that panicked ("index out of bounds:
/// the len is 0 but the index is 2").
#[test]
fn regression_cut_and_spliced_empty_index() {
    let data = cut_and_splice(
        &[(6376128340405003133, 14027746147085139091)],
        &[58, 32, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
    );
    for chunk in [1, 2, 3, 7, 64, data.len()] {
        feed(&data, chunk);
    }
}

/// The root of the case above: a sequence extension that changes the picture
/// size arriving between a picture's slices (unlike a sequence header, an
/// extension does not end the picture). The open picture must end before
/// the buffers it was decoded into are reallocated.
#[test]
fn a_size_change_mid_picture_does_not_panic() {
    let s = sample_stream();
    let seq_ext = s.windows(5).position(|w| w[..4] == [0, 0, 1, 0xb5] && w[4] >> 4 == 1).unwrap();
    let mut ext = s[seq_ext..seq_ext + 10].to_vec();
    ext[5] |= 0x01; // horizontal_size_extension's high bit: 8192 wider
    let pics: Vec<usize> = s.windows(4).enumerate().filter(|(_, w)| *w == [0, 0, 1, 0]).map(|(i, _)| i).collect();
    for &p in &pics {
        let slice = p + s[p..].windows(4).position(|w| w == [0, 0, 1, 1]).unwrap();
        let after = slice + 4 + s[slice + 4..].windows(3).position(|w| w == [0, 0, 1]).unwrap();
        let mut data = s[..after].to_vec();
        data.extend(&ext);
        data.extend(&s[after..]);
        for chunk in [1, 13, data.len()] {
            feed(&data, chunk);
        }
    }
}

/// Every picture's slices reversed and its first slice repeated at the end:
/// slices out of row order, rows written twice (the later write wins).
/// Threads split such pictures into bands the slices do not respect; the
/// frames must still be those of one thread.
#[test]
fn slices_out_of_order_decode_the_same_on_threads() {
    let cfg = EncoderConfig { b_frames: 1, gop_size: 4, ..EncoderConfig::new(64, 96) };
    let frames: Vec<_> = (0..5).map(|t| synthetic(64, 96, t)).collect();
    let s = encode(cfg, &frames);
    let starts: Vec<usize> = s.windows(3).enumerate().filter(|(_, w)| *w == [0, 0, 1]).map(|(i, _)| i).collect();
    let units: Vec<&[u8]> =
        starts.iter().enumerate().map(|(k, &a)| &s[a..starts.get(k + 1).copied().unwrap_or(s.len())]).collect();
    let is_slice = |u: &[u8]| (1..=0xaf).contains(&u[3]);
    let mut data = Vec::new();
    let mut run: Vec<&[u8]> = Vec::new();
    for u in units.iter().copied().chain([&[0u8, 0, 1, 0xb7][..]]) {
        if is_slice(u) {
            run.push(u);
            continue;
        }
        if !run.is_empty() {
            let first = run[0];
            run.reverse();
            run.push(first);
            data.extend(run.drain(..).flatten());
        }
        data.extend_from_slice(u);
    }
    let one = outcomes(&data, data.len(), 1);
    assert!(one.iter().all(|o| o.is_ok()), "{one:?}");
    for t in [2, 3, 4, 6, 16] {
        assert!(outcomes(&data, data.len(), t) == one, "{t} threads");
    }
    // And the same frames as the stream in order: every row's last write is
    // its own slice's.
    let all = |o: Vec<Result<Vec<mpeg2::Frame>, String>>| o.into_iter().flat_map(Result::unwrap).collect::<Vec<_>>();
    let (ordered, one) = (all(outcomes(&s, s.len(), 1)), all(one));
    assert_eq!(ordered.len(), 5);
    assert!(ordered == one);
}
