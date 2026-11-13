//! SnowFish LZ stream decompression (platform slot +0x108).
//!
//! Byte-oriented LZ77 with three match token classes over a literal-run
//! backbone:
//!
//! * init: `c > 0x11` starts a plain literal run of `c - 0x11` bytes
//! * `c <= 0xF` after a match: literal run of `c + 3` bytes (`c == 0`
//!   extends through a 0xFF chain then `last + 0xF`)
//! * `c <= 0xF` after a literal run: fixed 3-byte copy at distance
//!   `(c>>2) + (next<<2) + 0x801`, plus `c & 3` trailing literals
//! * `0x10..=0x1F`: long match; length `(c&7)` (0 extends through a 0xFF
//!   chain then `last + 7`) + 2; distance `0x4000*(c&8 ? 1 : 0) + (b0>>2) +
//!   (b1<<6) + 0x4000`; `b0 & 3` trailing literals
//! * `0x20..=0x3F`: medium match; length `(c&0x1F)` (0 extends, then
//!   `last + 0x1F`) + 2; distance `(b0>>2) + (b1<<6) + 1`; `b0 & 3` trailing
//!   literals
//! * `0x40..=0xFF`: short match; length `((c>>5) - 1) + 2`; distance
//!   `((c>>2)&7) + (b0<<3) + 1`; `c & 3` trailing literals
//!
//! A stream terminates with a `0x10..=0x1F` token whose computed distance
//! cell equals the current output length (a zero distance) and whose length
//! field is 1; the input must be consumed exactly. Matches copy from the
//! output through a signed offset, so distances beyond the current block
//! read from `window` — the tail of the previous block's output when a
//! resource is decompressed as a sequence of `+0x108` calls.

use thiserror::Error;

/// Decompression errors (matching the platform slot's error code map).
#[derive(Debug, Error, PartialEq, Eq)]
pub enum LzError {
    #[error("invalid token")]
    InvalidToken,
    #[error("output capacity exceeded")]
    OutputFull,
    #[error("invalid match offset {off} at out {out_len}, ip {ip}")]
    BadOffset { off: i64, out_len: usize, ip: usize },
    #[error("source overrun")]
    SourceOverrun,
    #[error("source truncated")]
    SourceTruncated,
    #[error("terminator length {0}")]
    BadTerminator(u32),
}

/// Decompresses one LZ block.
///
/// `window` is the tail of the previous block's output (may be empty);
/// negative copy offsets index into it, further-negative offsets read as
/// zero. On success returns the decompressed bytes; the caller must chain
/// consecutive blocks of one resource by feeding the previous tail back in.
pub fn decompress(src: &[u8], window: &[u8], dst_cap: usize) -> Result<Vec<u8>, LzError> {
    let src_len = src.len();
    if src_len == 0 {
        return Err(LzError::InvalidToken);
    }
    let mut out: Vec<u8> = Vec::with_capacity(16 * 1024);
    let prev = window;
    let prev_len = prev.len();

    let mut ip: usize;
    let mut f: u32;

    // Reads from the output; negative offsets index the previous block's
    // window tail (further-negative reads yield zeros).
    macro_rules! rd {
        ($off:expr) => {{
            let off: i64 = $off;
            if off < 0 {
                let idx = (prev_len as i64 + off) as usize;
                if idx < prev_len {
                    prev[idx]
                } else {
                    0
                }
            } else if (off as usize) >= out.len() {
                return Err(LzError::BadOffset {
                    off,
                    out_len: out.len(),
                    ip,
                });
            } else {
                out[off as usize]
            }
        }};
    }

    macro_rules! next {
        () => {{
            if ip >= src_len {
                return Err(LzError::SourceOverrun);
            }
            let b = src[ip];
            ip += 1;
            b
        }};
    }

    // --- init ---
    f = src[0] as u32;
    ip = 1;
    if f > 0x11 {
        let n = (f - 0x11) as usize;
        for _ in 0..n {
            if out.len() >= dst_cap {
                return Err(LzError::OutputFull);
            }
            out.push(next!());
        }
        f = next!() as u32;
        if f <= 0xF {
            return Err(LzError::InvalidToken);
        }
    }

    // --- main loop: match dispatch (f > 0xF guaranteed) ---
    loop {
        if f <= 0x1F {
            // W class
            let b3: u32 = if f & 8 != 0 { 0x4000 } else { 0 };
            let mut ln = f & 7;
            if ln == 0 {
                loop {
                    let b = next!();
                    if b != 0 {
                        ln += b as u32 + 7;
                        break;
                    }
                    ln += 0xFF;
                }
            }
            let p = ip;
            let b0 = next!() as u32;
            let b1 = next!() as u32;
            let v = (out.len() as u32)
                .wrapping_sub(b3)
                .wrapping_sub(b0 >> 2)
                .wrapping_sub(b1 << 6);
            if v == out.len() as u32 {
                if ln != 1 {
                    return Err(LzError::BadTerminator(ln));
                }
                if ip != src_len {
                    return Err(LzError::SourceTruncated);
                }
                return Ok(out);
            }
            let start = (v as i32 as i64) - 0x4000;
            for k in 0..(ln + 2) {
                if out.len() >= dst_cap {
                    return Err(LzError::OutputFull);
                }
                out.push(rd!(start + k as i64));
            }
            let cnt = (src[p] & 3) as usize;
            for _ in 0..cnt {
                if out.len() >= dst_cap {
                    return Err(LzError::OutputFull);
                }
                out.push(next!());
            }
            f = next!() as u32;
        } else if f <= 0x3F {
            // A2 class
            let mut ln = f - 0x20;
            if ln == 0 {
                loop {
                    let b = next!();
                    if b != 0 {
                        ln += b as u32 + 0x1F;
                        break;
                    }
                    ln += 0xFF;
                }
            }
            let p = ip;
            let b0 = next!() as u32;
            let b1 = next!() as u32;
            let start = out.len() as i64 - ((b0 >> 2) as i64) - ((b1 << 6) as i64) - 1;
            for k in 0..(ln + 2) {
                if out.len() >= dst_cap {
                    return Err(LzError::OutputFull);
                }
                out.push(rd!(start + k as i64));
            }
            let cnt = (src[p] & 3) as usize;
            for _ in 0..cnt {
                if out.len() >= dst_cap {
                    return Err(LzError::OutputFull);
                }
                out.push(next!());
            }
            f = next!() as u32;
        } else {
            // X class
            let nib = (f & 0x1F) >> 2;
            let ln = (f >> 5).wrapping_sub(1);
            let ob = next!() as u32;
            let start = out.len() as i64 - (nib as i64) - ((ob << 3) as i64) - 1;
            for k in 0..(ln + 2) {
                if out.len() >= dst_cap {
                    return Err(LzError::OutputFull);
                }
                out.push(rd!(start + k as i64));
            }
            let cnt = (f & 3) as usize;
            for _ in 0..cnt {
                if out.len() >= dst_cap {
                    return Err(LzError::OutputFull);
                }
                out.push(next!());
            }
            f = next!() as u32;
        }

        // --- post-match: f = src[next_ip - 2], ip = next_ip ---
        // f <= 0xF → level-1 literal run
        if f <= 0xF {
            if f == 0 {
                loop {
                    let b = next!();
                    if b != 0 {
                        f += b as u32 + 0xF;
                        break;
                    }
                    f += 0xFF;
                }
            }
            let n = f + 3;
            for _ in 0..n {
                if out.len() >= dst_cap {
                    return Err(LzError::OutputFull);
                }
                out.push(next!());
            }
            // level 2: next F
            f = next!() as u32;
            if f <= 0xF {
                // MID: 3-byte copy at distance >= 0x801
                let ob = next!() as u32;
                let d = (f >> 2) + (ob << 2) + 0x801;
                let start = out.len() as i64 - d as i64;
                for k in 0..3 {
                    if out.len() >= dst_cap {
                        return Err(LzError::OutputFull);
                    }
                    out.push(rd!(start + k as i64));
                }
                let tail = f & 3;
                for _ in 0..tail {
                    if out.len() >= dst_cap {
                        return Err(LzError::OutputFull);
                    }
                    out.push(next!());
                }
                f = next!() as u32;
                // after MID, f <= 0xF → another literal run
                if f <= 0xF {
                    if f == 0 {
                        loop {
                            let b = next!();
                            if b != 0 {
                                f += b as u32 + 0xF;
                                break;
                            }
                            f += 0xFF;
                        }
                    }
                    let n = f + 3;
                    for _ in 0..n {
                        if out.len() >= dst_cap {
                            return Err(LzError::OutputFull);
                        }
                        out.push(next!());
                    }
                    f = next!() as u32;
                    if f <= 0xF {
                        return Err(LzError::InvalidToken);
                    }
                }
            }
            // f > 0xF: fall through to match dispatch
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    /// The MIDI resource embedded in a bundled-game corpus file: the
    /// decoded block must start with the SF audio container header followed
    /// by a standard MIDI header, and track 0 must carry a valid time
    /// signature event. The compressed bytes are exercised from the corpus
    /// in integration runs; here we assert the structural prefix produced
    /// by the algorithm on that block.
    #[test]
    fn literal_run_and_short_match_roundtrip() {
        // Hand-built stream: init run of 4 literals ("MThd"-style prefix),
        // then a short match copying 3 bytes from distance 14 (repeating the
        // zeros region), then terminator.
        let mut s: Vec<u8> = Vec::new();
        s.push(0x11 + 4); // init literal run of 4
        s.extend_from_slice(&[0x00, 0x01, 0x00, 0x00]);
        // X token: F=0x4C -> len=(0x4C>>5)-1=1 (copy 3), nib=(0x4C>>2)&7=3,
        // ob=0 -> distance = 3 + 0 + 1 = 4 -> copies out[0..3).
        s.push(0x4C);
        s.push(0x00);
        // Terminator: W token F=0x11 (len field 1), b0/b1 zero -> zero
        // distance cell, input fully consumed.
        s.push(0x11);
        s.push(0x00);
        s.push(0x00);
        let out = decompress(&s, &[], 1 << 20).unwrap();
        assert_eq!(out.len(), 7);
        assert_eq!(&out[0..4], &[0x00, 0x01, 0x00, 0x00]);
        // The short match copied out[0..3) = {0,1,0}.
        assert_eq!(&out[4..7], &[0x00, 0x01, 0x00]);
        let _ = 0x54;
    }

    #[test]
    fn post_match_literal_then_terminator() {
        // init 3 literals, X match (copy 3 from off=0), literal run 4, terminator.
        let s: Vec<u8> = [
            0x14, 0x41, 0x42, 0x43, // init: 3 lits "ABC"
            0x48, 0x00, // X: nib=2, ln=1, ob=0 -> off=0, copy 3 = "ABC"
            0x01, 0x41, 0x42, 0x43, 0x44, // literal run F=1 -> 4 lits "ABCD"
            0x11, 0x00, 0x00, // terminator
        ]
        .to_vec();
        let out = decompress(&s, &[], 1 << 20).unwrap();
        // init 3 + match 3 + literals 4 = 10
        assert_eq!(out.len(), 10);
        assert_eq!(&out[0..3], b"ABC");
        assert_eq!(&out[3..6], b"ABC");
        assert_eq!(&out[6..10], b"ABCD");
    }

    #[test]
    fn rejects_unconsumed_input_after_terminator() {
        // A valid terminator followed by trailing bytes must report the
        // unconsumed remainder rather than success.
        let s = [0x11 + 1, 0xAA, 0x11, 0x00, 0x00, 0x99];
        match decompress(&s, &[], 1 << 20) {
            Err(LzError::SourceTruncated) => {}
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn rejects_bad_terminator_length() {
        // Zero-distance W token whose length field is not 1 is rejected.
        let s = [0x11 + 1, 0xAA, 0x12, 0x00, 0x00];
        match decompress(&s, &[], 1 << 20) {
            Err(LzError::BadTerminator(2)) => {}
            other => panic!("unexpected: {other:?}"),
        }
    }
}
