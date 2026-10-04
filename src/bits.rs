//! MSB-first bit reading and writing.
//!
//! The reader works on one start-code unit (the bytes between two start
//! codes) and reads zeros past its end, so a decoder can peek the 23 zero bits
//! that end a slice, or a VLC near the end of the data, without bounds checks
//! at every call site; [`BitReader::overrun`] tells it afterwards whether it
//! read data that was not there.

/// Reads bits, most significant first, from a byte slice.
pub(crate) struct BitReader<'a> {
    data: &'a [u8],
    /// Bit position from the start of `data`.
    pos: usize,
}

impl<'a> BitReader<'a> {
    pub(crate) fn new(data: &'a [u8]) -> Self {
        BitReader { data, pos: 0 }
    }

    /// The next 32 bits, zero-filled past the end, without consuming them.
    #[inline]
    pub(crate) fn peek32(&self) -> u32 {
        let byte = self.pos >> 3;
        let shift = self.pos & 7;
        let v: u64 = if byte + 5 <= self.data.len() {
            let b = &self.data[byte..byte + 5];
            (u64::from(b[0]) << 32)
                | (u64::from(b[1]) << 24)
                | (u64::from(b[2]) << 16)
                | (u64::from(b[3]) << 8)
                | u64::from(b[4])
        } else {
            let mut v = 0u64;
            for i in 0..5 {
                v = (v << 8) | u64::from(self.data.get(byte + i).copied().unwrap_or(0));
            }
            v
        };
        // 40 bits loaded; drop the `shift` already consumed, keep 32.
        ((v << shift) >> 8) as u32
    }

    /// The next `n` bits (0..=32) without consuming them.
    #[inline]
    pub(crate) fn peek(&self, n: u32) -> u32 {
        if n == 0 { 0 } else { self.peek32() >> (32 - n) }
    }

    #[inline]
    pub(crate) fn skip(&mut self, n: u32) {
        self.pos += n as usize;
    }

    /// Reads `n` bits (0..=32).
    #[inline]
    pub(crate) fn read(&mut self, n: u32) -> u32 {
        let v = self.peek(n);
        self.pos += n as usize;
        v
    }

    #[inline]
    pub(crate) fn read_bit(&mut self) -> bool {
        self.read(1) != 0
    }

    /// Bits left before the end of the data (zero once past it).
    pub(crate) fn bits_left(&self) -> usize {
        (self.data.len() * 8).saturating_sub(self.pos)
    }

    /// Whether more bits have been consumed than the data holds.
    pub(crate) fn overrun(&self) -> bool {
        self.pos > self.data.len() * 8
    }
}

/// Writes bits, most significant first.
#[derive(Default)]
pub(crate) struct BitWriter {
    out: Vec<u8>,
    acc: u64,
    /// Bits held in `acc` (always < 8 between calls).
    n: u32,
}

impl BitWriter {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Writes the low `n` bits (0..=32) of `v`.
    #[inline]
    pub(crate) fn put(&mut self, n: u32, v: u32) {
        debug_assert!(n <= 32);
        if n == 0 {
            return;
        }
        let v = u64::from(v) & ((1u64 << n) - 1);
        self.acc = (self.acc << n) | v;
        self.n += n;
        while self.n >= 8 {
            self.n -= 8;
            self.out.push((self.acc >> self.n) as u8);
        }
        self.acc &= (1u64 << self.n) - 1;
    }

    #[inline]
    pub(crate) fn put_bit(&mut self, b: bool) {
        self.put(1, u32::from(b));
    }

    /// Pads with zero bits to the next byte boundary.
    pub(crate) fn align(&mut self) {
        if self.n > 0 {
            let pad = 8 - self.n;
            self.put(pad, 0);
        }
    }

    /// Byte-aligns and writes a start code `00 00 01 code`.
    pub(crate) fn start_code(&mut self, code: u8) {
        self.align();
        self.out.extend_from_slice(&[0, 0, 1, code]);
    }

    /// Byte-aligns and then appends everything `other` holds, its last
    /// partial byte included: afterwards this writer is exactly as if
    /// `other`'s bits had been written to it directly after the alignment.
    pub(crate) fn append(&mut self, other: BitWriter) {
        self.align();
        self.out.extend_from_slice(&other.out);
        self.acc = other.acc;
        self.n = other.n;
    }

    /// Bits written so far.
    pub(crate) fn bit_len(&self) -> usize {
        self.out.len() * 8 + self.n as usize
    }

    /// Byte-aligns and returns the bytes.
    pub(crate) fn finish(mut self) -> Vec<u8> {
        self.align();
        self.out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_fields() {
        let mut w = BitWriter::new();
        let fields: &[(u32, u32)] = &[
            (1, 1),
            (3, 5),
            (12, 0xabc),
            (32, 0xdead_beef),
            (7, 0x55),
            (2, 2),
        ];
        for &(n, v) in fields {
            w.put(n, v);
        }
        let bytes = w.finish();
        let mut r = BitReader::new(&bytes);
        for &(n, v) in fields {
            assert_eq!(r.read(n), v);
        }
        assert!(!r.overrun());
        r.read(32);
        assert!(r.overrun());
    }

    #[test]
    fn peek_past_end_reads_zeros() {
        let r = BitReader::new(&[0xff]);
        assert_eq!(r.peek32(), 0xff00_0000);
    }
}
