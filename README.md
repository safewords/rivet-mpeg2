# rivet-mpeg2

[![CI](https://github.com/rivet-transcoder/rivet-mpeg2/actions/workflows/ci.yml/badge.svg)](https://github.com/rivet-transcoder/rivet-mpeg2/actions/workflows/ci.yml)

An **MPEG-2 Video** (ITU-T H.262 | ISO/IEC 13818-2) decoder and encoder in
Rust: no C, no system libraries, no build script, nothing to install on a
build host. Written from the Recommendation, not translated from any other
implementation. The decoder decodes every main-profile and 4:2:2-profile
conformance bitstream of ISO/IEC 13818-4, and reproduces sample for sample
the reconstructions the suite publishes (the figures are
[below](#how-it-is-checked)). MPEG-1 video (ISO/IEC 11172-2) decodes too.

Written for the **[rivet](https://github.com/rivet-transcoder/rivet)**
transcoder, to be its MPEG-2 codec on both sides: the decoder for DVD,
broadcast and archive sources on machines without a GPU that takes them,
and the encoder for MPEG-2 output. Usable on its own by anything that has an
elementary stream and wants planar frames back, or frames and wants a
stream.

Published as `rivet-mpeg2`; **imported as `mpeg2`** (`use mpeg2::…`). One
dependency (`thiserror`), no features, no build script.

```toml
[dependencies]
mpeg2 = { package = "rivet-mpeg2", git = "https://github.com/rivet-transcoder/rivet-mpeg2", branch = "develop" }
```

## What it decodes

| | supported | refused with `Error::Unsupported` |
|---|---|---|
| **Streams** | MPEG-2 video elementary streams (Main and 4:2:2 profiles, any level, sizes to 4096 × 4096 samples), in any chunking; sequence changes mid-stream (size, chroma format, matrices); MPEG-1 video (ISO/IEC 11172-2) | the scalable extensions (SNR, spatial, temporal, data partitioning); pictures larger than 4096 × 4096 samples |
| **Pictures** | I, P and B; frame pictures and field pictures (either field first, I/P and P/P field pairs); MPEG-1 D-pictures | — |
| **Prediction** | frame, field (in frame and field pictures), dual-prime (both), 16×8; half-sample interpolation; skipped macroblocks; concealment motion vectors parsed (and kept as predictors); MPEG-1 full-sample vectors | — |
| **Residual** | both DCT coefficient tables, zigzag and alternate scan, frame and field DCT, linear and non-linear quantiser scale, downloaded matrices (sequence header and quant matrix extension, luma and chroma), intra DC precision 8–11 bits, mismatch control; MPEG-1 escapes and odd reconstruction | — |
| **Chroma** | 4:2:0, 4:2:2 | 4:4:4 |
| **Skipped** | user data, sequence error codes, copyright, picture display (pan-scan) and camera extensions, composite display fields | — |

Output is a [`Frame`](src/frame.rs) per frame, in display order: one
buffer with the Y, Cb and Cr planes packed one after the other, 8 bits per
sample, cropped to `horizontal_size` × `vertical_size` — the shape
`rivet-h26x` hands back. A frame coded as two field pictures comes back as
one frame. Each carries the picture's `picture_type`,
`temporal_reference`, `progressive_frame`, `top_field_first` and
`repeat_first_field`; 3:2 pulldown is reported, not applied, so a consumer
that wants field-accurate timing applies it. `Decoder::sequence()` gives the
size, chroma format, frame rate (with frame_rate_extension), aspect ratio
code, bit rate, profile and level, and the sequence display extension's
colour description.

A damaged slice is an error from `decode`, and the macroblocks it did not
reach keep what the picture buffer held; the frames before and after it are
not lost, and decoding resumes at the next start code. There is no error
concealment beyond that.

## What it encodes

Main Profile 4:2:0 at any size up to 4095 × 2800, progressive frame
pictures: I, P and B (`b_frames` between references, 0–7) in GOPs of
`gop_size` frames, open after the first. One slice per macroblock row; a
coarse-grid, diamond and half-sample motion search (±`search_range`
samples, which sets f_code); intra / forward / backward / bidirectional /
skipped decisions by SAD; a dead-zone quantiser for non-intra blocks; the
default quantiser matrices. The quantiser is constant
(`RateControl::ConstantQuantiser`) or set per picture from a target bit rate
(`RateControl::Bitrate`, the Test Model 5 bit allocation, without a VBV
model: the stream signals variable bit rate). `intra_vlc_format`,
`alternate_scan`, `q_scale_type` and `intra_dc_precision` are settable.

Not implemented in the encoder: interlaced coding (field pictures, field
prediction, dual-prime, field DCT), 4:2:2, custom matrices, per-macroblock
quantiser adaptation, a VBV model, scene-change detection.

## How it is checked

No test runs another implementation, and no other implementation's output
is used.

- **The ISO/IEC 13818-4 conformance bitstreams** (`tests/conformance.rs`).
  ISO publishes them among its Publicly Available Standards;
  `tools/fetch-conformance.sh` downloads them (they are not committed here)
  and CI runs the test. All **57** main-profile and 4:2:2-profile streams —
  1972 frames: frame and field pictures, dual-prime in both, 16×8, field
  B-pictures, concealment vectors, 3:2 pulldown flags, bottom field first,
  every DC precision, downloaded matrices, maximum-range vectors, heavy
  stuffing, sequence changes, 4:2:2 at 50 Mbit/s, nine MPEG-1 streams
  including D-pictures — decode **without an error** to the frame count the
  stream codes. The slice parser is strict: a slice must end exactly where
  the next start code begins, so a table or syntax slip anywhere shows.
- **Reconstructed samples.** Two streams' traces carry the reconstruction
  of their first macroblock row: **tcela-17** (an I-picture and a P-picture
  whose vectors reach the limit of the range: 90 macroblocks, 23 040 luma
  samples) and **nokia6_dual** (four frame pictures, every P macroblock
  dual-prime: 180 macroblocks, 46 080 samples). Both match **exactly**.
  chroma_dct_type-1 ships the reconstruction of a decoder that wrongly
  field-organises 4:2:0 chroma; the luma matches it and the chroma does not.
  The last frame of 32 of the streams was inspected by eye (clean after
  every prediction chain), and tcela-17's README describes its two
  pictures, which match.
- **The IDCT** against IEEE Std 1180-1990, which H.262 Annex A requires —
  10 000 blocks for each range (−256…255, −5…5, −300…300) and sign, against
  a direct evaluation of Annex A's definition — and Annex A's items 3 and 4.
  The decoder's IDCT is the separable real-number IDCT in double precision
  with the basis as literal constants (the same output on every platform);
  every statistic is **zero**: peak error 0, mean square error 0.
- **Tables.** Every VLC table is checked as a prefix code whose Kraft sum
  leaves exactly the codewords the Recommendation leaves unused; every
  run/level pair round-trips through Tables B.14 and B.15; the motion_code
  table is checked against the address-increment table it shares codewords
  with; the scans are checked as permutations, the zigzag against its
  definition.
- **Round trips** through the encoder (`tests/round_trip.rs`): every frame
  back, in display order, at the quality its quantiser implies, for every
  combination of the settable coding tools, odd sizes, and the rate
  control.
- **Malformed input** (`tests/fuzz.rs`, proptest): arbitrary bytes, and
  valid streams with bits flipped, bytes cut and garbage spliced, in
  arbitrary chunkings — errors, never a panic; run in debug in CI so
  overflow traps.

The encoder on natural pictures: the first 30 frames of tcela-7 (Mobile &
Calendar, 720 × 480, 29.97 Hz — a hard sequence), as decoded, re-encoded
and decoded again; luma PSNR against the re-encoder's input:

| setting | B-pictures | rate | luma PSNR | chroma PSNR |
|---|---|---|---|---|
| quantiser 3 | 2 | 19.5 Mb/s | 42.2 dB | 43.5 dB |
| quantiser 6 | 2 | 10.8 Mb/s | 36.6 dB | 38.8 dB |
| quantiser 12 | 2 | 5.6 Mb/s | 31.4 dB | 35.1 dB |
| target 8 Mb/s | 2 | 8.9 Mb/s | 35.1 dB | 38.0 dB |
| target 4 Mb/s | 2 | 4.6 Mb/s | 30.6 dB | 35.1 dB |
| target 4 Mb/s | 0 | 4.8 Mb/s | 30.0 dB | 34.3 dB |

The rate control overshoots its target by 10–20 % over these 30 frames.
On one core of the machine these were measured on, the decoder runs at
about 400 frames/s at 704 × 480 (Tek-5-long, 150 frames) and the encoder
at 45–65 frames/s at 720 × 480. Measured 2026-10-02.

## Provenance and licensing

Written from ITU-T Rec. H.262 (02/2000) — the text ITU publishes free —
and, for the few MPEG-1 points H.262 only summarises in Annex D.9 (escape
coding, odd reconstruction, D-picture macroblocks), knowledge of ISO/IEC
11172-2. **No implementation's source was read** — not FFmpeg's
libavcodec, libmpeg2, the MPEG Software Simulation Group's reference code
or any other — and no implementation was run to make or check test data.
The normative tables (the VLCs of Annex B, the scans, the default
matrices, Table 7-6) are transcribed from the Recommendation. The
conformance bitstreams and traces are ISO's, used as data.

**Patents.** MPEG-2 video has been subject to patent licensing (a pool now
administered by Via LA). Nothing here is a licence to any patent, and the
authors make no claim about whether anyone needs one.

## Using it

```rust
// Decoding: elementary-stream bytes in any chunking, frames in display order.
let mut dec = mpeg2::Decoder::new();
for chunk in stream.chunks(65536) {
    for frame in dec.decode(chunk)? {
        // frame.plane(0) (Y), plane(1) (Cb), plane(2) (Cr); frame.chroma;
        // frame.top_field_first, frame.repeat_first_field, …
    }
}
let last = dec.flush()?;
let info = dec.sequence(); // size, frame rate, aspect ratio, …

// Encoding: 4:2:0 frames in display order, stream bytes back.
let mut enc = mpeg2::Encoder::new(mpeg2::EncoderConfig {
    frame_rate: (25, 1),
    rate_control: mpeg2::RateControl::Bitrate(6_000_000),
    ..mpeg2::EncoderConfig::new(720, 576)
})?;
let mut out = Vec::new();
for f in &frames {
    out.extend(enc.encode(f)?);
}
out.extend(enc.finish()?);
```

`examples/m2v_decode.rs` decodes a file and writes raw planar YUV.

## License

Open Encoding Attribution License v1.0 — a source-available (not OSI open-source)
license, royalty-free, with a commercial-attribution requirement. See
[LICENSE.md](LICENSE.md) and [NOTICE](NOTICE).
