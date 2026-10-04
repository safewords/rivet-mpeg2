# rivet-mpeg2

[![CI](https://github.com/safewords/rivet-mpeg2/actions/workflows/ci.yml/badge.svg)](https://github.com/safewords/rivet-mpeg2/actions/workflows/ci.yml)

An **MPEG-2 Video** (ITU-T H.262 | ISO/IEC 13818-2) decoder and encoder in
Rust: no C, no system libraries, no build script, nothing to install on a
build host. Written from the Recommendation, not translated from any other
implementation. The decoder decodes every main-profile and 4:2:2-profile
conformance bitstream of ISO/IEC 13818-4, and reproduces sample for sample
the reconstructions the suite publishes (the figures are
[below](#how-it-is-checked)). MPEG-1 video (ISO/IEC 11172-2) decodes too.

Written for the **[rivet](https://github.com/safewords/rivet)**
transcoder, to be its MPEG-2 codec on both sides: the decoder for DVD,
broadcast and archive sources on machines without a GPU that takes them,
and the encoder for MPEG-2 output. Usable on its own by anything that has an
elementary stream and wants planar frames back, or frames and wants a
stream.

Published as `rivet-mpeg2`; **imported as `mpeg2`** (`use mpeg2::…`). One
dependency (`thiserror`), no features, no build script.

```toml
[dependencies]
mpeg2 = { package = "rivet-mpeg2", git = "https://github.com/safewords/rivet-mpeg2", branch = "develop" }
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
concealment beyond that. (A picture's slices are decoded when the picture
ends — at the next unit that is not a slice — so a damaged slice's error
comes back with that unit.)

The slices of a picture are decoded on several threads at once
(`Decoder::set_threads`; by default one per core, fewer for small
pictures). The frames are the same, sample for sample, on any number of
threads — even for damaged streams whose slices overlap or arrive out of
order — and so are the errors; see [Speed](#speed).

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
The rows of a picture are coded on several threads at once
(`EncoderConfig::threads`, by default one per core), each a little behind
the row above, whose vectors its search starts from; the stream is the
same, bit for bit, on any number of threads.

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
  every statistic is **zero**: peak error 0, mean square error 0. The SIMD
  versions perform the same double-precision operations in the same order
  (no fused multiply-add), several columns at a time, so they return the
  very same integers: the IEEE 1180 run is repeated on every SIMD rung the
  machine has, and each rung's IDCT and FDCT are compared with the original
  scalar code on 30 000 blocks and every DC-only block.
- **Recorded output.** Every frame of every conformance stream is hashed
  and compared with the output of the original scalar, single-threaded
  decoder — decoded on 1, 2, 3 and 8 threads, with the SIMD kernels and
  with `MPEG2_FORCE_SCALAR=1` (CI on x86-64; aarch64 by hand, see "NEON on
  ARM hardware"). The encoder's stream
  for seven configurations (B-pictures, every coding tool, odd sizes, the
  extreme quantisers, the rate control) is compared, bit for bit, with the
  original encoder's on 1, 3 and 8 threads, and the decoder must reproduce
  every frame the encoder reconstructed — B-pictures included — sample for
  sample (`tests/encoder_exact.rs`).
- **The kernels** (`src/dsp/`): each SIMD rung against the scalar code on
  random and extreme inputs — motion compensation in every half-sample
  mode, averaging, residual addition, saturation, SAD.
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
  overflow traps. The same damaged streams decode to the same frames and
  the same errors on 1, 2, 3 and 8 threads.

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
Measured 2026-10-02.

### NEON on ARM hardware

CI runs on x86-64 Linux only, so the NEON (aarch64) code paths are not tested
there. They are verified by hand on ARM hardware (an aarch64 Linux machine,
or Apple silicon) after a change to them and before a release:

```sh
MPEG2_REQUIRE_SIMD=1 cargo test --release
MPEG2_FORCE_SCALAR=1 cargo test --release
```

The first run compares the NEON kernels with the scalar code (and fails if
NEON is not selected); the second runs the whole suite on the scalar code.
The conformance streams can be run the same way, as the `conformance` job in
`.github/workflows/ci.yml` does.

## Speed

The sample processing runs on SIMD kernels chosen at run time
(`src/dsp/`): SSE2 (every x86-64 processor), AVX2 and AVX-512F for the
transforms, NEON on AArch64; `mpeg2::simd_rung()` names the one in use and
`MPEG2_FORCE_SCALAR=1` forces the scalar code. Every rung returns exactly
what the scalar code returns — the IDCT and FDCT too: they perform the
same double-precision operations in the same order, several columns at a
time (no fused multiply-add), skipping only rows of zeros, whose
contribution is exactly nothing. Output does not depend on the processor.
On top of that, a picture's slices decode on several threads and a
picture's rows encode on several (a wavefront), neither changing a sample
or a bit.

Ryzen 9 9950X (16 cores), Windows, measured 2026-10-04 alongside other
work on the machine, fastest of several interleaved runs. The clips are
this encoder's (tcela-7, Mobile & Calendar, scaled to the size; 60 frames
at quantiser_scale_code 4, two B-pictures between references: about 23
and 40 Mb/s); "before" is the commit before the SIMD kernels, single
threaded.

| frames/s | before | 1 thread | 1 thread, scalar | 8 threads | 16 threads | 32 threads |
|---|---|---|---|---|---|---|
| decode 1280 × 720 | 165 | 422 | 213 | 1669 | 2028 | 2146 |
| decode 1920 × 1080 | 75 | 188 | 95 | 817 | 1072 | 1122 |
| decode 704 × 480, Tek-5-long (field pictures) | 441 | 859 | — | — | — | — |
| encode 1280 × 720 | 23.0 | 57.7 | 27.3 | 326 | 409 | 377 |
| encode 1920 × 1080 | 9.5 | 26.9 | 11.7 | 148 | 186 | 142 |

The threads are a pool started once per decoder or encoder. Decoding a
frame ends with copying it out into new memory, whose first writes the
operating system serves one page at a time however many threads write
them; one of the slice threads touches the next frame's memory while the
others decode, and the copy itself is shared out. The default for both is
a thread per core (the decoder's at most 32). Per kernel, nanoseconds per call
(`examples/mpeg2_kernels.rs`; "original" is the code each kernel replaced):

| kernel | original | scalar | SSE2 | AVX2 | AVX-512 |
|---|---|---|---|---|---|
| IDCT 8×8, few coefficients | 164 | 107 | 83 | 47 | 37 |
| IDCT 8×8, all 64 | 170 | 140 | 128 | 65 | 46 |
| FDCT 8×8 | 138 | 127 | 118 | 54 | 31 |
| MC 16×16, whole-sample | 66 | 67 | 7.6 | 7.6 | 7.5 |
| MC 16×16, half-sample both ways | 200 | 232 | 19.2 | 19.2 | 19.2 |
| MC 16×16, half-sample, averaged (B) | 225 | 204 | 10.0 | 10.0 | 10.1 |
| MC 8×8, half-sample both ways | 52 | 69 | 8.5 | 8.5 | 8.5 |
| residual add 8×8 | 3.5 | 4.1 | 2.7 | 2.7 | 2.7 |
| SAD 16×16 | 8.6 | 9.5 | 5.5 | 5.2 | 5.2 |

The encoder's motion search also stops a candidate's SAD after 8 rows
once they alone exceed the best so far, and its quantiser divides by
multiplying with an exact reciprocal. To reproduce: `tools/bench.sh`
(fetches the conformance suite if needed, makes the clips, runs both
examples).

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
