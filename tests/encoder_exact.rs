//! The encoder's output, exactly: the stream does not depend on the thread
//! count or the SIMD kernels (it is the one recorded below, made by the
//! original single-threaded scalar encoder), and a decoder reconstructs
//! every frame — B-pictures too — exactly as the encoder did.

mod common;

use common::*;
use mpeg2::{Encoder, EncoderConfig, Frame, RateControl};

/// FNV-1a.
fn fnv(data: &[u8]) -> u64 {
    data.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &b| {
        (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

/// (name, configuration, frames).
fn configs() -> Vec<(&'static str, EncoderConfig, u32)> {
    vec![
        ("default 176x144", EncoderConfig::new(176, 144), 9),
        (
            "i/p 208x160 range 64",
            EncoderConfig {
                b_frames: 0,
                search_range: 64,
                ..EncoderConfig::new(208, 160)
            },
            6,
        ),
        (
            "tools 96x64",
            EncoderConfig {
                intra_vlc_format: false,
                alternate_scan: true,
                q_scale_type: true,
                intra_dc_precision: 2,
                gop_size: 5,
                b_frames: 3,
                ..EncoderConfig::new(96, 64)
            },
            9,
        ),
        (
            "odd 37x21",
            EncoderConfig {
                b_frames: 1,
                ..EncoderConfig::new(37, 21)
            },
            5,
        ),
        (
            "q2",
            EncoderConfig {
                rate_control: RateControl::ConstantQuantiser(2),
                ..EncoderConfig::new(64, 48)
            },
            5,
        ),
        (
            "q31",
            EncoderConfig {
                rate_control: RateControl::ConstantQuantiser(31),
                ..EncoderConfig::new(64, 48)
            },
            5,
        ),
        (
            "bitrate 128x96",
            EncoderConfig {
                rate_control: RateControl::Bitrate(400_000),
                ..EncoderConfig::new(128, 96)
            },
            14,
        ),
    ]
}

fn source(cfg: &EncoderConfig, n: u32) -> Vec<Frame> {
    (0..n)
        .map(|t| synthetic(cfg.width, cfg.height, t))
        .collect()
}

/// Recorded from the encoder before the SIMD kernels and the threads.
const STREAMS: &[(&str, u64)] = &[
    ("default 176x144", 0xbbcc5adda50959f4),
    ("i/p 208x160 range 64", 0x5b842c7e20bcc763),
    ("tools 96x64", 0x7c04e26edab5a200),
    ("odd 37x21", 0xc6fe92a85f1a040d),
    ("q2", 0x7367538ea1a0e5cf),
    ("q31", 0xa39aca0b126f5464),
    ("bitrate 128x96", 0xce900ed6bee23a1d),
];

#[test]
fn the_stream_is_as_recorded_on_any_thread_count() {
    let print = std::env::var_os("MPEG2_PRINT_HASHES").is_some();
    for (name, cfg, n) in configs() {
        let src = source(&cfg, n);
        for threads in [1, 3, 8] {
            let h = fnv(&encode(with_threads(cfg.clone(), threads), &src));
            if print {
                println!("    (\"{name}\", {h:#018x}),");
                break;
            }
            let want = STREAMS.iter().find(|s| s.0 == name).expect("recorded").1;
            assert_eq!(
                h, want,
                "{name}, {threads} threads: {h:#018x}, recorded {want:#018x}"
            );
        }
    }
}

fn with_threads(cfg: EncoderConfig, threads: usize) -> EncoderConfig {
    EncoderConfig { threads, ..cfg }
}

#[test]
fn the_decoder_reconstructs_every_frame_as_the_encoder_did() {
    for (name, cfg, n) in configs() {
        let src = source(&cfg, n);
        let mut enc = Encoder::new(cfg).unwrap();
        enc.keep_reconstructions();
        let mut stream = Vec::new();
        for f in &src {
            stream.extend(enc.encode(f).unwrap());
        }
        stream.extend(enc.finish().unwrap());
        let recon = enc.take_reconstructions();
        let dec = decode(&stream, 4096);
        assert_eq!(recon.len(), src.len(), "{name}");
        assert_eq!(dec.len(), src.len(), "{name}");
        for (i, (r, d)) in recon.iter().zip(&dec).enumerate() {
            assert!(
                r.data == d.data,
                "{name}: frame {i} ({:?}) differs from the encoder's reconstruction",
                d.picture_type
            );
        }
    }
}

/// The thread pools inside keep both types movable to and shareable
/// between threads, as they were.
#[test]
fn decoder_and_encoder_are_send_and_sync() {
    fn check<T: Send + Sync>() {}
    check::<mpeg2::Decoder>();
    check::<Encoder>();
}
