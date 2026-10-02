//! Variable length code decoding and encoding over the tables of
//! [`crate::tables`].
//!
//! Each table is decoded through one direct lookup indexed by the next
//! `max_len` bits (at most 16, Table B.14's longest code without its sign),
//! built once on first use.

use crate::bits::{BitReader, BitWriter};
use crate::tables::{self, parse_code};
use std::sync::OnceLock;

/// A direct-lookup decoder for one prefix code.
pub(crate) struct Vlc {
    bits: u32,
    /// `(value, length)` by the next `bits` bits; length 0 is no codeword.
    table: Vec<(i16, u8)>,
}

impl Vlc {
    fn build(entries: impl IntoIterator<Item = (u32, u32, i16)>) -> Vlc {
        let entries: Vec<_> = entries.into_iter().collect();
        let bits = entries.iter().map(|e| e.1).max().unwrap_or(1);
        let mut table = vec![(0i16, 0u8); 1 << bits];
        for (code, len, value) in entries {
            let shift = bits - len;
            let base = (code << shift) as usize;
            for slot in &mut table[base..base + (1 << shift)] {
                debug_assert_eq!(slot.1, 0, "overlapping codes");
                *slot = (value, len as u8);
            }
        }
        Vlc { bits, table }
    }

    fn from_table<T: Copy + Into<i16>>(t: &[(&str, T)]) -> Vlc {
        Vlc::build(t.iter().map(|&(s, v)| {
            let (c, n) = parse_code(s);
            (c, n, v.into())
        }))
    }

    /// Decodes one codeword; `None` if the bits match none.
    #[inline]
    pub(crate) fn decode(&self, r: &mut BitReader) -> Option<i16> {
        let (v, len) = self.table[r.peek(self.bits) as usize];
        if len == 0 {
            return None;
        }
        r.skip(u32::from(len));
        Some(v)
    }
}

/// Decoded DCT VLC: end of block.
pub(crate) const DCT_EOB: i16 = -1;
/// Decoded DCT VLC: escape.
pub(crate) const DCT_ESCAPE: i16 = -2;

/// Packs a run/level pair as a [`Vlc`] value.
const fn run_level(run: u8, level: u8) -> i16 {
    ((run as i16) << 8) | level as i16
}

/// The decoders, built on first use.
pub(crate) struct Decoders {
    pub(crate) mb_address_increment: Vlc,
    pub(crate) mb_type: [Vlc; 3],
    pub(crate) cbp: Vlc,
    pub(crate) motion_code: Vlc,
    pub(crate) dmvector: Vlc,
    pub(crate) dc_size: [Vlc; 2],
    pub(crate) dct: [Vlc; 2],
}

/// Value of macroblock_escape in the address-increment decoder.
pub(crate) const MB_ESCAPE: i16 = 100;
/// Value of MPEG-1 macroblock_stuffing in the address-increment decoder.
pub(crate) const MB_STUFFING: i16 = 101;

pub(crate) fn decoders() -> &'static Decoders {
    static D: OnceLock<Decoders> = OnceLock::new();
    D.get_or_init(|| {
        let mut mba: Vec<(u32, u32, i16)> = tables::MB_ADDRESS_INCREMENT
            .iter()
            .map(|&(s, v)| {
                let (c, n) = parse_code(s);
                (c, n, i16::from(v))
            })
            .collect();
        mba.push((tables::MB_ESCAPE.0, tables::MB_ESCAPE.1, MB_ESCAPE));
        mba.push((tables::MB_STUFFING.0, tables::MB_STUFFING.1, MB_STUFFING));

        // motion_code: magnitude codes plus the sign bit (none for 0).
        let mut mc = vec![(1u32, 1u32, 0i16)];
        for &(s, m) in &tables::MOTION_CODE[1..] {
            let (c, n) = parse_code(s);
            mc.push((c << 1, n + 1, i16::from(m)));
            mc.push(((c << 1) | 1, n + 1, -i16::from(m)));
        }

        let dct = |rows: Vec<&(&str, u8, u8)>, eob: &str| {
            let mut e: Vec<(u32, u32, i16)> = rows
                .into_iter()
                .map(|&(s, run, level)| {
                    let (c, n) = parse_code(s);
                    (c, n, run_level(run, level))
                })
                .collect();
            let (c, n) = parse_code(eob);
            e.push((c, n, DCT_EOB));
            let (c, n) = parse_code(tables::ESCAPE);
            e.push((c, n, DCT_ESCAPE));
            Vlc::build(e)
        };

        Decoders {
            mb_address_increment: Vlc::build(mba),
            mb_type: [
                Vlc::from_table(tables::MB_TYPE_I),
                Vlc::from_table(tables::MB_TYPE_P),
                Vlc::from_table(tables::MB_TYPE_B),
            ],
            cbp: Vlc::from_table(tables::CODED_BLOCK_PATTERN),
            motion_code: Vlc::build(mc),
            dmvector: Vlc::from_table(tables::DMVECTOR),
            dc_size: [Vlc::from_table(tables::DC_SIZE_LUMA), Vlc::from_table(tables::DC_SIZE_CHROMA)],
            dct: [
                dct(tables::dct_table_zero().collect(), tables::EOB_ZERO),
                dct(tables::dct_table_one().collect(), tables::EOB_ONE),
            ],
        }
    })
}

/// Splits a decoded DCT value into `(run, level)`.
#[inline]
pub(crate) fn split_run_level(v: i16) -> (usize, i32) {
    ((v >> 8) as usize, i32::from(v & 0xff))
}

/// `[table][run][level]` → `(code, length)`, codes without the sign bit.
type DctCodes = [[[(u32, u32); 41]; 32]; 2];

/// `(code, length)` lookups for the encoder.
pub(crate) struct Encoders {
    /// Index = increment − 1 (1..=33).
    mb_address_increment: [(u32, u32); 33],
    /// By macroblock_type flag byte, per picture type I, P, B.
    mb_type: [[(u32, u32); 32]; 3],
    cbp: [(u32, u32); 64],
    /// By magnitude 0..=16, without the sign bit.
    motion_code: [(u32, u32); 17],
    dc_size: [[(u32, u32); 12]; 2],
    /// `[table][run][level]`, codes without the sign bit; length 0 = none.
    dct: Box<DctCodes>,
}

pub(crate) fn encoders() -> &'static Encoders {
    static E: OnceLock<Encoders> = OnceLock::new();
    E.get_or_init(|| {
        let mut e = Encoders {
            mb_address_increment: [(0, 0); 33],
            mb_type: [[(0, 0); 32]; 3],
            cbp: [(0, 0); 64],
            motion_code: [(0, 0); 17],
            dc_size: [[(0, 0); 12]; 2],
            dct: Box::new([[[(0, 0); 41]; 32]; 2]),
        };
        for &(s, v) in tables::MB_ADDRESS_INCREMENT {
            e.mb_address_increment[v as usize - 1] = parse_code(s);
        }
        for (i, t) in [tables::MB_TYPE_I, tables::MB_TYPE_P, tables::MB_TYPE_B].iter().enumerate() {
            for &(s, v) in t.iter() {
                e.mb_type[i][v as usize] = parse_code(s);
            }
        }
        for &(s, v) in tables::CODED_BLOCK_PATTERN {
            e.cbp[v as usize] = parse_code(s);
        }
        for &(s, v) in tables::MOTION_CODE {
            e.motion_code[v as usize] = parse_code(s);
        }
        for (i, t) in [tables::DC_SIZE_LUMA, tables::DC_SIZE_CHROMA].iter().enumerate() {
            for &(s, v) in t.iter() {
                e.dc_size[i][v as usize] = parse_code(s);
            }
        }
        for &(s, run, level) in tables::dct_table_zero() {
            e.dct[0][run as usize][level as usize] = parse_code(s);
        }
        for &(s, run, level) in tables::dct_table_one() {
            e.dct[1][run as usize][level as usize] = parse_code(s);
        }
        e
    })
}

impl Encoders {
    /// Writes a macroblock_address_increment (≥ 1), with escapes.
    pub(crate) fn put_mb_address_increment(&self, w: &mut BitWriter, mut inc: u32) {
        while inc > 33 {
            w.put(tables::MB_ESCAPE.1, tables::MB_ESCAPE.0);
            inc -= 33;
        }
        let (c, n) = self.mb_address_increment[inc as usize - 1];
        w.put(n, c);
    }

    /// Writes macroblock_type; `pic` 0 I, 1 P, 2 B.
    pub(crate) fn put_mb_type(&self, w: &mut BitWriter, pic: usize, flags: u8) {
        let (c, n) = self.mb_type[pic][flags as usize];
        debug_assert!(n > 0, "macroblock_type {flags:#07b} not codable in picture type {pic}");
        w.put(n, c);
    }

    pub(crate) fn put_cbp(&self, w: &mut BitWriter, cbp: u8) {
        let (c, n) = self.cbp[cbp as usize];
        w.put(n, c);
    }

    /// Writes a motion_code (−16..=16).
    pub(crate) fn put_motion_code(&self, w: &mut BitWriter, code: i32) {
        let (c, n) = self.motion_code[code.unsigned_abs() as usize];
        w.put(n, c);
        if code != 0 {
            w.put_bit(code < 0);
        }
    }

    /// Writes dct_dc_size and dct_dc_differential for a DC difference.
    pub(crate) fn put_dc(&self, w: &mut BitWriter, chroma: bool, diff: i32) {
        let size = 32 - diff.unsigned_abs().leading_zeros();
        let (c, n) = self.dc_size[usize::from(chroma)][size as usize];
        w.put(n, c);
        if size > 0 {
            let v = if diff > 0 { diff } else { diff + (1 << size) - 1 };
            w.put(size, v as u32);
        }
    }

    /// Bits of the VLC for a run/level pair (with its sign bit), or `None`
    /// when it needs an escape.
    #[cfg(test)]
    pub(crate) fn dct_len(&self, table: usize, run: usize, level: i32) -> Option<u32> {
        let a = level.unsigned_abs() as usize;
        if run < 32 && a <= 40 {
            let (_, n) = self.dct[table][run][a];
            if n > 0 {
                return Some(n + 1);
            }
        }
        None
    }

    /// Writes one run/level pair of a block: the table's VLC with its sign,
    /// or the H.262 escape (6-bit run, 12-bit signed level). `first_non_intra`
    /// selects the `1s` form of Table B.14 for run 0, level ±1.
    pub(crate) fn put_dct(
        &self,
        w: &mut BitWriter,
        table: usize,
        run: usize,
        level: i32,
        first_non_intra: bool,
    ) {
        debug_assert!(level != 0 && (-2047..=2047).contains(&level));
        if first_non_intra && run == 0 && level.abs() == 1 {
            w.put(2, 0b10 | u32::from(level < 0));
            return;
        }
        let a = level.unsigned_abs() as usize;
        if run < 32 && a <= 40 {
            let (c, n) = self.dct[table][run][a];
            if n > 0 {
                w.put(n, c);
                w.put_bit(level < 0);
                return;
            }
        }
        let (c, n) = parse_code(tables::ESCAPE);
        w.put(n, c);
        w.put(6, run as u32);
        w.put(12, (level as u32) & 0xfff);
    }

    /// Writes the end-of-block code of a table.
    pub(crate) fn put_eob(&self, w: &mut BitWriter, table: usize) {
        if table == 0 {
            w.put(2, 0b10);
        } else {
            w.put(4, 0b0110);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_dct_code_decodes_to_its_own_pair() {
        let d = decoders();
        let e = encoders();
        for table in 0..2 {
            for run in 0..32 {
                for level in 1..=40 {
                    for sign in [1, -1] {
                        let lv = level * sign;
                        if e.dct_len(table, run, lv).is_none() {
                            continue;
                        }
                        let mut w = BitWriter::new();
                        e.put_dct(&mut w, table, run, lv, false);
                        let bytes = w.finish();
                        let mut r = BitReader::new(&bytes);
                        let v = d.dct[table].decode(&mut r).unwrap();
                        let (r2, l2) = split_run_level(v);
                        let l2 = if r.read_bit() { -l2 } else { l2 };
                        assert_eq!((r2, l2), (run, lv));
                    }
                }
            }
        }
    }

    #[test]
    fn motion_codes_round_trip() {
        let d = decoders();
        let e = encoders();
        for m in -16..=16 {
            let mut w = BitWriter::new();
            e.put_motion_code(&mut w, m);
            let b = w.finish();
            assert_eq!(d.motion_code.decode(&mut BitReader::new(&b)), Some(m as i16));
        }
    }

    #[test]
    fn dc_differentials_round_trip() {
        let e = encoders();
        let d = decoders();
        for chroma in [false, true] {
            for diff in -2047..=2047 {
                let mut w = BitWriter::new();
                e.put_dc(&mut w, chroma, diff);
                let b = w.finish();
                let mut r = BitReader::new(&b);
                let size = d.dc_size[usize::from(chroma)].decode(&mut r).unwrap() as u32;
                let got = if size == 0 {
                    0
                } else {
                    let v = r.read(size) as i32;
                    if v >= 1 << (size - 1) { v } else { v + 1 - (1 << size) }
                };
                assert_eq!(got, diff);
            }
        }
    }

    #[test]
    fn mb_address_increments_round_trip_with_escapes() {
        let e = encoders();
        let d = decoders();
        for inc in 1..200u32 {
            let mut w = BitWriter::new();
            e.put_mb_address_increment(&mut w, inc);
            let b = w.finish();
            let mut r = BitReader::new(&b);
            let mut total = 0;
            loop {
                match d.mb_address_increment.decode(&mut r).unwrap() {
                    MB_ESCAPE => total += 33,
                    v => {
                        total += v as u32;
                        break;
                    }
                }
            }
            assert_eq!(total, inc);
        }
    }
}
