//! SnowFish LZ stream decompression (platform slot +0x108).
//!
//! Exact port of the platform's `lz_decompress` state machine (validated
//! against every lz call the reference session logged: all blocks decode to
//! their observed `out_len`). Two dispatch states drive the token stream:
//!
//! * **dispatch** — `F > 0x3F` short match (X), `0x20..=0x3F` medium match
//!   (A2), `F <= 0xF` near copy (MidA), `0x10..=0x1F` far match (W)
//! * **litcheck** — `F > 0xF` back to dispatch; `F <= 0xF` a literal run of
//!   `F + 3` bytes (`F == 0` extends through a 0xFF chain then `last + 0xF`);
//!   the following `F <= 0xF` is a far 3-byte copy (MidB)
//!
//! Token anatomy (`ip` = index of the first byte after the token byte; the
//! offset bytes are read *without* advancing `ip`; `p` = index past the last
//! offset byte):
//!
//! * W: `b0=src[ip]`, `b1=src[ip+1]`, `p=ip+2`; length `(F&7)` (0 extends
//!   through a 0xFF chain then `last + 7`) + 2; distance `0x4000*(F&8?1:0) +
//!   (b0>>2) + (b1<<6) + 0x4000`; `v = out_len - distance` equal to `out_len`
//!   is the end-of-stream token (then `p == src_len` and length field 1 are
//!   required); trailing literals `b0 & 3`
//! * A2: same layout, `p=ip+2`; length `(F&0x1F)` (0 extends, then
//!   `last + 0x1F`) + 2; distance `(b0>>2) + (b1<<6) + 1`; trailing `b0 & 3`
//! * X: `ob=src[ip]`, `p=ip+1`; length `((F>>5) - 1) + 2`; distance
//!   `((F&0x1C)>>2) + (ob<<3) + 1`; trailing `F & 3`
//! * MidA: `ob=src[ip]`, `p=ip+1`; fixed 2-byte copy at distance
//!   `(F>>2) + (ob<<2) + 1`; trailing `F & 3`
//! * MidB: `ob=src[ip]`, `p=ip+1`; fixed 3-byte copy at distance
//!   `(F>>2) + (ob<<2) + 1 + 0x801`; trailing `F & 3`
//!
//! After every match the `F&3`-style trailing literal count is read from
//! `src[p-2]` (the token's first offset byte, or the token byte itself for
//! one-offset-byte classes), the trailing bytes are emitted from `src[p..]`,
//! and the next token byte is `src[p + count]` — read sequentially. The next
//! state is **dispatch** when trailing literals were emitted, **litcheck**
//! when none were. Init: `F > 0x11` emits `F - 0x11` literals then requires
//! `F > 0xF`; `F <= 0x11` enters litcheck directly.
//!
//! Matches copy byte-by-byte from the output buffer, so overlapping ranges
//! replicate. A computed start offset below zero is the `-6` error (blocks
//! are self-contained; cross-block references do not occur).

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
/// Returns the decompressed bytes. `dst_cap` bounds the output exactly as
/// the platform's destination buffer would.
pub fn decompress(src: &[u8], dst_cap: usize) -> Result<Vec<u8>, LzError> {
    let src_len = src.len();
    if src_len == 0 {
        return Err(LzError::InvalidToken);
    }
    let mut out: Vec<u8> = Vec::with_capacity(16 * 1024);
    let mut ip: usize;
    let mut f: u32;

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

    // Emits `n` literals from the stream (per-byte capacity check).
    macro_rules! emit_literals {
        ($n:expr) => {{
            for _ in 0..($n) {
                if out.len() >= dst_cap {
                    return Err(LzError::OutputFull);
                }
                out.push(next!());
            }
        }};
    }

    // Byte-by-byte copy from the output (overlapping ranges replicate).
    macro_rules! copy {
        ($start:expr, $count:expr, $ip_at:expr) => {{
            let start: i64 = $start;
            if start < 0 {
                return Err(LzError::BadOffset {
                    off: start,
                    out_len: out.len(),
                    ip: $ip_at,
                });
            }
            for k in 0..($count) {
                if out.len() >= dst_cap {
                    return Err(LzError::OutputFull);
                }
                let idx = (start + k as i64) as usize;
                if idx >= out.len() {
                    return Err(LzError::BadOffset {
                        off: idx as i64,
                        out_len: out.len(),
                        ip: $ip_at,
                    });
                }
                let b = out[idx];
                out.push(b);
            }
        }};
    }

    // Emits `cnt` trailing literals starting at `p` (the post-offset marker).
    macro_rules! emit_trailing {
        ($p:expr, $cnt:expr) => {{
            for k in 0..($cnt) {
                if ($p) + k >= src_len {
                    return Err(LzError::SourceOverrun);
                }
                if out.len() >= dst_cap {
                    return Err(LzError::OutputFull);
                }
                out.push(src[($p) + k]);
            }
        }};
    }

    /// Shared match tail: trailing literals from `p`, then the next token
    /// byte read sequentially; dispatches to the next state.
    macro_rules! match_tail {
        ($p:expr, $cnt:expr) => {{
            let p: usize = $p;
            let cnt: usize = $cnt;
            emit_trailing!(p, cnt);
            if p + cnt >= src_len {
                return Err(LzError::SourceOverrun);
            }
            f = src[p + cnt] as u32;
            ip = p + cnt + 1;
            if cnt > 0 {
                State::Dispatch
            } else {
                State::LitCheck
            }
        }};
    }

    enum State {
        Dispatch,
        LitCheck,
    }

    // --- init ---
    f = src[0] as u32;
    ip = 1;
    let state = if f > 0x11 {
        emit_literals!((f - 0x11) as usize);
        f = next!() as u32;
        if f <= 0xF {
            return Err(LzError::InvalidToken);
        }
        State::Dispatch
    } else {
        State::LitCheck
    };

    let mut state = state;
    loop {
        match state {
            State::Dispatch => {
                if f > 0x3F {
                    // X: short match, one offset byte
                    let ip_at = ip;
                    let ob = *src.get(ip).ok_or(LzError::SourceOverrun)? as u32;
                    let p = ip + 1;
                    let nib = ((f & 0x1C) >> 2) as i64;
                    let ln = f.checked_shr(5).unwrap_or(0).wrapping_sub(1);
                    let start = out.len() as i64 - nib - ((ob << 3) as i64) - 1;
                    copy!(start, ln + 2, ip_at);
                    let cnt = (f & 3) as usize;
                    state = match_tail!(p, cnt);
                } else if f > 0x1F {
                    // A2: medium match, two offset bytes
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
                    let ip_at = ip;
                    let b0 = *src.get(ip).ok_or(LzError::SourceOverrun)? as u32;
                    let b1 = *src.get(ip + 1).ok_or(LzError::SourceOverrun)? as u32;
                    let p = ip + 2;
                    let start = out.len() as i64 - ((b0 >> 2) as i64) - ((b1 << 6) as i64) - 1;
                    copy!(start, ln + 2, ip_at);
                    let cnt = (b0 & 3) as usize;
                    state = match_tail!(p, cnt);
                } else if f <= 0xF {
                    // MidA: near 2-byte copy, one offset byte
                    let ip_at = ip;
                    let ob = *src.get(ip).ok_or(LzError::SourceOverrun)? as u32;
                    let p = ip + 1;
                    let start = out.len() as i64 - ((f >> 2) as i64) - ((ob << 2) as i64) - 1;
                    copy!(start, 2, ip_at);
                    let cnt = (f & 3) as usize;
                    state = match_tail!(p, cnt);
                } else {
                    // W: far match, two offset bytes; also the terminator
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
                    let ip_at = ip;
                    let b0 = *src.get(ip).ok_or(LzError::SourceOverrun)? as u32;
                    let b1 = *src.get(ip + 1).ok_or(LzError::SourceOverrun)? as u32;
                    let p = ip + 2;
                    let v = (out.len() as u32)
                        .wrapping_sub(b3)
                        .wrapping_sub(b0 >> 2)
                        .wrapping_sub(b1 << 6);
                    if v == out.len() as u32 {
                        // end-of-stream token: source must be exactly consumed
                        if p < src_len {
                            return Err(LzError::SourceTruncated);
                        }
                        if p > src_len {
                            return Err(LzError::SourceOverrun);
                        }
                        if ln != 1 {
                            return Err(LzError::BadTerminator(ln));
                        }
                        return Ok(out);
                    }
                    let start = (v as i32).wrapping_sub(0x4000) as i64;
                    copy!(start, ln + 2, ip_at);
                    let cnt = (b0 & 3) as usize;
                    state = match_tail!(p, cnt);
                }
            }
            State::LitCheck => {
                if f > 0xF {
                    state = State::Dispatch;
                    continue;
                }
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
                emit_literals!((f + 3) as usize);
                f = next!() as u32;
                if f > 0xF {
                    state = State::Dispatch;
                    continue;
                }
                // MidB: far 3-byte copy after a literal run
                let ip_at = ip;
                let ob = *src.get(ip).ok_or(LzError::SourceOverrun)? as u32;
                let p = ip + 1;
                let start = out.len() as i64 - ((f >> 2) as i64) - ((ob << 2) as i64) - 1 - 0x801;
                copy!(start, 3, ip_at);
                let cnt = (f & 3) as usize;
                emit_trailing!(p, cnt);
                if p + cnt >= src_len {
                    return Err(LzError::SourceOverrun);
                }
                f = src[p + cnt] as u32;
                ip = p + cnt + 1;
                state = if cnt > 0 {
                    State::Dispatch
                } else {
                    State::LitCheck
                };
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CAP: usize = 1 << 20;

    #[test]
    fn init_literal_run_and_terminator() {
        // init run of 5 literals, then the zero-distance W terminator.
        let s: Vec<u8> = [0x11 + 5, b'h', b'e', b'l', b'l', b'o', 0x11, 0x00, 0x00].to_vec();
        let out = decompress(&s, CAP).unwrap();
        assert_eq!(out, b"hello");
    }

    #[test]
    fn init_small_f_enters_litcheck() {
        // Init F <= 0x11 goes to the litcheck state: F <= 0xF is a literal
        // run of F+3 bytes, then the terminator.
        let s: Vec<u8> = [0x02, b'a', b'b', b'c', b'd', b'e', 0x11, 0x00, 0x00].to_vec();
        let out = decompress(&s, CAP).unwrap();
        assert_eq!(out, b"abcde");
    }

    #[test]
    fn x_short_match_overlapping_copy() {
        // init "ABC", then X copies 3 bytes from distance 3 (nib=2, ob=0).
        let s: Vec<u8> = [0x14, b'A', b'B', b'C', 0x48, 0x00, 0x11, 0x00, 0x00].to_vec();
        let out = decompress(&s, CAP).unwrap();
        assert_eq!(out, b"ABCABC");
    }

    #[test]
    fn a2_match_then_mida_via_trailing_literals() {
        // init "ABCD"; A2 copies 3 from distance 1..4 (b0=13 -> dist 4,
        // trailing 1); the next token F<=0xF after trailing literals is a
        // MidA 2-byte near copy; then the terminator.
        let s: Vec<u8> = [
            0x15, b'A', b'B', b'C', b'D', // init: out="ABCD"
            0x21, 0x0D, 0x00, // A2: dist=4, len 3 -> "ABC"; trailing 1
            0x45, // trailing literal 'E' -> out="ABCDABCE"
            0x0E, 0x01, // MidA: dist=8-(3)-(4)-1=0, copy 2 -> "AB"
            0x58, 0x59, // MidA trailing literals 'X','Y'
            0x11, 0x00, 0x00, // terminator
        ]
        .to_vec();
        let out = decompress(&s, CAP).unwrap();
        assert_eq!(out, b"ABCDABCEABXY");
    }

    #[test]
    fn post_match_zero_trailing_enters_literal_run() {
        // After a match with no trailing literals, F<=0xF is a literal run
        // of F+3 (NOT a MidA copy).
        let s: Vec<u8> = [
            0x15, b'A', b'B', b'C', b'D', // init: out="ABCD"
            0x21, 0x0C, 0x00, // A2: dist=4, len 3 -> "ABC", trailing 0
            0x01, b'W', b'X', b'Y', b'Z', // literal run of 4
            0x11, 0x00, 0x00, // terminator
        ]
        .to_vec();
        let out = decompress(&s, CAP).unwrap();
        assert_eq!(out, b"ABCDABCWXYZ");
    }

    #[test]
    fn midb_far_copy_after_literal_run() {
        // Grow output past 0x801 with A2 copies, then a literal run whose
        // following F<=0xF token is a MidB 3-byte copy at distance >=0x801.
        let mut s: Vec<u8> = vec![0x11 + 238]; // init run of 238 literals
        s.extend(std::iter::repeat_n(0x41u8, 238));
        // A2 groups: F=0x3F (len 33), b0=0x10 (dist 5, no trailing).
        while s.len() < 238 + 62 * 3 {
            s.extend_from_slice(&[0x3F, 0x10, 0x00]);
        }
        // After the last copy (no trailing) we are in litcheck: literal run
        // of 4, then F<=0xF is a MidB copy.
        s.extend_from_slice(&[0x01, b'w', b'x', b'y', b'z']);
        s.extend_from_slice(&[0x04, 0x30]); // MidB: dist = 1 + 192 + 1 + 0x801
        s.extend_from_slice(&[0x11, 0x00, 0x00]); // terminator
        let out = decompress(&s, CAP).unwrap();
        // The MidB token starts when out = 238 + 62*33 + 4 = 2288; distance
        // = 1 + (0x30<<2=192) + 1 + 0x801 = 2243 -> start = 2288 - 2243 = 45.
        assert_eq!(out.len(), 2288 + 3);
        assert_eq!(&out[2288..2291], &out[45..48]);
        assert_eq!(&out[2284..2288], b"wxyz");
        assert_eq!(&out[0..3], b"AAA");
    }

    #[test]
    fn literal_run_zero_extends_through_0xff_chain() {
        // Post-match literal run with F=0 extends through a 0xFF chain:
        // F += b + 0xF, run = F + 3.
        let s: Vec<u8> = [
            0x14, b'A', b'B', b'C', // init 3 literals
            0x48, 0x00, // X match: copy 3 (out=6), trailing 0
            0x00, 0x14, // chain: F = 0x14 + 0xF = 0x23 -> run of 0x26
            // 0x26 = 38 literals, then the terminator
            0x11, 0x00, 0x00,
        ]
        .to_vec();
        // 38 literals needed before the terminator:
        let mut s = s;
        // The literal run consumes 38 bytes; place them before the terminator
        // by rebuilding: [init][X][chain][38 lits][term]
        s.clear();
        s.extend_from_slice(&[0x14, b'A', b'B', b'C', 0x48, 0x00, 0x00, 0x14]);
        s.extend(std::iter::repeat_n(b'q', 38));
        s.extend_from_slice(&[0x11, 0x00, 0x00]);
        let out = decompress(&s, CAP).unwrap();
        assert_eq!(out.len(), 3 + 3 + 38);
        assert_eq!(&out[0..6], b"ABCABC");
        assert_eq!(&out[6..44], &[b'q'; 38]);
    }

    #[test]
    fn rejects_unconsumed_input_after_terminator() {
        // A valid terminator followed by trailing bytes must report the
        // unconsumed remainder (-8 source truncated).
        let s = [0x11 + 1, 0xAA, 0x11, 0x00, 0x00, 0x99];
        match decompress(&s, CAP) {
            Err(LzError::SourceTruncated) => {}
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn rejects_bad_terminator_length() {
        // Zero-distance W token whose length field is not 1 is rejected.
        let s = [0x11 + 1, 0xAA, 0x12, 0x00, 0x00];
        match decompress(&s, CAP) {
            Err(LzError::BadTerminator(2)) => {}
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn rejects_match_offset_before_output_start() {
        // A match reaching before the output start is the -6 error (blocks
        // are self-contained).
        let s = [0x14, b'A', b'B', b'C', 0x4C, 0x00, 0x11, 0x00, 0x00];
        match decompress(&s, CAP) {
            Err(LzError::BadOffset { .. }) => {}
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn output_capacity_is_enforced() {
        // 300 literals against a 100-byte cap must fail with OutputFull.
        let s: Vec<u8> = {
            let mut v = vec![0x11 + 238];
            v.extend(std::iter::repeat_n(0x41u8, 238));
            v.extend_from_slice(&[0x0F]); // literal run of 0x12 = 18
            v.extend(std::iter::repeat_n(0x42u8, 18));
            v.extend_from_slice(&[0x0F]);
            v.extend(std::iter::repeat_n(0x43u8, 18));
            v.extend_from_slice(&[0x11, 0x00, 0x00]);
            v
        };
        match decompress(&s, 100) {
            Err(LzError::OutputFull) => {}
            other => panic!("unexpected: {other:?}"),
        }
    }
}
