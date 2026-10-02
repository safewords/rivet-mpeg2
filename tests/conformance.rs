//! The ISO/IEC 13818-4 video conformance bitstreams (main-profile and
//! 422-profile), fetched by `tools/fetch-conformance.sh` into
//! `MPEG2_CONFORMANCE_DIR`. Without that variable the test reports that it
//! did not run and passes; with `MPEG2_REQUIRE_CONFORMANCE=1` (CI) it fails
//! instead.
//!
//! Every stream must decode without an error — the slice parser is strict:
//! a slice must end exactly where the next start code begins — to the number
//! of frames the stream codes, at its size and chroma format. The suite
//! ships reconstructed samples for two streams, in the decoding traces of
//! their first macroblock row; those must match exactly.

use mpeg2::{ChromaFormat, Decoder, Frame};
use std::path::{Path, PathBuf};

/// (path, frames, width, height, chroma). Frame counts are the streams'
/// own — the number of coded frames — which their READMEs state, except
/// where a README rounds (ntr_skipped_v3: 60 coded, "19 + 19 + 21";
/// tcela-14: 61, "60"; mei_2stream.60f.new: 61, "60"; bits_conf_lep_11:
/// I, P, B, "4 frames").
const STREAMS: &[(&str, usize, u32, u32, ChromaFormat)] = {
    use ChromaFormat::{Yuv420 as C420, Yuv422 as C422};
    &[
        ("422-profile/hhi/hhi_burst_422/hhi_burst_422_long.bits", 46, 720, 608, C422),
        ("422-profile/hhi/hhi_burst_422/hhi_burst_422_short.bits", 4, 720, 608, C422),
        ("422-profile/ibm/ibm_dp_intra_422/ibm_dp_intra_422.m2v", 66, 720, 512, C422),
        ("422-profile/sony/sony_422_id01-1/sony_422_id01-1.bs", 73, 720, 512, C422),
        ("422-profile/sony/sony_422_id03-1/sony_422_id03-1.bs", 60, 720, 512, C422),
        ("422-profile/sony/sony_422_id13-1/sony_422_id13-1.bs", 60, 720, 512, C422),
        ("422-profile/tek/Tek6-422-bigBpic/Tek6.bit", 4, 720, 486, C422),
        ("422-profile/tek/Tek7-422-smallSlices/Tek7.bit", 4, 720, 486, C422),
        ("422-profile/tek/Tek9-422-uniformVLC/Tek9.bit", 4, 720, 512, C422),
        ("main-profile/att/att_mismatch/att.bits", 132, 32, 32, C420),
        ("main-profile/ccett/mcp10ccett/mcp10ccett.bits", 4, 720, 576, C420),
        ("main-profile/chromatic/chroma_dct_type-1/test.mpg", 1, 720, 480, C420),
        ("main-profile/compcore/ccm1/ccm1.mpg", 129, 16, 16, C420),
        ("main-profile/gi/gi4/video.bits", 4, 704, 480, C420),
        ("main-profile/gi/gi6/bit_stream", 4, 720, 480, C420),
        ("main-profile/gi/gi7/bit_stream", 4, 720, 480, C420),
        ("main-profile/gi/gi_9/bit_stream", 16, 720, 480, C420),
        ("main-profile/gi/gi_from_tape/gi_stream", 19, 720, 480, C420),
        ("main-profile/hhi/hhi_burst_long/hhi_burst_long.bits", 46, 720, 576, C420),
        ("main-profile/hhi/hhi_burst_short/hhi_burst_short.bits", 4, 720, 576, C420),
        ("main-profile/ibm/ibm-bw-v3/ibm-bw.BITS", 30, 720, 480, C420),
        ("main-profile/lep/bits_conf_lep_11/bits_conf_lep_11.bits", 3, 720, 576, C420),
        ("main-profile/mei/MEI.stream16.long/MEI.stream16.long", 61, 352, 240, C420),
        ("main-profile/mei/MEI.stream16v2/MEI.stream16v2", 4, 352, 240, C420),
        ("main-profile/mei/mei.2conftest.4f/mei_2stream.4f", 4, 704, 480, C420),
        ("main-profile/mei/mei.2conftest.60f.new/mei_2stream.60f.new", 61, 704, 480, C420),
        ("main-profile/nokia/nokia6/nokia6_dual.bit", 4, 720, 576, C420),
        ("main-profile/nokia/nokia6/nokia6_dual_60.bit", 60, 720, 576, C420),
        ("main-profile/nokia/nokia_7/nokia7_dual.bit", 4, 720, 576, C420),
        ("main-profile/ntr/ntr_skipped_v3/ntr_skipped_v3.bits", 60, 720, 576, C420),
        ("main-profile/sony/sony-ct1/sony-ct1.bits", 60, 352, 224, C420),
        ("main-profile/sony/sony-ct2/sony-ct2.bits", 24, 704, 480, C420),
        ("main-profile/sony/sony-ct3/sony-ct3.bs", 7, 720, 480, C420),
        ("main-profile/sony/sony-ct4/sony-ct4.bs", 60, 720, 480, C420),
        ("main-profile/tceh/tceh_conf2/conf2.bits", 31, 720, 576, C420),
        ("main-profile/tcela/tcela-10-killer/tcela-10.bits", 61, 720, 480, C420),
        ("main-profile/tcela/tcela-14-bff-dp/tcela-14.bits", 61, 720, 480, C420),
        ("main-profile/tcela/tcela-14-bff-dp/tcela-14.short.bits", 4, 720, 480, C420),
        ("main-profile/tcela/tcela-15-stuffing/tcela-15.bits", 61, 720, 480, C420),
        ("main-profile/tcela/tcela-16-matrices/tcela-16.bits", 31, 352, 240, C420),
        ("main-profile/tcela/tcela-17-dots/tcela-17.bits", 2, 720, 480, C420),
        ("main-profile/tcela/tcela-18-d-pict/tcela-18.bits", 31, 720, 480, C420),
        ("main-profile/tcela/tcela-19-wide/tcela-19.bits", 31, 768, 128, C420),
        ("main-profile/tcela/tcela-6-slices/tcela-6.bits", 4, 720, 480, C420),
        ("main-profile/tcela/tcela-7-slices/tcela-7.bits", 61, 720, 480, C420),
        ("main-profile/tcela/tcela-8-fp-dp/tcela-8.bits", 4, 720, 480, C420),
        ("main-profile/tcela/tcela-9-fp-dp/tcela-9.bits", 61, 720, 480, C420),
        ("main-profile/tek/Tek-5-long/conf4.bit", 150, 704, 480, C420),
        ("main-profile/tek/Tek-5.2/conf4.bit", 4, 704, 480, C420),
        ("main-profile/teracom/teracom_vlc4/teracom_vlc4.bin", 65, 720, 576, C420),
        ("main-profile/ti/TI_cl_2/TI_c1_2.bits", 4, 704, 480, C420),
        ("main-profile/toshiba/toshiba_DPall-0/toshiba_DPall-0.mpg", 4, 720, 480, C420),
        ("main-profile/twilight_zone/anonymous/mpeg_target_practice.mpg", 1, 480, 480, C420),
        // Two sequences, 704x480 then 256x256 after a sequence_end_code: the
        // frame size checked is the last one's.
        ("main-profile/twilight_zone/mei/MEI.stream17.long/MEI.stream17.long", 122, 256, 256, C420),
        ("main-profile/twilight_zone/mei/MEI2.stream17/MEI2.stream17", 8, 256, 256, C420),
        ("main-profile/twilight_zone/tcela/tcela-11v2/tcela-11v2.bits", 15, 352, 240, C420),
        ("main-profile/twilight_zone/tcela/tcela-12/tcela-12.bits", 30, 352, 240, C420),
    ]
};

fn suite() -> Option<PathBuf> {
    match std::env::var_os("MPEG2_CONFORMANCE_DIR") {
        Some(d) => Some(PathBuf::from(d)),
        None => {
            assert!(
                std::env::var_os("MPEG2_REQUIRE_CONFORMANCE").is_none(),
                "MPEG2_REQUIRE_CONFORMANCE is set but MPEG2_CONFORMANCE_DIR is not"
            );
            eprintln!("conformance: MPEG2_CONFORMANCE_DIR not set; run tools/fetch-conformance.sh — NOT RUN");
            None
        }
    }
}

fn decode_file(path: &Path) -> Vec<Frame> {
    let data = std::fs::read(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let mut dec = Decoder::new();
    let mut frames = Vec::new();
    for chunk in data.chunks(65536) {
        frames.extend(dec.decode(chunk).unwrap_or_else(|e| panic!("{}: {e}", path.display())));
    }
    frames.extend(dec.flush().unwrap_or_else(|e| panic!("{}: {e}", path.display())));
    frames
}

#[test]
fn every_stream_decodes_to_its_frame_count() {
    let Some(dir) = suite() else { return };
    let mut total = 0;
    for &(path, n, w, h, chroma) in STREAMS {
        let frames = decode_file(&dir.join(path));
        assert_eq!(frames.len(), n, "{path}: frame count");
        let last = frames.last().unwrap();
        assert_eq!((last.width, last.height, last.chroma), (w, h, chroma), "{path}");
        total += frames.len();
    }
    eprintln!("conformance: {} streams, {total} frames decoded", STREAMS.len());
}

/// The reconstructed macroblocks of a trace: (picture in decoding order,
/// luma x, luma y, 16×16 luma samples). Two trace formats: tcela-17's
/// (`mb_addr: n` … `Reconstructed:`) and nokia6_dual's (`--- X: x, Y: y ---`
/// … `Reconstructed macroblock`).
fn trace_macroblocks(text: &str, width: u32) -> Vec<(usize, u32, u32, Vec<u8>)> {
    let mut out = Vec::new();
    let mut pic: Option<usize> = None;
    let (mut x, mut y) = (0, 0);
    let mut lines = text.lines();
    while let Some(l) = lines.next() {
        let t = l.trim();
        let lower = t.to_ascii_lowercase();
        if t.starts_with('%') && lower.trim_matches('%').trim() == "picture header" {
            pic = Some(pic.map_or(0, |p| p + 1));
        } else if let Some(rest) = t.strip_prefix("mb_addr: ") {
            let a: u32 = rest.split(',').next().unwrap().trim().parse().unwrap();
            let mbw = width / 16;
            (x, y) = ((a % mbw) * 16, (a / mbw) * 16);
        } else if let Some(rest) = t.strip_prefix("--- X: ") {
            let mut parts = rest.trim_end_matches(" ---").split(", Y: ");
            x = parts.next().unwrap().parse().unwrap();
            y = parts.next().unwrap().parse().unwrap();
        } else if t.starts_with("Reconstructed") {
            let mut samples = Vec::with_capacity(256);
            while samples.len() < 256 {
                let row: Vec<u8> = lines.next().unwrap().split_whitespace().map(|v| v.parse().unwrap()).collect();
                if row.len() == 16 {
                    samples.extend(row);
                }
            }
            out.push((pic.expect("picture header before macroblocks"), x, y, samples));
        }
    }
    out
}

fn check_trace(dir: &Path, stream: &str, trace: &str, expect_mbs: usize) {
    let frames = decode_file(&dir.join(stream));
    let text = std::fs::read_to_string(dir.join(trace)).expect("trace");
    let mbs = trace_macroblocks(&text, frames[0].width);
    assert_eq!(mbs.len(), expect_mbs, "{trace}: macroblocks traced");
    let mut compared = 0;
    for (pic, x, y, samples) in &mbs {
        // These streams have no B-pictures: decoding order is display order.
        let f = &frames[*pic];
        let w = f.width as usize;
        for j in 0..16 {
            for i in 0..16 {
                let got = f.plane(0)[(*y as usize + j) * w + *x as usize + i];
                assert_eq!(got, samples[j * 16 + i], "{stream}: picture {pic}, sample ({}, {})", *x as usize + i, *y as usize + j);
                compared += 1;
            }
        }
    }
    eprintln!("conformance: {stream}: {compared} reconstructed samples match the trace");
}

/// tcela-17: motion vectors at the limit of the range Main Level allows.
#[test]
fn tcela_17_matches_its_trace() {
    let Some(dir) = suite() else { return };
    check_trace(
        &dir,
        "main-profile/tcela/tcela-17-dots/tcela-17.bits",
        "main-profile/tcela/tcela-17-dots/tcela-17.trace",
        90,
    );
}

/// nokia6_dual: every macroblock dual-prime, in frame pictures.
#[test]
fn nokia6_dual_matches_its_trace() {
    let Some(dir) = suite() else { return };
    check_trace(
        &dir,
        "main-profile/nokia/nokia6/nokia6_dual.bit",
        "main-profile/nokia/nokia6/nokia6_dual.trace",
        180,
    );
}

/// chroma_dct_type-1 ships the reconstruction of a decoder that wrongly
/// field-organises 4:2:0 chroma when dct_type is 1: the luma must match it
/// and the chroma must not.
#[test]
fn chroma_dct_type_is_not_applied_to_420_chroma() {
    let Some(dir) = suite() else { return };
    let f = decode_file(&dir.join("main-profile/chromatic/chroma_dct_type-1/test.mpg"));
    let wrong = std::fs::read(dir.join("main-profile/chromatic/chroma_dct_type-1/wrong.decoded")).expect("wrong.decoded");
    assert_eq!(wrong.len(), f[0].data.len());
    let luma = f[0].plane(0).len();
    assert_eq!(&wrong[..luma], f[0].plane(0));
    let differ = wrong[luma..].iter().zip(&f[0].data[luma..]).filter(|(a, b)| a != b).count();
    assert!(differ > (wrong.len() - luma) / 2, "chroma differs from the wrong decode in {differ} samples");
}

/// The encoder on natural pictures: the first 12 frames of tcela-7 (Mobile &
/// Calendar, 720x480) as decoded, re-encoded at quantiser_scale_code 6 with
/// two B-pictures between references, and decoded again.
#[test]
fn encoder_on_natural_pictures() {
    use mpeg2::{Encoder, EncoderConfig, RateControl};
    let Some(dir) = suite() else { return };
    let src: Vec<Frame> =
        decode_file(&dir.join("main-profile/tcela/tcela-7-slices/tcela-7.bits")).into_iter().take(12).collect();
    let (w, h) = (src[0].width, src[0].height);
    let cfg = EncoderConfig {
        rate_control: RateControl::ConstantQuantiser(6),
        frame_rate: (30000, 1001),
        ..EncoderConfig::new(w, h)
    };
    let mut enc = Encoder::new(cfg).unwrap();
    let mut stream = Vec::new();
    for f in &src {
        stream.extend(enc.encode(f).unwrap());
    }
    stream.extend(enc.finish().unwrap());
    let out = decode_file_bytes(&stream);
    assert_eq!(out.len(), src.len());
    let psnr = |a: &[u8], b: &[u8]| {
        let mse = a.iter().zip(b).map(|(&x, &y)| (f64::from(x) - f64::from(y)).powi(2)).sum::<f64>() / a.len() as f64;
        10.0 * (255.0f64 * 255.0 / mse).log10()
    };
    let y = src.iter().zip(&out).map(|(a, b)| psnr(a.plane(0), b.plane(0))).sum::<f64>() / src.len() as f64;
    let rate = stream.len() as f64 * 8.0 * 30000.0 / 1001.0 / src.len() as f64;
    eprintln!("conformance: tcela-7 re-encoded at q 6: {:.2} Mb/s, luma PSNR {y:.2} dB", rate / 1e6);
    assert!(y > 35.5, "luma PSNR {y:.2}");
}

fn decode_file_bytes(data: &[u8]) -> Vec<Frame> {
    let mut dec = Decoder::new();
    let mut frames = dec.decode(data).unwrap();
    frames.extend(dec.flush().unwrap());
    frames
}
