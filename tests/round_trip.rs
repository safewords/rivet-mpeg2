//! Round trips through this crate's encoder and decoder: every frame comes
//! back, in display order, at the quality the quantiser implies.

mod common;

use common::*;
use mpeg2::{EncoderConfig, PictureType, RateControl};

fn check(cfg: EncoderConfig, n: u32, min_psnr: f64) -> (f64, usize) {
    let (w, h) = (cfg.width, cfg.height);
    let src: Vec<_> = (0..n).map(|t| synthetic(w, h, t)).collect();
    let stream = encode(cfg.clone(), &src);
    let dec = decode(&stream, 4096);
    assert_eq!(dec.len(), src.len(), "frame count");
    let mut worst = f64::INFINITY;
    let mut sum = 0.0;
    for (i, (a, b)) in src.iter().zip(&dec).enumerate() {
        assert_eq!((b.width, b.height), (w, h));
        let p = psnr(a, b, 0);
        let pc = psnr(a, b, 1).min(psnr(a, b, 2));
        assert!(p >= min_psnr, "frame {i} ({:?}): luma PSNR {p:.2} dB", b.picture_type);
        assert!(pc >= min_psnr, "frame {i}: chroma PSNR {pc:.2} dB");
        worst = worst.min(p);
        sum += p;
    }
    let mean = sum / f64::from(n);
    eprintln!(
        "{w}x{h} b={} q={:?} ivlc={} alt={} qst={} dcp={}: {} bytes, mean luma PSNR {mean:.2} dB, worst {worst:.2} dB",
        cfg.b_frames,
        cfg.rate_control,
        cfg.intra_vlc_format,
        cfg.alternate_scan,
        cfg.q_scale_type,
        cfg.intra_dc_precision,
        stream.len()
    );
    (mean, stream.len())
}

#[test]
fn intra_only() {
    let cfg = EncoderConfig { gop_size: 1, b_frames: 0, ..EncoderConfig::new(64, 48) };
    check(cfg, 4, 36.0);
}

#[test]
fn i_and_p() {
    let cfg = EncoderConfig { b_frames: 0, ..EncoderConfig::new(96, 64) };
    check(cfg, 10, 36.0);
}

#[test]
fn i_p_and_b_in_display_order() {
    let cfg = EncoderConfig { b_frames: 2, gop_size: 6, ..EncoderConfig::new(80, 48) };
    let src: Vec<_> = (0..14).map(|t| synthetic(80, 48, t)).collect();
    let stream = encode(cfg.clone(), &src);
    let dec = decode(&stream, 7);
    assert_eq!(dec.len(), 14);
    // Display order: each decoded frame is closest to its own source.
    for (i, d) in dec.iter().enumerate() {
        let best = (0..src.len()).max_by(|&a, &b| psnr(&src[a], d, 0).total_cmp(&psnr(&src[b], d, 0))).unwrap();
        assert_eq!(best, i, "frame {i} matches source {best}");
    }
    let types: Vec<_> = dec.iter().map(|f| f.picture_type).collect();
    assert_eq!(&types[..7], &[
        PictureType::I,
        PictureType::B,
        PictureType::B,
        PictureType::P,
        PictureType::B,
        PictureType::B,
        PictureType::I
    ]);
    check(cfg, 14, 36.0);
}

#[test]
fn coding_tools() {
    for (ivlc, alt, qst, dcp) in
        [(false, false, false, 0), (true, true, false, 1), (false, true, true, 2), (true, false, true, 0)]
    {
        let cfg = EncoderConfig {
            intra_vlc_format: ivlc,
            alternate_scan: alt,
            q_scale_type: qst,
            intra_dc_precision: dcp,
            gop_size: 5,
            b_frames: 1,
            ..EncoderConfig::new(48, 32)
        };
        check(cfg, 7, 34.0);
    }
}

#[test]
fn odd_sizes_are_cropped() {
    let cfg = EncoderConfig { b_frames: 1, ..EncoderConfig::new(37, 21) };
    check(cfg, 5, 34.0);
}

#[test]
fn quantiser_trades_size_for_quality() {
    let mut last = (f64::INFINITY, 0usize);
    for q in [2u8, 6, 16, 31] {
        let cfg = EncoderConfig { rate_control: RateControl::ConstantQuantiser(q), ..EncoderConfig::new(64, 64) };
        let r = check(cfg, 6, 24.0);
        assert!(r.0 < last.0, "PSNR falls as q rises");
        if last.1 != 0 {
            assert!(r.1 < last.1, "size falls as q rises");
        }
        last = r;
    }
}

#[test]
fn rate_control_tracks_the_target() {
    let (w, h, n) = (128u32, 96u32, 24u32);
    for target in [200_000u32, 800_000] {
        let cfg = EncoderConfig { rate_control: RateControl::Bitrate(target), ..EncoderConfig::new(w, h) };
        let (_, bytes) = check(cfg, n, 22.0);
        let rate = bytes as f64 * 8.0 * 25.0 / f64::from(n);
        eprintln!("target {target} b/s: got {rate:.0} b/s");
        assert!(rate > f64::from(target) * 0.5 && rate < f64::from(target) * 1.6, "rate {rate}");
    }
}

#[test]
fn bad_configurations_and_frames_are_refused() {
    use mpeg2::{ChromaFormat, Encoder, Error, Frame};
    for cfg in [
        EncoderConfig::new(0, 16),
        EncoderConfig::new(16, 3000),
        EncoderConfig { frame_rate: (1, 1000), ..EncoderConfig::new(16, 16) },
        EncoderConfig { rate_control: RateControl::ConstantQuantiser(0), ..EncoderConfig::new(16, 16) },
        EncoderConfig { intra_dc_precision: 3, ..EncoderConfig::new(16, 16) },
        EncoderConfig { gop_size: 0, ..EncoderConfig::new(16, 16) },
    ] {
        assert!(matches!(Encoder::new(cfg), Err(Error::Config(_))));
    }
    let mut enc = Encoder::new(EncoderConfig::new(32, 32)).unwrap();
    assert!(matches!(enc.encode(&Frame::new(16, 16, ChromaFormat::Yuv420)), Err(Error::Config(_))));
    assert!(matches!(enc.encode(&Frame::new(32, 32, ChromaFormat::Yuv422)), Err(Error::Config(_))));
    let mut broken = Frame::new(32, 32, ChromaFormat::Yuv420);
    broken.data.truncate(100);
    assert!(matches!(enc.encode(&broken), Err(Error::Config(_))));
    assert!(enc.encode(&Frame::new(32, 32, ChromaFormat::Yuv420)).is_ok());
}

#[test]
fn sequence_info_describes_the_stream() {
    let cfg = EncoderConfig {
        frame_rate: (30000, 1001),
        aspect_ratio_information: 3,
        b_frames: 0,
        ..EncoderConfig::new(40, 24)
    };
    let stream = encode(cfg, &[synthetic(40, 24, 0), synthetic(40, 24, 1)]);
    let mut dec = mpeg2::Decoder::new();
    let mut frames = dec.decode(&stream).unwrap();
    frames.extend(dec.flush().unwrap());
    let s = dec.sequence().unwrap();
    assert_eq!((s.width, s.height, s.chroma), (40, 24, mpeg2::ChromaFormat::Yuv420));
    assert_eq!(s.frame_rate, Some((30000, 1001)));
    assert_eq!(s.aspect_ratio_information, 3);
    assert_eq!(s.profile_and_level_indication, 0x48);
    assert!(s.progressive_sequence && s.low_delay && !s.mpeg1);
    assert_eq!(frames.len(), 2);
    for (i, f) in frames.iter().enumerate() {
        assert!(f.progressive_frame && !f.repeat_first_field && !f.field_pictures);
        assert_eq!(f.decode_index, i as u64);
        assert_eq!(f.temporal_reference, i as u16);
    }
}
