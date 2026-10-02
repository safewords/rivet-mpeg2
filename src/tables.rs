//! The normative tables of ITU-T H.262 (02/2000) | ISO/IEC 13818-2:2000,
//! transcribed from the Recommendation: the variable length codes of Annex B,
//! the scans of Figures 7-2 and 7-3, the default quantiser matrices of 6.3.11,
//! Table 7-6 (quantiser_scale) and Table 6-4 (frame_rate_value).
//!
//! Codes are written as the Recommendation prints them — strings of `0` and
//! `1` with spaces between nibbles — so each row can be compared with the
//! printed table by eye. The sign bit `s` of Tables B.10, B.14 and B.15 is
//! not part of the strings; the code tables in [`crate::vlc`] add it.

/// Table B.1 — macroblock_address_increment. `(code, increment)`;
/// `macroblock_escape` is handled separately ([`MB_ESCAPE`]).
pub(crate) const MB_ADDRESS_INCREMENT: &[(&str, u8)] = &[
    ("1", 1),
    ("011", 2),
    ("010", 3),
    ("0011", 4),
    ("0010", 5),
    ("0001 1", 6),
    ("0001 0", 7),
    ("0000 111", 8),
    ("0000 110", 9),
    ("0000 1011", 10),
    ("0000 1010", 11),
    ("0000 1001", 12),
    ("0000 1000", 13),
    ("0000 0111", 14),
    ("0000 0110", 15),
    ("0000 0101 11", 16),
    ("0000 0101 10", 17),
    ("0000 0101 01", 18),
    ("0000 0101 00", 19),
    ("0000 0100 11", 20),
    ("0000 0100 10", 21),
    ("0000 0100 011", 22),
    ("0000 0100 010", 23),
    ("0000 0100 001", 24),
    ("0000 0100 000", 25),
    ("0000 0011 111", 26),
    ("0000 0011 110", 27),
    ("0000 0011 101", 28),
    ("0000 0011 100", 29),
    ("0000 0011 011", 30),
    ("0000 0011 010", 31),
    ("0000 0011 001", 32),
    ("0000 0011 000", 33),
];

/// `macroblock_escape`: `0000 0001 000` (adds 33 to the increment).
// The digit groups follow the printed table.
#[allow(clippy::unusual_byte_groupings)]
pub(crate) const MB_ESCAPE: (u32, u32) = (0b0000_0001_000, 11);
/// ISO/IEC 11172-2 `macroblock_stuffing`: `0000 0001 111` (discarded; H.262
/// D.9.2).
#[allow(clippy::unusual_byte_groupings)]
pub(crate) const MB_STUFFING: (u32, u32) = (0b0000_0001_111, 11);

/// macroblock_type flag bits.
pub(crate) const MB_QUANT: u8 = 1 << 4;
pub(crate) const MB_FORWARD: u8 = 1 << 3;
pub(crate) const MB_BACKWARD: u8 = 1 << 2;
pub(crate) const MB_PATTERN: u8 = 1 << 1;
pub(crate) const MB_INTRA: u8 = 1;

/// Table B.2 — macroblock_type in I-pictures.
pub(crate) const MB_TYPE_I: &[(&str, u8)] = &[("1", MB_INTRA), ("01", MB_QUANT | MB_INTRA)];

/// Table B.3 — macroblock_type in P-pictures.
pub(crate) const MB_TYPE_P: &[(&str, u8)] = &[
    ("1", MB_FORWARD | MB_PATTERN),
    ("01", MB_PATTERN),
    ("001", MB_FORWARD),
    ("0001 1", MB_INTRA),
    ("0001 0", MB_QUANT | MB_FORWARD | MB_PATTERN),
    ("0000 1", MB_QUANT | MB_PATTERN),
    ("0000 01", MB_QUANT | MB_INTRA),
];

/// Table B.4 — macroblock_type in B-pictures.
pub(crate) const MB_TYPE_B: &[(&str, u8)] = &[
    ("10", MB_FORWARD | MB_BACKWARD),
    ("11", MB_FORWARD | MB_BACKWARD | MB_PATTERN),
    ("010", MB_BACKWARD),
    ("011", MB_BACKWARD | MB_PATTERN),
    ("0010", MB_FORWARD),
    ("0011", MB_FORWARD | MB_PATTERN),
    ("0001 1", MB_INTRA),
    ("0001 0", MB_QUANT | MB_FORWARD | MB_BACKWARD | MB_PATTERN),
    ("0000 11", MB_QUANT | MB_FORWARD | MB_PATTERN),
    ("0000 10", MB_QUANT | MB_BACKWARD | MB_PATTERN),
    ("0000 01", MB_QUANT | MB_INTRA),
];

/// ISO/IEC 11172-2 macroblock_type in D-pictures (only `1`, intra); D-pictures
/// are refused, the table documents why.
#[allow(dead_code)]
pub(crate) const MB_TYPE_D: &[(&str, u8)] = &[("1", MB_INTRA)];

/// Table B.9 — coded_block_pattern_420 → cbp. The code `0000 0000 1` (cbp 0)
/// shall not be used with 4:2:0.
pub(crate) const CODED_BLOCK_PATTERN: &[(&str, u8)] = &[
    ("111", 60),
    ("1101", 4),
    ("1100", 8),
    ("1011", 16),
    ("1010", 32),
    ("1001 1", 12),
    ("1001 0", 48),
    ("1000 1", 20),
    ("1000 0", 40),
    ("0111 1", 28),
    ("0111 0", 44),
    ("0110 1", 52),
    ("0110 0", 56),
    ("0101 1", 1),
    ("0101 0", 61),
    ("0100 1", 2),
    ("0100 0", 62),
    ("0011 11", 24),
    ("0011 10", 36),
    ("0011 01", 3),
    ("0011 00", 63),
    ("0010 111", 5),
    ("0010 110", 9),
    ("0010 101", 17),
    ("0010 100", 33),
    ("0010 011", 6),
    ("0010 010", 10),
    ("0010 001", 18),
    ("0010 000", 34),
    ("0001 1111", 7),
    ("0001 1110", 11),
    ("0001 1101", 19),
    ("0001 1100", 35),
    ("0001 1011", 13),
    ("0001 1010", 49),
    ("0001 1001", 21),
    ("0001 1000", 41),
    ("0001 0111", 14),
    ("0001 0110", 50),
    ("0001 0101", 22),
    ("0001 0100", 42),
    ("0001 0011", 15),
    ("0001 0010", 51),
    ("0001 0001", 23),
    ("0001 0000", 43),
    ("0000 1111", 25),
    ("0000 1110", 37),
    ("0000 1101", 26),
    ("0000 1100", 38),
    ("0000 1011", 29),
    ("0000 1010", 45),
    ("0000 1001", 53),
    ("0000 1000", 57),
    ("0000 0111", 30),
    ("0000 0110", 46),
    ("0000 0101", 54),
    ("0000 0100", 58),
    ("0000 0011 1", 31),
    ("0000 0011 0", 47),
    ("0000 0010 1", 55),
    ("0000 0010 0", 59),
    ("0000 0001 1", 27),
    ("0000 0001 0", 39),
    ("0000 0000 1", 0),
];

/// Table B.10 — motion_code magnitudes, without the trailing sign bit
/// (`0` positive, `1` negative). Magnitude 0 is the single code `1`, which
/// has no sign bit.
pub(crate) const MOTION_CODE: &[(&str, u8)] = &[
    ("1", 0),
    ("01", 1),
    ("001", 2),
    ("0001", 3),
    ("0000 11", 4),
    ("0000 101", 5),
    ("0000 100", 6),
    ("0000 011", 7),
    ("0000 0101 1", 8),
    ("0000 0101 0", 9),
    ("0000 0100 1", 10),
    ("0000 0100 01", 11),
    ("0000 0100 00", 12),
    ("0000 0011 11", 13),
    ("0000 0011 10", 14),
    ("0000 0011 01", 15),
    ("0000 0011 00", 16),
];

/// Table B.11 — dmvector.
pub(crate) const DMVECTOR: &[(&str, i8)] = &[("11", -1), ("0", 0), ("10", 1)];

/// Table B.12 — dct_dc_size_luminance.
pub(crate) const DC_SIZE_LUMA: &[(&str, u8)] = &[
    ("100", 0),
    ("00", 1),
    ("01", 2),
    ("101", 3),
    ("110", 4),
    ("1110", 5),
    ("1111 0", 6),
    ("1111 10", 7),
    ("1111 110", 8),
    ("1111 1110", 9),
    ("1111 1111 0", 10),
    ("1111 1111 1", 11),
];

/// Table B.13 — dct_dc_size_chrominance.
pub(crate) const DC_SIZE_CHROMA: &[(&str, u8)] = &[
    ("00", 0),
    ("01", 1),
    ("10", 2),
    ("110", 3),
    ("1110", 4),
    ("1111 0", 5),
    ("1111 10", 6),
    ("1111 110", 7),
    ("1111 1110", 8),
    ("1111 1111 0", 9),
    ("1111 1111 10", 10),
    ("1111 1111 11", 11),
];

/// End of block in Table B.14.
pub(crate) const EOB_ZERO: &str = "10";
/// End of block in Table B.15.
pub(crate) const EOB_ONE: &str = "0110";
/// Escape in both tables.
pub(crate) const ESCAPE: &str = "0000 01";

/// The run/level codes Tables B.14 and B.15 share (every code of 12 bits or
/// more except four in B.14's 12- and 13-bit groups, listed in
/// [`DCT_ZERO_ONLY`]), without the sign bit. `(code, run, level)`.
const DCT_COMMON: &[(&str, u8, u8)] = &[
    ("0000 0001 1100", 3, 3),
    ("0000 0001 0010", 4, 3),
    ("0000 0001 1110", 6, 2),
    ("0000 0001 0101", 7, 2),
    ("0000 0001 0001", 8, 2),
    ("0000 0001 1111", 17, 1),
    ("0000 0001 1010", 18, 1),
    ("0000 0001 1001", 19, 1),
    ("0000 0001 0111", 20, 1),
    ("0000 0001 0110", 21, 1),
    ("0000 0000 1011 0", 1, 6),
    ("0000 0000 1010 1", 1, 7),
    ("0000 0000 1010 0", 2, 5),
    ("0000 0000 1001 1", 3, 4),
    ("0000 0000 1001 0", 5, 3),
    ("0000 0000 1000 1", 9, 2),
    ("0000 0000 1000 0", 10, 2),
    ("0000 0000 1111 1", 22, 1),
    ("0000 0000 1111 0", 23, 1),
    ("0000 0000 1110 1", 24, 1),
    ("0000 0000 1110 0", 25, 1),
    ("0000 0000 1101 1", 26, 1),
    ("0000 0000 0111 11", 0, 16),
    ("0000 0000 0111 10", 0, 17),
    ("0000 0000 0111 01", 0, 18),
    ("0000 0000 0111 00", 0, 19),
    ("0000 0000 0110 11", 0, 20),
    ("0000 0000 0110 10", 0, 21),
    ("0000 0000 0110 01", 0, 22),
    ("0000 0000 0110 00", 0, 23),
    ("0000 0000 0101 11", 0, 24),
    ("0000 0000 0101 10", 0, 25),
    ("0000 0000 0101 01", 0, 26),
    ("0000 0000 0101 00", 0, 27),
    ("0000 0000 0100 11", 0, 28),
    ("0000 0000 0100 10", 0, 29),
    ("0000 0000 0100 01", 0, 30),
    ("0000 0000 0100 00", 0, 31),
    ("0000 0000 0011 000", 0, 32),
    ("0000 0000 0010 111", 0, 33),
    ("0000 0000 0010 110", 0, 34),
    ("0000 0000 0010 101", 0, 35),
    ("0000 0000 0010 100", 0, 36),
    ("0000 0000 0010 011", 0, 37),
    ("0000 0000 0010 010", 0, 38),
    ("0000 0000 0010 001", 0, 39),
    ("0000 0000 0010 000", 0, 40),
    ("0000 0000 0011 111", 1, 8),
    ("0000 0000 0011 110", 1, 9),
    ("0000 0000 0011 101", 1, 10),
    ("0000 0000 0011 100", 1, 11),
    ("0000 0000 0011 011", 1, 12),
    ("0000 0000 0011 010", 1, 13),
    ("0000 0000 0011 001", 1, 14),
    ("0000 0000 0001 0011", 1, 15),
    ("0000 0000 0001 0010", 1, 16),
    ("0000 0000 0001 0001", 1, 17),
    ("0000 0000 0001 0000", 1, 18),
    ("0000 0000 0001 0100", 6, 3),
    ("0000 0000 0001 1010", 11, 2),
    ("0000 0000 0001 1001", 12, 2),
    ("0000 0000 0001 1000", 13, 2),
    ("0000 0000 0001 0111", 14, 2),
    ("0000 0000 0001 0110", 15, 2),
    ("0000 0000 0001 0101", 16, 2),
    ("0000 0000 0001 1111", 27, 1),
    ("0000 0000 0001 1110", 28, 1),
    ("0000 0000 0001 1101", 29, 1),
    ("0000 0000 0001 1100", 30, 1),
    ("0000 0000 0001 1011", 31, 1),
];

/// Table B.14 rows not in [`DCT_COMMON`]. The first row, `11` (run 0, level 1),
/// is the form for every coefficient but the first of a non-intra block,
/// which uses `1` instead (Notes 3 and 4 of the table).
const DCT_ZERO_ONLY: &[(&str, u8, u8)] = &[
    ("11", 0, 1),
    ("011", 1, 1),
    ("0100", 0, 2),
    ("0101", 2, 1),
    ("0010 1", 0, 3),
    ("0011 1", 3, 1),
    ("0011 0", 4, 1),
    ("0001 10", 1, 2),
    ("0001 11", 5, 1),
    ("0001 01", 6, 1),
    ("0001 00", 7, 1),
    ("0000 110", 0, 4),
    ("0000 100", 2, 2),
    ("0000 111", 8, 1),
    ("0000 101", 9, 1),
    ("0010 0110", 0, 5),
    ("0010 0001", 0, 6),
    ("0010 0101", 1, 3),
    ("0010 0100", 3, 2),
    ("0010 0111", 10, 1),
    ("0010 0011", 11, 1),
    ("0010 0010", 12, 1),
    ("0010 0000", 13, 1),
    ("0000 0010 10", 0, 7),
    ("0000 0011 00", 1, 4),
    ("0000 0010 11", 2, 3),
    ("0000 0011 11", 4, 2),
    ("0000 0010 01", 5, 2),
    ("0000 0011 10", 14, 1),
    ("0000 0011 01", 15, 1),
    ("0000 0010 00", 16, 1),
    ("0000 0001 1101", 0, 8),
    ("0000 0001 1000", 0, 9),
    ("0000 0001 0011", 0, 10),
    ("0000 0001 0000", 0, 11),
    ("0000 0001 1011", 1, 5),
    ("0000 0001 0100", 2, 4),
    ("0000 0000 1101 0", 0, 12),
    ("0000 0000 1100 1", 0, 13),
    ("0000 0000 1100 0", 0, 14),
    ("0000 0000 1011 1", 0, 15),
];

/// Table B.15 rows not in [`DCT_COMMON`].
const DCT_ONE_ONLY: &[(&str, u8, u8)] = &[
    ("10", 0, 1),
    ("010", 1, 1),
    ("110", 0, 2),
    ("0010 1", 2, 1),
    ("0111", 0, 3),
    ("0011 1", 3, 1),
    ("0001 10", 4, 1),
    ("0011 0", 1, 2),
    ("0001 11", 5, 1),
    ("0000 110", 6, 1),
    ("0000 100", 7, 1),
    ("1110 0", 0, 4),
    ("0000 111", 2, 2),
    ("0000 101", 8, 1),
    ("1111 000", 9, 1),
    ("1110 1", 0, 5),
    ("0001 01", 0, 6),
    ("1111 001", 1, 3),
    ("0010 0110", 3, 2),
    ("1111 010", 10, 1),
    ("0010 0001", 11, 1),
    ("0010 0101", 12, 1),
    ("0010 0100", 13, 1),
    ("0001 00", 0, 7),
    ("0010 0111", 1, 4),
    ("1111 1100", 2, 3),
    ("1111 1101", 4, 2),
    ("0000 0010 0", 5, 2),
    ("0000 0010 1", 14, 1),
    ("0000 0011 1", 15, 1),
    ("0000 0011 01", 16, 1),
    ("1111 011", 0, 8),
    ("1111 100", 0, 9),
    ("0010 0011", 0, 10),
    ("0010 0010", 0, 11),
    ("0010 0000", 1, 5),
    ("0000 0011 00", 2, 4),
    ("1111 1010", 0, 12),
    ("1111 1011", 0, 13),
    ("1111 1110", 0, 14),
    ("1111 1111", 0, 15),
];

/// All run/level rows of Table B.14 (`11` form for run 0 level 1).
pub(crate) fn dct_table_zero() -> impl Iterator<Item = &'static (&'static str, u8, u8)> {
    DCT_ZERO_ONLY.iter().chain(DCT_COMMON.iter())
}

/// All run/level rows of Table B.15.
pub(crate) fn dct_table_one() -> impl Iterator<Item = &'static (&'static str, u8, u8)> {
    DCT_ONE_ONLY.iter().chain(DCT_COMMON.iter())
}

/// Figure 7-2: `scan[0][v][u]`, the scan index of each raster position.
pub(crate) const SCAN_ZIGZAG_RASTER: [u8; 64] = [
    0, 1, 5, 6, 14, 15, 27, 28, //
    2, 4, 7, 13, 16, 26, 29, 42, //
    3, 8, 12, 17, 25, 30, 41, 43, //
    9, 11, 18, 24, 31, 40, 44, 53, //
    10, 19, 23, 32, 39, 45, 52, 54, //
    20, 22, 33, 38, 46, 51, 55, 60, //
    21, 34, 37, 47, 50, 56, 59, 61, //
    35, 36, 48, 49, 57, 58, 62, 63,
];

/// Figure 7-3: `scan[1][v][u]`, the alternate scan.
pub(crate) const SCAN_ALTERNATE_RASTER: [u8; 64] = [
    0, 4, 6, 20, 22, 36, 38, 52, //
    1, 5, 7, 21, 23, 37, 39, 53, //
    2, 8, 19, 24, 34, 40, 50, 54, //
    3, 9, 18, 25, 35, 41, 51, 55, //
    10, 17, 26, 30, 42, 46, 56, 60, //
    11, 16, 27, 31, 43, 47, 57, 61, //
    12, 15, 28, 32, 44, 48, 58, 62, //
    13, 14, 29, 33, 45, 49, 59, 63,
];

const fn invert(raster_to_scan: &[u8; 64]) -> [u8; 64] {
    let mut out = [0u8; 64];
    let mut i = 0;
    while i < 64 {
        out[raster_to_scan[i] as usize] = i as u8;
        i += 1;
    }
    out
}

/// `SCAN[alternate_scan][n]`: the raster position (`v * 8 + u`) of scan index n.
pub(crate) const SCAN: [[u8; 64]; 2] =
    [invert(&SCAN_ZIGZAG_RASTER), invert(&SCAN_ALTERNATE_RASTER)];

/// 6.3.11: the default intra quantiser matrix, raster order.
pub(crate) const DEFAULT_INTRA_MATRIX: [u8; 64] = [
    8, 16, 19, 22, 26, 27, 29, 34, //
    16, 16, 22, 24, 27, 29, 34, 37, //
    19, 22, 26, 27, 29, 34, 34, 38, //
    22, 22, 26, 27, 29, 34, 37, 40, //
    22, 26, 27, 29, 32, 35, 40, 48, //
    26, 27, 29, 32, 35, 40, 48, 58, //
    26, 27, 29, 34, 38, 46, 56, 69, //
    27, 29, 35, 38, 46, 56, 69, 83,
];

/// 6.3.11: the default non-intra quantiser matrix (all 16).
pub(crate) const DEFAULT_NON_INTRA_MATRIX: [u8; 64] = [16; 64];

/// Table 7-6, `q_scale_type` = 1 (non-linear), indexed by quantiser_scale_code
/// (index 0 is forbidden).
pub(crate) const NON_LINEAR_QSCALE: [u8; 32] = [
    0, 1, 2, 3, 4, 5, 6, 7, 8, 10, 12, 14, 16, 18, 20, 22, 24, 28, 32, 36, 40, 44, 48, 52, 56, 64,
    72, 80, 88, 96, 104, 112,
];

/// quantiser_scale for a quantiser_scale_code (Table 7-6).
#[inline]
pub(crate) fn quantiser_scale(q_scale_type: bool, code: u8) -> i32 {
    if q_scale_type { i32::from(NON_LINEAR_QSCALE[code as usize & 31]) } else { 2 * i32::from(code) }
}

/// Table 6-4: frame_rate_value as a fraction, by frame_rate_code (1..=8).
pub(crate) fn frame_rate_value(code: u8) -> Option<(u32, u32)> {
    Some(match code {
        1 => (24000, 1001),
        2 => (24, 1),
        3 => (25, 1),
        4 => (30000, 1001),
        5 => (30, 1),
        6 => (50, 1),
        7 => (60000, 1001),
        8 => (60, 1),
        _ => return None,
    })
}

/// Parses a printed code (`"0000 01"`) into `(bits, length)`.
pub(crate) fn parse_code(s: &str) -> (u32, u32) {
    let mut v = 0u32;
    let mut n = 0u32;
    for c in s.bytes() {
        match c {
            b'0' | b'1' => {
                v = (v << 1) | u32::from(c - b'0');
                n += 1;
            }
            b' ' => {}
            _ => panic!("bad code {s}"),
        }
    }
    (v, n)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The Kraft sum of a code set, in units of 2^-24.
    fn kraft(codes: impl Iterator<Item = (u32, u32)>) -> u64 {
        codes.map(|(_, n)| 1u64 << (24 - n)).sum()
    }

    /// No code is a prefix of another.
    fn prefix_free(codes: &[(u32, u32)]) {
        for (i, &(a, la)) in codes.iter().enumerate() {
            for (j, &(b, lb)) in codes.iter().enumerate() {
                if i != j && la <= lb {
                    assert_ne!(b >> (lb - la), a, "code {a:0la$b} is a prefix of {b:0lb$b}",
                        la = la as usize, lb = lb as usize);
                }
            }
        }
    }

    #[test]
    fn macroblock_address_increment_is_prefix_free_and_complete() {
        let mut codes: Vec<_> = MB_ADDRESS_INCREMENT.iter().map(|r| parse_code(r.0)).collect();
        codes.push(MB_ESCAPE);
        codes.push(MB_STUFFING);
        prefix_free(&codes);
        // 1..=33 in order.
        for (i, r) in MB_ADDRESS_INCREMENT.iter().enumerate() {
            assert_eq!(r.1 as usize, i + 1);
        }
    }

    #[test]
    fn macroblock_type_tables_are_complete_prefix_codes() {
        for (t, missing) in [(MB_TYPE_I, 1u64 << 22), (MB_TYPE_P, 1 << 18), (MB_TYPE_B, 1 << 18)] {
            let codes: Vec<_> = t.iter().map(|r| parse_code(r.0)).collect();
            prefix_free(&codes);
            // Each table leaves exactly the all-zeros code of its longest
            // length unused.
            assert_eq!(kraft(codes.iter().copied()) + missing, 1 << 24);
        }
    }

    #[test]
    fn coded_block_pattern_covers_0_to_63_once() {
        let codes: Vec<_> = CODED_BLOCK_PATTERN.iter().map(|r| parse_code(r.0)).collect();
        prefix_free(&codes);
        let mut seen = [false; 64];
        for r in CODED_BLOCK_PATTERN {
            assert!(!seen[r.1 as usize]);
            seen[r.1 as usize] = true;
        }
        assert!(seen.iter().all(|&s| s));
        // Only `0000 0000 0` is unused.
        assert_eq!(kraft(codes.into_iter()) + (1 << 15), 1 << 24);
    }

    #[test]
    fn motion_code_mirrors_the_address_increment_codes() {
        // The two tables share codewords: with its sign bit, B.10's code for
        // +m is B.1's code for increment 2m + 1, and -m's is 2m's. A
        // transcription slip in either table breaks the relation.
        for m in 1..=16u32 {
            let (c, n) = parse_code(MOTION_CODE[m as usize].0);
            let pos = (c << 1, n + 1);
            let neg = ((c << 1) | 1, n + 1);
            assert_eq!(parse_code(MB_ADDRESS_INCREMENT[(2 * m) as usize].0), pos, "+{m}");
            assert_eq!(parse_code(MB_ADDRESS_INCREMENT[(2 * m - 1) as usize].0), neg, "-{m}");
        }
    }

    #[test]
    fn dc_size_tables_are_complete() {
        for t in [DC_SIZE_LUMA, DC_SIZE_CHROMA] {
            let codes: Vec<_> = t.iter().map(|r| parse_code(r.0)).collect();
            prefix_free(&codes);
            assert_eq!(kraft(codes.into_iter()), 1 << 24);
            for (i, r) in t.iter().enumerate() {
                assert_eq!(r.1 as usize, i);
            }
        }
    }

    fn check_dct(table: Vec<(u32, u32, u8, u8)>, eob: &str) {
        // 111 run/level pairs, each once.
        assert_eq!(table.len(), 111);
        let mut pairs: Vec<_> = table.iter().map(|r| (r.2, r.3)).collect();
        pairs.sort();
        pairs.dedup();
        assert_eq!(pairs.len(), 111);
        let mut codes: Vec<_> = table.iter().map(|r| (r.0, r.1)).collect();
        codes.push(parse_code(eob));
        codes.push(parse_code(ESCAPE));
        prefix_free(&codes);
        // Codes are followed by a sign bit except EOB and escape; the tree is
        // full apart from the all-zero prefix `0000 0000 0000`.
        let k: u64 = codes.iter().map(|&(_, n)| 1u64 << (24 - n)).sum();
        assert_eq!(k + (1 << 12), 1 << 24);
    }

    #[test]
    fn dct_table_zero_is_a_complete_prefix_code() {
        let t: Vec<_> = dct_table_zero()
            .map(|r| {
                let (c, n) = parse_code(r.0);
                (c, n, r.1, r.2)
            })
            .collect();
        check_dct(t, EOB_ZERO);
    }

    #[test]
    fn dct_table_one_is_a_complete_prefix_code_with_six_codes_unused() {
        let t: Vec<_> = dct_table_one()
            .map(|r| {
                let (c, n) = parse_code(r.0);
                (c, n, r.1, r.2)
            })
            .collect();
        assert_eq!(t.len(), 111);
        let mut codes: Vec<_> = t.iter().map(|r| (r.0, r.1)).collect();
        codes.push(parse_code(EOB_ONE));
        codes.push(parse_code(ESCAPE));
        prefix_free(&codes);
        // B.15 reassigns B.14's (0,8..11), (1,5), (2,4) (12-bit) and
        // (0,12..15) (13-bit) to shorter codes and leaves those ten codewords
        // unused: 6 × 2^-12 + 4 × 2^-13.
        let k: u64 = codes.iter().map(|&(_, n)| 1u64 << (24 - n)).sum();
        assert_eq!(k + (1 << 12) + 6 * (1 << 12) + 4 * (1 << 11), 1 << 24);
        let mut pairs: Vec<_> = t.iter().map(|r| (r.2, r.3)).collect();
        pairs.sort();
        pairs.dedup();
        assert_eq!(pairs.len(), 111);
        // Both tables code the same run/level pairs.
        let mut zero: Vec<_> = dct_table_zero().map(|r| (r.1, r.2)).collect();
        zero.sort();
        assert_eq!(zero, pairs);
    }

    #[test]
    fn scans_are_permutations_and_zigzag_is_a_zigzag() {
        for s in SCAN {
            let mut seen = [false; 64];
            for &p in &s {
                assert!(!seen[p as usize]);
                seen[p as usize] = true;
            }
        }
        // Rebuild the zigzag from its definition: anti-diagonals, alternating
        // direction, starting rightwards along the top row.
        let mut zz = Vec::new();
        for d in 0..15i32 {
            let mut diag: Vec<(i32, i32)> =
                (0..8).filter_map(|v| { let u = d - v; (0..8).contains(&u).then_some((v, u)) }).collect();
            if d % 2 == 0 {
                diag.reverse(); // v decreasing: up and to the right
            }
            zz.extend(diag.into_iter().map(|(v, u)| (v * 8 + u) as u8));
        }
        assert_eq!(zz.as_slice(), &SCAN[0][..]);
        // The alternate scan starts down the first column.
        assert_eq!(&SCAN[1][..8], &[0, 8, 16, 24, 1, 9, 2, 10]);
    }

    #[test]
    fn non_linear_qscale_is_increasing() {
        for i in 2..32 {
            assert!(NON_LINEAR_QSCALE[i] > NON_LINEAR_QSCALE[i - 1]);
        }
        assert_eq!(quantiser_scale(false, 31), 62);
        assert_eq!(quantiser_scale(true, 31), 112);
    }
}
