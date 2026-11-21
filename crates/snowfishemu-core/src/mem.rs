//! Guest memory map.
//!
//! The guest sees a handful of fixed service regions (code, stack, app data,
//! platform table, heap) with the code region sized from the loaded image.
//! [`GuestMem`] stores each region in a flat `Vec<u8>` and translates guest
//! addresses on access; unmapped accesses are errors the machine can trace.

use crate::consts::{
    APP_DATA_BASE, APP_DATA_SIZE, CODE_MAX_SIZE, CODE_REGION_BASE, HEAP_BASE, HEAP_SIZE, PLAT_BASE,
    PLAT_SIZE, STACK_BASE, STACK_SIZE,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Guest memory errors.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum MemError {
    #[error("unmapped guest address 0x{0:08X}")]
    Unmapped(u32),
    #[error("region layout mismatch")]
    LayoutMismatch,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Region {
    base: u32,
    data: Vec<u8>,
}

/// Guest address space.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GuestMem {
    regions: Vec<Region>,
    /// Watched guest addresses (`SF_WATCH`): the next write that touches
    /// one of them records the address in `watch_hit` for the machine's
    /// execution loop to attribute to the current instruction.
    pub watch: Vec<u32>,
    pub watch_hit: Option<u32>,
    /// Suppressed guest addresses (`SF_FREE_SCREEN`): writes from the CPU
    /// to these addresses are silently dropped while `suppress_active` is
    /// set. The machine's own writes bypass the check through
    /// `write_unsuppressed`.
    pub suppress: Vec<u32>,
    pub suppress_active: bool,
}

/// Rounds the image size up to a page with room for the 0x1000 reserved
/// head the code segment loads above (the loader maps image + 0x2000 slack).
fn page_round(v: u32) -> u32 {
    v.saturating_add(0x2FFF) & !0xFFF
}

impl GuestMem {
    /// Builds the memory map for an image of `image_size` bytes (code + bss).
    pub fn new(image_size: u32) -> Self {
        let code_size = page_round(image_size).min(CODE_MAX_SIZE);
        let regions = vec![
            Region {
                base: CODE_REGION_BASE,
                data: vec![0; code_size as usize],
            },
            Region {
                base: STACK_BASE,
                data: vec![0; STACK_SIZE as usize],
            },
            Region {
                base: APP_DATA_BASE,
                data: vec![0; APP_DATA_SIZE as usize],
            },
            Region {
                base: PLAT_BASE,
                data: vec![0; PLAT_SIZE as usize],
            },
            Region {
                base: HEAP_BASE,
                data: vec![0; HEAP_SIZE as usize],
            },
        ];
        Self {
            regions,
            watch: Vec::new(),
            watch_hit: None,
            suppress: Vec::new(),
            suppress_active: false,
        }
    }

    fn region(&self, addr: u32, len: u32) -> Result<&Region, MemError> {
        self.regions
            .iter()
            .find(|r| {
                let off = addr.wrapping_sub(r.base);
                off < r.data.len() as u32 && len <= r.data.len() as u32 - off
            })
            .ok_or(MemError::Unmapped(addr))
    }

    fn region_mut(&mut self, addr: u32, len: u32) -> Result<&mut Region, MemError> {
        let found = self.regions.iter().position(|r| {
            let off = addr.wrapping_sub(r.base);
            off < r.data.len() as u32 && len <= r.data.len() as u32 - off
        });
        match found {
            Some(i) => Ok(&mut self.regions[i]),
            None => Err(MemError::Unmapped(addr)),
        }
    }

    /// Bulk read from guest memory.
    pub fn read(&self, addr: u32, buf: &mut [u8]) -> Result<(), MemError> {
        let region = self.region(addr, buf.len() as u32)?;
        let off = (addr - region.base) as usize;
        buf.copy_from_slice(&region.data[off..off + buf.len()]);
        Ok(())
    }

    /// Bulk write into guest memory.
    /// Writes without checking the suppress list (the machine's own writes:
    /// SF_FORCE, the platform hooks, the state serialisation).
    pub fn write_unsuppressed(&mut self, addr: u32, data: &[u8]) -> Result<(), MemError> {
        let region = self.region_mut(addr, data.len() as u32)?;
        let off = (addr - region.base) as usize;
        region.data[off..off + data.len()].copy_from_slice(data);
        Ok(())
    }

    pub fn write(&mut self, addr: u32, data: &[u8]) -> Result<(), MemError> {
        let len = data.len() as u32;
        if self.suppress_active && !self.suppress.is_empty() {
            for &s in &self.suppress {
                if addr <= s && s < addr + len {
                    return Ok(());
                }
            }
        }
        if !self.watch.is_empty() && self.watch_hit.is_none() {
            for &w in &self.watch {
                if addr <= w && w < addr + len {
                    self.watch_hit = Some(w);
                    break;
                }
            }
        }
        let region = self.region_mut(addr, len)?;
        let off = (addr - region.base) as usize;
        region.data[off..off + data.len()].copy_from_slice(data);
        Ok(())
    }

    /// Reads a little-endian `u32`.
    pub fn read_u32(&self, addr: u32) -> Result<u32, MemError> {
        let mut b = [0u8; 4];
        self.read(addr, &mut b)?;
        Ok(u32::from_le_bytes(b))
    }

    /// Writes a little-endian `u32`.
    pub fn write_u32(&mut self, addr: u32, v: u32) -> Result<(), MemError> {
        self.write(addr, &v.to_le_bytes())
    }

    /// Reads a little-endian `u16`.
    pub fn read_u16(&self, addr: u32) -> Result<u16, MemError> {
        let mut b = [0u8; 2];
        self.read(addr, &mut b)?;
        Ok(u16::from_le_bytes(b))
    }

    /// Writes a little-endian `u16`.
    pub fn write_u16(&mut self, addr: u32, v: u16) -> Result<(), MemError> {
        self.write(addr, &v.to_le_bytes())
    }

    /// Reads a single byte.
    pub fn read_u8(&self, addr: u32) -> Result<u8, MemError> {
        let region = self.region(addr, 1)?;
        Ok(region.data[(addr - region.base) as usize])
    }

    /// Writes a single byte.
    pub fn write_u8(&mut self, addr: u32, v: u8) -> Result<(), MemError> {
        if self.watch.contains(&addr) && self.watch_hit.is_none() {
            self.watch_hit = Some(addr);
        }
        let region = self.region_mut(addr, 1)?;
        region.data[(addr - region.base) as usize] = v;
        Ok(())
    }

    /// Returns a direct pointer to `len` bytes of guest memory, when the
    /// span lies within a single region. Callers must keep borrow discipline.
    pub fn slice_mut(&mut self, addr: u32, len: u32) -> Result<&mut [u8], MemError> {
        let region = self.region_mut(addr, len)?;
        let off = (addr - region.base) as usize;
        Ok(&mut region.data[off..off + len as usize])
    }

    /// Returns a direct slice view of `len` bytes of guest memory.
    pub fn slice(&self, addr: u32, len: u32) -> Result<&[u8], MemError> {
        let region = self.region(addr, len)?;
        let off = (addr - region.base) as usize;
        Ok(&region.data[off..off + len as usize])
    }

    /// (base, len) of every mapped region, in address order. Read-only
    /// layout summary for tools and frontend memory registration.
    pub fn region_spans(&self) -> Vec<(u32, usize)> {
        self.regions
            .iter()
            .map(|r| (r.base, r.data.len()))
            .collect()
    }

    /// Raw pointer to the first byte of the region based at `base`.
    ///
    /// The allocation lives as long as the machine does (regions never
    /// resize after load), so frontends may keep the pointer across frames.
    pub fn region_pointer(&mut self, base: u32) -> Option<*mut u8> {
        self.regions
            .iter_mut()
            .find(|r| r.base == base)
            .map(|r| r.data.as_mut_ptr())
    }

    /// Copies the region contents of `src` into `self` in place.
    ///
    /// Both maps must have the same region layout; differing layouts are a
    /// caller bug (a save state restored into mismatched content), reported
    /// as [`MemError::LayoutMismatch`]. Copying in place (instead of
    /// swapping maps) keeps raw region pointers handed out through
    /// [`GuestMem::region_pointer`] valid across a save-state restore.
    pub fn copy_regions_from(&mut self, src: &GuestMem) -> Result<(), MemError> {
        if self.regions.len() != src.regions.len() {
            return Err(MemError::LayoutMismatch);
        }
        for (dst, src) in self.regions.iter_mut().zip(&src.regions) {
            if dst.base != src.base || dst.data.len() != src.data.len() {
                return Err(MemError::LayoutMismatch);
            }
            dst.data.copy_from_slice(&src.data);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::consts::{CODE_BASE, HEAP_BASE};

    #[test]
    fn regions_map_and_round() {
        let m = GuestMem::new(0x1234);
        // Code region rounds the image up with the 0x1000 loader head:
        // (0x1234 + 0x2FFF) & !0xFFF = 0x4000.
        assert!(m.read_u8(CODE_REGION_BASE + 0x3FFF).is_ok());
        assert!(m.read_u8(CODE_REGION_BASE + 0x4000).is_err());
        assert!(m.read_u8(HEAP_BASE).is_ok());
    }

    #[test]
    fn word_access_roundtrips() {
        let mut m = GuestMem::new(0x10000);
        m.write_u32(CODE_BASE, 0xDEAD_BEEF).unwrap();
        assert_eq!(m.read_u32(CODE_BASE).unwrap(), 0xDEAD_BEEF);
        m.write_u16(CODE_BASE + 4, 0x1234).unwrap();
        assert_eq!(m.read_u16(CODE_BASE + 4).unwrap(), 0x1234);
        m.write_u8(CODE_BASE + 6, 0x7F).unwrap();
        assert_eq!(m.read_u8(CODE_BASE + 6).unwrap(), 0x7F);
    }

    #[test]
    fn bulk_access_and_slices() {
        let mut m = GuestMem::new(0x10000);
        m.write(CODE_BASE + 0x10, &[1, 2, 3, 4]).unwrap();
        let mut b = [0u8; 4];
        m.read(CODE_BASE + 0x10, &mut b).unwrap();
        assert_eq!(b, [1, 2, 3, 4]);
        m.slice_mut(CODE_BASE + 0x20, 2).unwrap()[0] = 9;
        assert_eq!(m.read_u8(CODE_BASE + 0x20).unwrap(), 9);
        assert_eq!(m.slice(CODE_BASE + 0x20, 2).unwrap(), [9, 0]);
    }

    #[test]
    fn unmapped_access_errors() {
        let m = GuestMem::new(0x10000);
        assert_eq!(m.read_u32(0), Err(MemError::Unmapped(0)));
        assert_eq!(
            m.read_u32(0x5000_0000),
            Err(MemError::Unmapped(0x5000_0000))
        );
    }
}
