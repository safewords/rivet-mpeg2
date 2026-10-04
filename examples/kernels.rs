//! Per-kernel timings: each kernel on every SIMD rung this processor runs,
//! and the code it replaced ("original", copied below as it was), in
//! nanoseconds per call — the fastest of many repetitions.
//!
//! `cargo run --release --example kernels`

use std::hint::black_box;
use std::time::Instant;

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn range(&mut self, lo: i32, hi: i32) -> i32 {
        lo + (self.next() % (hi - lo + 1) as u64) as i32
    }
}

/// Nanoseconds per item of `f` over `n` items: the fastest of 25 runs.
fn time(n: usize, mut f: impl FnMut()) -> f64 {
    let mut best = f64::INFINITY;
    for _ in 0..25 {
        let t = Instant::now();
        f();
        best = best.min(t.elapsed().as_secs_f64());
    }
    best * 1e9 / n as f64
}

// ----------------------------------------- the code the kernels replaced

const BASIS: [[f64; 8]; 8] = {
    // C(u)/2 · cos((2x+1)uπ/16) — the same literal values as src/idct.rs.
    [
        [0.35355339059327373; 8],
        [
            0.4903926402016152,
            0.4157348061512726,
            0.27778511650980114,
            0.09754516100806417,
            -0.0975451610080641,
            -0.277785116509801,
            -0.4157348061512727,
            -0.4903926402016152,
        ],
        [
            0.46193976625564337,
            0.19134171618254492,
            -0.19134171618254486,
            -0.46193976625564337,
            -0.4619397662556434,
            -0.19134171618254517,
            0.191341716182545,
            0.46193976625564326,
        ],
        [
            0.4157348061512726,
            -0.0975451610080641,
            -0.4903926402016152,
            -0.2777851165098011,
            0.2777851165098009,
            0.4903926402016152,
            0.09754516100806439,
            -0.41573480615127256,
        ],
        [
            0.3535533905932738,
            -0.35355339059327373,
            -0.35355339059327384,
            0.3535533905932737,
            0.35355339059327384,
            -0.35355339059327334,
            -0.35355339059327356,
            0.3535533905932733,
        ],
        [
            0.27778511650980114,
            -0.4903926402016152,
            0.09754516100806415,
            0.4157348061512728,
            -0.41573480615127256,
            -0.09754516100806401,
            0.4903926402016153,
            -0.27778511650980076,
        ],
        [
            0.19134171618254492,
            -0.4619397662556434,
            0.46193976625564326,
            -0.19134171618254495,
            -0.19134171618254528,
            0.46193976625564337,
            -0.4619397662556432,
            0.19134171618254478,
        ],
        [
            0.09754516100806417,
            -0.2777851165098011,
            0.4157348061512728,
            -0.4903926402016153,
            0.49039264020161527,
            -0.4157348061512725,
            0.27778511650980076,
            -0.09754516100806429,
        ],
    ]
};

fn idct_original(block: &mut [i32; 64]) {
    if block[1..].iter().all(|&c| c == 0) {
        let v = (f64::from(block[0]) / 8.0).round().clamp(-256.0, 255.0) as i32;
        block.fill(v);
        return;
    }
    let mut tmp = [0.0f64; 64];
    for v in 0..8 {
        let row = &block[v * 8..v * 8 + 8];
        if row.iter().all(|&c| c == 0) {
            continue;
        }
        for x in 0..8 {
            let mut acc = 0.0;
            for u in 0..8 {
                acc += f64::from(row[u]) * BASIS[u][x];
            }
            tmp[v * 8 + x] = acc;
        }
    }
    for x in 0..8 {
        for y in 0..8 {
            let mut acc = 0.0;
            for v in 0..8 {
                acc += tmp[v * 8 + x] * BASIS[v][y];
            }
            block[y * 8 + x] = acc.round().clamp(-256.0, 255.0) as i32;
        }
    }
}

fn fdct_original(input: &[i32; 64]) -> [i32; 64] {
    let mut tmp = [0.0f64; 64];
    for y in 0..8 {
        for u in 0..8 {
            let mut acc = 0.0;
            for x in 0..8 {
                acc += f64::from(input[y * 8 + x]) * BASIS[u][x];
            }
            tmp[y * 8 + u] = acc;
        }
    }
    let mut out = [0; 64];
    for u in 0..8 {
        for v in 0..8 {
            let mut acc = 0.0;
            for y in 0..8 {
                acc += tmp[y * 8 + u] * BASIS[v][y];
            }
            out[v * 8 + u] = acc.round() as i32;
        }
    }
    out
}

#[allow(clippy::too_many_arguments)]
fn mc_original(src: &[u8], stride: usize, off: usize, w: usize, h: usize, hx: i32, hy: i32, avg: bool, d: &mut [u8]) {
    for j in 0..h {
        let r0 = off + j * stride;
        let r1 = off + (j + hy as usize) * stride;
        let n = w + hx as usize;
        let a = &src[r0..r0 + n];
        let b = &src[r1..r1 + n];
        let d = &mut d[j * w..j * w + w];
        for i in 0..w {
            let p = match (hx, hy) {
                (0, 0) => u32::from(a[i]),
                (1, 0) => (u32::from(a[i]) + u32::from(a[i + 1]) + 1) >> 1,
                (0, _) => (u32::from(a[i]) + u32::from(b[i]) + 1) >> 1,
                _ => (u32::from(a[i]) + u32::from(a[i + 1]) + u32::from(b[i]) + u32::from(b[i + 1]) + 2) >> 2,
            };
            d[i] = if avg { ((u32::from(d[i]) + p + 1) >> 1) as u8 } else { p as u8 };
        }
    }
}

fn add_block_original(plane: &mut [u8], x0: usize, intra: bool, blk: &[i32; 64]) {
    for y in 0..8 {
        let row = &mut plane[y * 16 + x0..][..8];
        let b = &blk[y * 8..y * 8 + 8];
        for x in 0..8 {
            let base = if intra { 0 } else { i32::from(row[x]) };
            row[x] = (base + b[x]).clamp(0, 255) as u8;
        }
    }
}

fn sad_original(a: &[u8], b: &[u8], w: usize, bo: usize, limit: u32) -> u32 {
    let mut sad = 0u32;
    for j in 0..16 {
        let ra = &a[j * w..][..16];
        let rb = &b[bo + j * w..][..16];
        for i in 0..16 {
            sad += u32::from(ra[i].abs_diff(rb[i]));
        }
        if sad >= limit {
            return sad;
        }
    }
    sad
}

fn main() {
    let rungs = mpeg2::__bench::rungs();
    let mut rng = Rng(0x2545_f491_4f6c_dd1d);
    println!("rungs: {rungs:?}; in use: {}", mpeg2::simd_rung());

    // Coefficient blocks: "sparse" as most coded blocks are (2-6
    // coefficients in the first rows), and "dense" (all 64).
    let n = 4096;
    let sparse: Vec<[i32; 64]> = (0..n)
        .map(|_| {
            let mut b = [0i32; 64];
            for _ in 0..rng.range(2, 6) {
                b[rng.range(0, 23) as usize] = rng.range(-200, 200);
            }
            // Mismatch control makes F[7][7] odd in about half the blocks.
            if rng.next() & 1 == 0 {
                b[63] = 1;
            }
            b
        })
        .collect();
    let dense: Vec<[i32; 64]> =
        (0..n).map(|_| std::array::from_fn(|_| rng.range(-300, 300))).collect();
    let samples: Vec<[i32; 64]> = (0..n).map(|_| std::array::from_fn(|_| rng.range(-255, 255))).collect();

    println!("\n{:<28} {:>10} {}", "kernel", "original", rungs.iter().map(|r| format!("{r:>10}")).collect::<String>());
    let row = |name: &str, orig: f64, per: Vec<f64>| {
        println!("{name:<28} {orig:>10.1} {}", per.iter().map(|v| format!("{v:>10.1}")).collect::<String>());
    };

    for (label, set) in [("idct 8x8 sparse", &sparse), ("idct 8x8 dense", &dense)] {
        let orig = time(n, || {
            for b in set.iter() {
                let mut b = *b;
                idct_original(&mut b);
                black_box(&b);
            }
        });
        let per = rungs
            .iter()
            .map(|r| {
                let mut work = set.clone();
                time(n, || {
                    work.copy_from_slice(set);
                    mpeg2::__bench::idct(r, black_box(&mut work));
                })
            })
            .collect();
        row(label, orig, per);
    }
    {
        let orig = time(n, || {
            for b in &samples {
                black_box(fdct_original(b));
            }
        });
        let per = rungs.iter().map(|r| time(n, || { black_box(mpeg2::__bench::fdct(r, &samples)); })).collect();
        row("fdct 8x8", orig, per);
    }

    // Motion compensation: random positions in a 1920-wide plane.
    let stride = 1920;
    let src: Vec<u8> = (0..stride * 200).map(|_| (rng.next() & 255) as u8).collect();
    let offs: Vec<usize> = (0..n).map(|_| rng.range(0, 150) as usize * stride + rng.range(0, 1880) as usize).collect();
    for (w, h) in [(16, 16), (8, 8)] {
        for (hx, hy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
            for avg in [false, true] {
                let mut d = vec![0u8; w * h];
                let orig = time(n, || {
                    for &o in &offs {
                        mc_original(&src, stride, o, w, h, hx, hy, avg, &mut d);
                    }
                    black_box(&d);
                });
                let per = rungs
                    .iter()
                    .map(|r| {
                        time(n, || {
                            mpeg2::__bench::mc(r, &src, stride, &offs, w, h, hx == 1, hy == 1, avg, &mut d);
                            black_box(&d);
                        })
                    })
                    .collect();
                row(&format!("mc {w}x{h} h{hx}{hy}{}", if avg { " avg" } else { "" }), orig, per);
            }
        }
    }

    // Residual addition.
    let res: Vec<[i32; 64]> = (0..n).map(|_| std::array::from_fn(|_| rng.range(-256, 255))).collect();
    for intra in [false, true] {
        let mut d = vec![128u8; 16 * 8];
        let orig = time(n, || {
            for (i, r) in res.iter().enumerate() {
                add_block_original(&mut d, (i & 1) * 8, intra, r);
            }
            black_box(&d);
        });
        let per = rungs
            .iter()
            .map(|r| {
                time(n, || {
                    mpeg2::__bench::add_block(r, &res, &mut d, intra);
                    black_box(&d);
                })
            })
            .collect();
        row(if intra { "put block 8x8 (intra)" } else { "add block 8x8" }, orig, per);
    }

    // SAD.
    let a: Vec<u8> = src[..stride * 16].to_vec();
    let orig = time(n, || {
        let s: u64 = offs.iter().map(|&o| u64::from(sad_original(&a, &src, stride, o, u32::MAX))).sum();
        black_box(s);
    });
    let per = rungs.iter().map(|r| time(n, || { black_box(mpeg2::__bench::sad16(r, &a, &src, stride, &offs)); })).collect();
    row("sad 16x16", orig, per);
}
