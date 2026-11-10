//! `.app` package format parsing and relocation processing.
//!
//! An `.app` file is a flat ARM image with a fixed header:
//!
//! ```text
//! offset  size  field
//! 0x00    4     magic: 0x19414653 ("SFA\x19") or 0x19414654 ("TFA\x19")
//! 0x04    4     version (0 or 1)
//! 0x08    4     reserved (0)
//! 0x0C    4     reserved (0)
//! 0x10    4     header length (0x20 for SFA, 0x24 for TFA)
//! 0x14    4     code segment size (c0)
//! 0x18    4     bss segment size (c1)
//! 0x1C    4     relocation table entry count
//! ```
//!
//! The code segment follows the header at `header_len`, and the relocation
//! table (one little-endian `u32` image offset per entry) follows the code
//! segment. Each relocation adds [`crate::consts::CODE_BASE`] to the word at
//! that image offset, because absolute pointers in the image are stored
//! relative to the load address.

use crate::consts::CODE_BASE;
use thiserror::Error;

/// Magic accepted for `SFA` images (bytes `"SFA\x19"`).
pub const MAGIC_SFA: u32 = 0x1941_4653;
/// Magic accepted for `TFA` images (bytes `"TFA\x19"`); uses a 0x24 header.
pub const MAGIC_TFA: u32 = 0x1941_4654;

/// Header size for `SFA` images.
pub const HEADER_LEN_SFA: u32 = 0x20;
/// Header size for `TFA` images.
pub const HEADER_LEN_TFA: u32 = 0x24;

/// `.app` parsing and relocation errors.
#[derive(Debug, Error)]
pub enum SfaError {
    #[error("file too small: {0} bytes")]
    TooSmall(usize),
    #[error("bad magic 0x{0:08X}")]
    BadMagic(u32),
    #[error("bad version {0}")]
    BadVersion(u32),
    #[error("bad reserved fields {0:#x}/{1:#x}")]
    BadReserved(u32, u32),
    #[error("bad header length {0:#x}")]
    BadHeaderLen(u32),
    #[error("file truncated: len={0} need={1}")]
    Truncated(u32, u32),
    #[error("relocation offset {1:#x} (entry {0}) out of image")]
    RelocOutOfRange(u32, u32),
}

/// Parsed `.app` header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SfaHeader {
    /// Raw magic value.
    pub magic: u32,
    /// Version field (0 or 1).
    pub version: u32,
    /// Header length in bytes (0x20 or 0x24).
    pub header_len: u32,
    /// Code segment size in bytes.
    pub code_size: u32,
    /// BSS segment size in bytes (zero-initialized at load time).
    pub bss_size: u32,
    /// Number of relocation entries.
    pub reloc_count: u32,
}

impl SfaHeader {
    /// Total image size mapped into guest memory (code + bss).
    pub fn image_size(&self) -> u32 {
        self.code_size + self.bss_size
    }

    /// File offset of the relocation table.
    pub fn reloc_offset(&self) -> u32 {
        self.header_len + self.code_size
    }
}

/// A parsed and relocated `.app` image.
#[derive(Debug, Clone)]
pub struct SfaImage {
    /// Parsed header.
    pub header: SfaHeader,
    /// Code segment, with relocations applied.
    pub code: Vec<u8>,
    /// BSS segment size (the segment itself is zero-initialized at load).
    pub bss_size: u32,
}

impl SfaImage {
    /// Parse and relocate an `.app` file image.
    ///
    /// Relocations are applied against [`CODE_BASE`]; the returned code
    /// segment can be written verbatim at that address.
    pub fn parse(data: &[u8]) -> Result<Self, SfaError> {
        let header = parse_header(data)?;

        let reloc_off = header.reloc_offset() as usize;
        let reloc_end = reloc_off + header.reloc_count as usize * 4;
        if data.len() < reloc_end {
            return Err(SfaError::Truncated(
                data.len() as u32,
                header.header_len + header.code_size,
            ));
        }

        let mut code = data[header.header_len as usize..][..header.code_size as usize].to_vec();
        for i in 0..header.reloc_count as usize {
            let ro = u32::from_le_bytes([
                data[reloc_off + i * 4],
                data[reloc_off + i * 4 + 1],
                data[reloc_off + i * 4 + 2],
                data[reloc_off + i * 4 + 3],
            ]);
            let end = ro
                .checked_add(4)
                .ok_or(SfaError::RelocOutOfRange(i as u32, ro))?;
            if end as u64 > header.image_size() as u64 {
                return Err(SfaError::RelocOutOfRange(i as u32, ro));
            }
            let at = ro as usize;
            let v = u32::from_le_bytes([code[at], code[at + 1], code[at + 2], code[at + 3]]);
            code[at..at + 4].copy_from_slice(&(v.wrapping_add(CODE_BASE)).to_le_bytes());
        }

        Ok(Self {
            header,
            code,
            bss_size: header.bss_size,
        })
    }

    /// Total number of relocations applied.
    pub fn reloc_count(&self) -> u32 {
        self.header.reloc_count
    }
}

// Mirrors the reference loader's validation order: length, magic, version,
// reserved pair, header length, then total-size coverage.
fn parse_header(data: &[u8]) -> Result<SfaHeader, SfaError> {
    if data.len() <= 0x23 {
        return Err(SfaError::TooSmall(data.len()));
    }
    let rd = |o: usize| u32::from_le_bytes([data[o], data[o + 1], data[o + 2], data[o + 3]]);
    let header = SfaHeader {
        magic: rd(0x00),
        version: rd(0x04),
        header_len: rd(0x10),
        code_size: rd(0x14),
        bss_size: rd(0x18),
        reloc_count: rd(0x1C),
    };
    let res1 = rd(0x08);
    let res2 = rd(0x0C);

    if header.magic != MAGIC_SFA && header.magic != MAGIC_TFA {
        return Err(SfaError::BadMagic(header.magic));
    }
    if header.version != 0 && header.version != 1 {
        return Err(SfaError::BadVersion(header.version));
    }
    if res1 != 0 || res2 != 0 {
        return Err(SfaError::BadReserved(res1, res2));
    }
    let expected_len = match header.magic {
        MAGIC_SFA => HEADER_LEN_SFA,
        _ => HEADER_LEN_TFA,
    };
    if header.header_len != expected_len {
        return Err(SfaError::BadHeaderLen(header.header_len));
    }
    let need = header
        .header_len
        .wrapping_add(header.code_size)
        .wrapping_add(header.reloc_count.wrapping_mul(4));
    if (data.len() as u32) < need {
        return Err(SfaError::Truncated(data.len() as u32, need));
    }
    Ok(header)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::consts::CODE_BASE;

    fn build(magic: u32, version: u32, code: &[u8], relocs: &[u32]) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(&magic.to_le_bytes());
        v.extend_from_slice(&version.to_le_bytes());
        v.extend_from_slice(&0u32.to_le_bytes());
        v.extend_from_slice(&0u32.to_le_bytes());
        v.extend_from_slice(&(HEADER_LEN_SFA).to_le_bytes());
        v.extend_from_slice(&(code.len() as u32).to_le_bytes());
        v.extend_from_slice(&0u32.to_le_bytes()); // bss
        v.extend_from_slice(&(relocs.len() as u32).to_le_bytes());
        v.extend_from_slice(code);
        for r in relocs {
            v.extend_from_slice(&r.to_le_bytes());
        }
        v
    }

    #[test]
    fn parses_and_relocates() {
        // Word at image offset 0 holds 0x100; relocation adds CODE_BASE.
        let mut code = 0x100u32.to_le_bytes().to_vec();
        code.extend_from_slice(&[0xAA, 0xBB, 0xCC, 0xDD]);
        let data = build(MAGIC_SFA, 0, &code, &[0]);
        let img = SfaImage::parse(&data).unwrap();
        assert_eq!(img.header.magic, MAGIC_SFA);
        assert_eq!(img.header.version, 0);
        assert_eq!(img.header.code_size, 8);
        assert_eq!(img.header.bss_size, 0);
        assert_eq!(img.reloc_count(), 1);
        assert_eq!(
            &img.code[0..4],
            &CODE_BASE.wrapping_add(0x100).to_le_bytes()
        );
        assert_eq!(&img.code[4..8], &[0xAA, 0xBB, 0xCC, 0xDD]);
    }

    #[test]
    fn rejects_bad_magic() {
        let data = build(0x1234_5678, 0, &[0u8; 8], &[]);
        match SfaImage::parse(&data) {
            Err(SfaError::BadMagic(m)) => assert_eq!(m, 0x1234_5678),
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn rejects_bad_version() {
        let data = build(MAGIC_SFA, 7, &[0u8; 8], &[]);
        assert!(matches!(
            SfaImage::parse(&data),
            Err(SfaError::BadVersion(7))
        ));
    }

    #[test]
    fn rejects_bad_reserved() {
        let mut data = build(MAGIC_SFA, 0, &[0u8; 8], &[]);
        data[0x08] = 1;
        assert!(matches!(
            SfaImage::parse(&data),
            Err(SfaError::BadReserved(1, 0))
        ));
    }

    #[test]
    fn rejects_bad_header_len() {
        let data = build(MAGIC_SFA, 0, &[0u8; 8], &[]);
        let mut d = data.clone();
        d[0x10] = 0x24;
        assert!(matches!(
            SfaImage::parse(&d),
            Err(SfaError::BadHeaderLen(0x24))
        ));
    }

    #[test]
    fn rejects_truncated() {
        let code = [0u8; 16];
        let data = build(MAGIC_SFA, 0, &code, &[0]);
        // Cut into the code segment.
        let cut = &data[..data.len() - 8];
        assert!(matches!(SfaImage::parse(cut), Err(SfaError::Truncated(..))));
    }

    #[test]
    fn rejects_relocation_out_of_range() {
        let code = [0u8; 8];
        let data = build(MAGIC_SFA, 0, &code, &[0x1000]);
        assert!(matches!(
            SfaImage::parse(&data),
            Err(SfaError::RelocOutOfRange(0, 0x1000))
        ));
    }

    #[test]
    fn tfa_variant_uses_extended_header() {
        // TFA declares a 0x24 header; the code segment starts 4 bytes later.
        let mut v = Vec::new();
        v.extend_from_slice(&MAGIC_TFA.to_le_bytes());
        v.extend_from_slice(&0u32.to_le_bytes());
        v.extend_from_slice(&0u32.to_le_bytes());
        v.extend_from_slice(&0u32.to_le_bytes());
        v.extend_from_slice(&(HEADER_LEN_TFA).to_le_bytes());
        v.extend_from_slice(&4u32.to_le_bytes());
        v.extend_from_slice(&0u32.to_le_bytes());
        v.extend_from_slice(&1u32.to_le_bytes()); // one relocation
        v.extend_from_slice(&[0u8; 4]); // extra header word
        v.extend_from_slice(&0x200u32.to_le_bytes()); // code word to relocate
        v.extend_from_slice(&0u32.to_le_bytes()); // relocation: image offset 0
        let img = SfaImage::parse(&v).unwrap();
        assert_eq!(img.header.header_len, HEADER_LEN_TFA);
        assert_eq!(
            &img.code[0..4],
            &CODE_BASE.wrapping_add(0x200).to_le_bytes()
        );
    }
}
