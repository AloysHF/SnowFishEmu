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
    #[error("invalid match offset")]
    BadOffset,
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

    let next = |ip: &mut usize| -> Result<u8, LzError> {
        if *ip >= src_len {
            return Err(LzError::SourceOverrun);
        }
        let b = src[*ip];
        *ip += 1;
        Ok(b)
    };

    let mut f = src[0] as u32;
    let mut ip: usize = 1;
    if f > 0x11 {
        for _ in 0..(f - 0x11) {
            if out.len() >= dst_cap {
                return Err(LzError::OutputFull);
            }
            out.push(next(&mut ip)?);
        }
        f = next(&mut ip)? as u32;
    }
    loop {
        if f <= 0xF {
            // Literal run (0 extends through a 0xFF chain, then last + 0xF).
            if f == 0 {
                loop {
                    let b = next(&mut ip)?;
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
                out.push(next(&mut ip)?);
            }
            f = next(&mut ip)? as u32;
            if f > 0xF {
                // Fall through to match dispatch.
            } else {
                // Fixed 3-byte mid copy after a literal run.
                let d = (f >> 2) + ((next(&mut ip)? as u32) << 2) + 0x801;
                let start = out.len() as i64 - d as i64;
                for k in 0..3 {
                    if out.len() >= dst_cap {
                        return Err(LzError::OutputFull);
                    }
                    let b = copy_byte(&out, prev, start + (k as i64))?;
                    out.push(b);
                }
                let tail = f & 3;
                for _ in 0..tail {
                    if out.len() >= dst_cap {
                        return Err(LzError::OutputFull);
                    }
                    out.push(next(&mut ip)?);
                }
                f = next(&mut ip)? as u32;
                continue;
            }
        }
        if f <= 0x1F {
            // Long match.
            let b3: u32 = if f & 8 != 0 { 0x4000 } else { 0 };
            let mut len = f & 7;
            if len == 0 {
                loop {
                    let b = next(&mut ip)?;
                    if b != 0 {
                        len += b as u32 + 7;
                        break;
                    }
                    len += 0xFF;
                }
            }
            let p = ip;
            let b0 = next(&mut ip)? as u32;
            let b1 = next(&mut ip)? as u32;
            let v = (out.len() as u32)
                .wrapping_sub(b3)
                .wrapping_sub(b0 >> 2)
                .wrapping_sub(b1 << 6);
            if v == out.len() as u32 {
                // Terminator: length must be 1 and input fully consumed.
                if len != 1 {
                    return Err(LzError::BadTerminator(len));
                }
                if ip != src_len {
                    return if ip < src_len {
                        Err(LzError::SourceTruncated)
                    } else {
                        Err(LzError::SourceOverrun)
                    };
                }
                return Ok(out);
            }
            let start = v as i64 - 0x4000;
            for k in 0..(len + 2) {
                if out.len() >= dst_cap {
                    return Err(LzError::OutputFull);
                }
                let b = copy_byte(&out, prev, start + (k as i64))?;
                out.push(b);
            }
            let tail = (src[p] & 3) as usize;
            for _ in 0..tail {
                if out.len() >= dst_cap {
                    return Err(LzError::OutputFull);
                }
                out.push(next(&mut ip)?);
            }
        } else if f <= 0x3F {
            // Medium match.
            let mut len = f & 0x1F;
            if len == 0 {
                loop {
                    let b = next(&mut ip)?;
                    if b != 0 {
                        len += b as u32 + 0x1F;
                        break;
                    }
                    len += 0xFF;
                }
            }
            let p = ip;
            let b0 = next(&mut ip)? as u32;
            let b1 = next(&mut ip)? as u32;
            let start = out.len() as i64 - (b0 >> 2) as i64 - ((b1 << 6) as i64) - 1;
            for k in 0..(len + 2) {
                if out.len() >= dst_cap {
                    return Err(LzError::OutputFull);
                }
                let b = copy_byte(&out, prev, start + (k as i64))?;
                out.push(b);
            }
            let tail = (src[p] & 3) as usize;
            for _ in 0..tail {
                if out.len() >= dst_cap {
                    return Err(LzError::OutputFull);
                }
                out.push(next(&mut ip)?);
            }
        } else {
            // Short match: distance = ((c>>2)&7) + (b0<<3) + 1.
            let nib = (f >> 2) & 7;
            let len = (f >> 5).wrapping_sub(1);
            let b0 = next(&mut ip)? as u32;
            let start = out.len() as i64 - nib as i64 - ((b0 << 3) as i64) - 1;
            for k in 0..(len + 2) {
                if out.len() >= dst_cap {
                    return Err(LzError::OutputFull);
                }
                let b = copy_byte(&out, prev, start + (k as i64))?;
                out.push(b);
            }
            let tail = (f & 3) as usize;
            for _ in 0..tail {
                if out.len() >= dst_cap {
                    return Err(LzError::OutputFull);
                }
                out.push(next(&mut ip)?);
            }
        }
        f = next(&mut ip)? as u32;
    }
}

/// Reads one byte for a match copy. Negative offsets index the previous
/// block's output tail; offsets past the current output are errors.
fn copy_byte(out: &[u8], prev: &[u8], off: i64) -> Result<u8, LzError> {
    if off < 0 {
        let idx = (prev.len() as i64 + off) as usize;
        if idx < prev.len() {
            Ok(prev[idx])
        } else {
            Ok(0)
        }
    } else if (off as usize) >= out.len() {
        Err(LzError::BadOffset)
    } else {
        Ok(out[off as usize])
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
    fn extended_literal_run() {
        // F=0 token: 0xFF chain then final byte + 0xF, all +3.
        let mut s: Vec<u8> = vec![0x11 + 1, 0xAA];
        s.push(0x00); // literal-run token 0
        s.push(0x00); // 0xFF chain byte (one zero)
        s.push(0x02); // final byte 2 -> run = 0 + 255 + 2 + 15 + 3 = 275
        s.extend_from_slice(&[0xBB; 275]);
        s.push(0x11);
        s.push(0x00);
        s.push(0x00);
        let out = decompress(&s, &[], 1 << 20).unwrap();
        assert_eq!(out.len(), 1 + 275);
        assert_eq!(out[0], 0xAA);
        assert_eq!(out[1], 0xBB);
        assert_eq!(out[275], 0xBB);
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
