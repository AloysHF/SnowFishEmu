//! Display bridge (platform GDI slots).
//!
//! All drawing lands in a host-side RGB565 surface; the guest framebuffer
//! window in the plat region is never written by the bridge (mirroring the
//! platform contract where the guest reads back through an explicit sync
//! call it never makes). A single dirty flag drives presentation.

use crate::font::FONT_ASCII8X8;
use crate::plat::{FB_HEIGHT, FB_WIDTH};

/// Display bridge state. All drawing lands in the guest framebuffer window
/// (plat region 0x540720) — the same surface the canvas descriptor points
/// at, so GDI ops and direct guest writes compose on one surface.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct Gdi {
    dirty: bool,
    fill_color: u16,
    fill_count: u64,
    text_count: u64,
    draw_count: u64,
    clear_count: u64,
    fill2_count: u64,
    flush_count: u64,
}

/// RGB888 → RGB565 truncation used by the platform bridge.
pub fn rgb888_to_565(c: u32) -> u16 {
    (((c >> 19) << 11) | (((c >> 10) << 5) & 0x7E0) | ((c >> 3) & 0x1F)) as u16
}

impl Gdi {
    pub fn new() -> Self {
        Self {
            dirty: true,
            fill_color: 0xFFFF,
            fill_count: 0,
            text_count: 0,
            draw_count: 0,
            clear_count: 0,
            fill2_count: 0,
            flush_count: 0,
        }
    }

    /// Whether the surface changed since the last presentation.
    pub fn take_dirty(&mut self) -> bool {
        std::mem::take(&mut self.dirty)
    }

    /// Call counters per GDI operation (fill, text, draw, clear, fill2,
    /// flush) for bring-up diagnostics.
    pub fn counters(&self) -> [u64; 6] {
        [
            self.fill_count,
            self.text_count,
            self.draw_count,
            self.clear_count,
            self.fill2_count,
            self.flush_count,
        ]
    }

    /// The surface as RGBA8 words (minifb-ready), read from guest memory.
    pub fn frame_pixels(&self, mem: &crate::mem::GuestMem) -> Vec<u32> {
        match mem.slice(crate::plat::FB_ADDR, crate::plat::FB_SIZE) {
            Ok(raw) => {
                let mut out = Vec::with_capacity((crate::plat::FB_SIZE / 2) as usize);
                for i in 0..out.capacity() {
                    let v = u16::from_le_bytes([raw[i * 2], raw[i * 2 + 1]]);
                    let r = ((v >> 11) & 0x1F) as u32;
                    let g = ((v >> 5) & 0x3F) as u32;
                    let b = (v & 0x1F) as u32;
                    out.push(0xFF00_0000 | (r << 19) | (g << 10) | (b << 3));
                }
                out
            }
            Err(_) => Vec::new(),
        }
    }

    /// Single pixel with full clipping, written into guest memory.
    fn set_px(&mut self, mem: &mut crate::mem::GuestMem, x: i32, y: i32, c: u16) {
        if x < 0 || x >= FB_WIDTH as i32 || y < 0 || y >= FB_HEIGHT as i32 {
            return;
        }
        let addr = crate::plat::FB_ADDR + (y as u32 * FB_WIDTH + x as u32) * 2;
        let _ = mem.write_u16(addr, c);
    }

    /// Rect fill with the same RGB565 value; dirty only when non-empty.
    #[allow(clippy::too_many_arguments)]
    pub fn fill(
        &mut self,
        mem: &mut crate::mem::GuestMem,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        color: u16,
    ) {
        self.fill_count += 1;
        let x0 = x.max(0);
        let y0 = y.max(0);
        let x1 = (x + w).min(FB_WIDTH as i32);
        let y1 = (y + h).min(FB_HEIGHT as i32);
        if x1 > x0 && y1 > y0 {
            for yy in y0..y1 {
                let addr = crate::plat::FB_ADDR + (yy as u32 * FB_WIDTH + x0 as u32) * 2;
                let count = (x1 - x0) as usize;
                if let Ok(dst) = mem.slice_mut(addr, (count * 2) as u32) {
                    for i in 0..count {
                        let b = color.to_le_bytes();
                        dst[i * 2] = b[0];
                        dst[i * 2 + 1] = b[1];
                    }
                }
            }
            self.dirty = true;
        }
    }

    /// Single point; dirty set unconditionally (reference behavior).
    fn point(&mut self, mem: &mut crate::mem::GuestMem, x: i32, y: i32, c: u16) {
        self.set_px(mem, x, y, c);
        self.dirty = true;
    }

    /// Verbatim RGB565 surface copy with clipping.
    #[allow(clippy::too_many_arguments)]
    fn blit_rgb565(
        &mut self,
        mem: &mut crate::mem::GuestMem,
        src: &[u16],
        src_w: i32,
        src_h: i32,
        w: i32,
        h: i32,
        dx: i32,
        dy: i32,
    ) {
        if w <= 0 || h <= 0 {
            return;
        }
        for j in 0..h {
            let dj = j + dy;
            if dj < 0 || dj >= FB_HEIGHT as i32 {
                continue;
            }
            for i in 0..w {
                let di = i + dx;
                if di < 0 || di >= FB_WIDTH as i32 {
                    continue;
                }
                let u = i;
                let v = j;
                if u < 0 || u >= src_w || v < 0 || v >= src_h {
                    continue;
                }
                self.set_px(mem, di, dj, src[(v * src_w + u) as usize]);
            }
        }
        self.dirty = true;
    }

    /// `+0x12C fill(obj, x, y, w, h, color)`.
    pub fn op_fill(
        &mut self,
        mem: &mut crate::mem::GuestMem,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        color: u32,
    ) {
        self.fill(mem, x, y, w, h, color as u16);
    }

    /// `+0x15C fill2(obj, x, y, w, h)` — color resolved by the caller from
    /// the canvas object or the global fill color.
    pub fn op_fill2(
        &mut self,
        mem: &mut crate::mem::GuestMem,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        color: u16,
    ) {
        self.fill2_count += 1;
        self.fill(mem, x, y, w, h, color);
    }

    /// `+0x184 clear(obj, r, g, b)` — records the pen color (returned as
    /// `(rgb888, rgb565)` for the caller to store into the canvas object);
    /// the surface itself is untouched.
    pub fn op_clear(&mut self, r: u32, g: u32, b: u32) -> (u32, u16) {
        self.clear_count += 1;
        let rgb24 = ((r << 16) & 0xFF0000) | ((g << 8) & 0xFF00) | (b & 0xFF);
        let rgb565 = rgb888_to_565(rgb24);
        self.fill_color = rgb565;
        (rgb24, rgb565)
    }

    /// The global fill color (fill2 fallback).
    pub fn fill_color(&self) -> u16 {
        self.fill_color
    }

    /// `+0x194 bmp(obj, data, w, h)` — counted only; nothing drawn.
    pub fn op_bmp(&mut self) {
        self.draw_count += 1;
    }

    /// `+0x010 flush()` — counted only; presentation is host-driven.
    pub fn op_flush(&mut self, mem: &crate::mem::GuestMem) {
        self.flush_count += 1;
        if std::env::var_os("SF_FB_PROBE").is_some() {
            let p00 = mem.read_u16(crate::plat::FB_ADDR).unwrap_or(0);
            let pmid = mem
                .read_u16(crate::plat::FB_ADDR + (180 * 240 + 99) * 2)
                .unwrap_or(0);
            log::debug!("[gdi] flush probe (0,0)={p00:#06x} (99,180)={pmid:#06x}");
        }
    }

    /// `+0x198 draw(obj, bmp, w, h, _, dx, dy)` — palette or RGB565 blit
    /// with color-key transparency. `read_u8`/`read_u16` fetch guest bytes;
    /// failures skip pixels like the reference's per-pixel reads.
    #[allow(clippy::too_many_arguments)]
    pub fn op_draw(
        &mut self,
        mem: &mut crate::mem::GuestMem,
        bmp: &[u8],
        sw: i32,
        sh: i32,
        key: u32,
        pal_mode: bool,
        w: i32,
        h: i32,
        dx: i32,
        dy: i32,
        pal_addr: u32,
    ) {
        self.draw_count += 1;
        // Pre-read the palette (256 entries max) so pixel writes and palette
        // reads don't alias the same guest-memory borrow.
        let pal_cache: Vec<u16> = if pal_mode {
            (0..256u32)
                .map(|i| mem.read_u16(pal_addr.wrapping_add(i * 2)).unwrap_or(0))
                .collect()
        } else {
            Vec::new()
        };
        if pal_mode {
            let tr = (key & 0xFF) as u8;
            for j in 0..h.min(sh) {
                let dyy = j + dy;
                if dyy < 0 || dyy >= FB_HEIGHT as i32 {
                    continue;
                }
                for i in 0..w.min(sw) {
                    let dxx = i + dx;
                    if dxx < 0 || dxx >= FB_WIDTH as i32 {
                        continue;
                    }
                    let idx = bmp[(j * sw + i) as usize];
                    if idx == tr {
                        continue;
                    }
                    let px = pal_cache.get(idx as usize).copied().unwrap_or(0);
                    self.point(mem, dxx, dyy, px);
                }
            }
        } else {
            let tr = (key & 0xFFFF) as u16;
            let src: Vec<u16> = (0..bmp.len() / 2)
                .map(|i| u16::from_le_bytes([bmp[i * 2], bmp[i * 2 + 1]]))
                .collect();
            if tr != 0 && tr != 0xFFFF {
                for j in 0..h.min(sh) {
                    let dyy = j + dy;
                    if dyy < 0 || dyy >= FB_HEIGHT as i32 {
                        continue;
                    }
                    for i in 0..w.min(sw) {
                        let dxx = i + dx;
                        if dxx < 0 || dxx >= FB_WIDTH as i32 {
                            continue;
                        }
                        let px = src[(j * sw + i) as usize];
                        if px == tr {
                            continue;
                        }
                        self.point(mem, dxx, dyy, px);
                    }
                }
            } else {
                self.blit_rgb565(mem, &src, sw, sh, w, h, dx, dy);
            }
        }
    }

    /// `+0x154 text(obj, str, x, y, font)` — UTF-16 string with transparent
    /// background. ASCII uses the embedded 8x8 font (advance 8); other
    /// characters use the embedded Unicode font (advance 16). Returns the
    /// total advance.
    pub fn op_text(
        &mut self,
        mem: &mut crate::mem::GuestMem,
        chars: &[u16],
        x: i32,
        y: i32,
        color: u16,
    ) -> i32 {
        self.text_count += 1;
        let mut cx = x;
        for &ch in chars {
            if ch == 0 {
                break;
            }
            match ch {
                0x00..=0x1F => cx += 8,
                0x20..=0x7E => {
                    let glyph = FONT_ASCII8X8[(ch - 0x20) as usize];
                    for (row, bits) in glyph.iter().enumerate() {
                        for col in 0..8usize {
                            // font8x8 encodes pixels LSB-first (col 0 = 0x01).
                            if bits & (1 << col) != 0 {
                                self.set_px(mem, cx + col as i32, y + row as i32, color);
                            }
                        }
                    }
                    cx += 8;
                }
                0x7F..=0x4DFF => cx += 8,
                _ => {
                    if let Some(c) = char::from_u32(ch as u32) {
                        if let Some(glyph) = unifont::get_glyph(c) {
                            for row in 0..16usize {
                                for col in 0..glyph.get_width() {
                                    if glyph.get_pixel(col, row) {
                                        self.set_px(mem, cx + col as i32, y + row as i32, color);
                                    }
                                }
                            }
                        }
                    }
                    cx += 16;
                }
            }
        }
        self.dirty = true;
        cx - x
    }
}

impl Default for Gdi {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fill_clips_and_marks_dirty() {
        let mut g = Gdi::new();
        let mut mem = crate::mem::GuestMem::new(0x1000);
        g.take_dirty();
        g.fill(&mut mem, -10, -10, 15, 15, 0xF800);
        assert!(g.take_dirty());
        assert_eq!(g.frame_pixels(&mem)[0], 0xFFF80000 | (0x1F << 19));
        // Empty rect leaves the dirty flag alone.
        g.take_dirty();
        g.fill(&mut mem, 500, 500, 10, 10, 0);
        assert!(!g.take_dirty());
    }

    #[test]
    fn rgb_conversion_truncates() {
        assert_eq!(rgb888_to_565(0xFFFFFF), 0xFFFF);
        assert_eq!(rgb888_to_565(0x000000), 0x0000);
        assert_eq!(rgb888_to_565(0xF80000), 0xF800u16);
    }

    #[test]
    fn text_advances_and_draws() {
        let mut g = Gdi::new();
        let mut mem = crate::mem::GuestMem::new(0x1000);
        let advance = g.op_text(&mut mem, &[0x41, 0], 10, 10, 0x07E0); // "A"
        assert_eq!(advance, 8);
        assert!(g.take_dirty());
        // The glyph drew at least one green pixel somewhere on row band 10..18.
        let hit = (0..8).any(|row| {
            (0..8).any(|col| g.frame_pixels(&mem)[(10 + row) * 240 + 10 + col] != 0xFF000000)
        });
        assert!(hit, "'A' should draw pixels");
    }
}
