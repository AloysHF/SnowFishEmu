//! Display bridge (platform GDI slots).
//!
//! All drawing lands in a host-side RGB565 surface; the guest framebuffer
//! window in the plat region is never written by the bridge (mirroring the
//! platform contract where the guest reads back through an explicit sync
//! call it never makes). A single dirty flag drives presentation.

use crate::font::FONT_ASCII8X8;
use crate::plat::{FB_HEIGHT, FB_WIDTH};

/// Host-side display state.
pub struct Gdi {
    fb: Vec<u16>,
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
            fb: vec![0; (FB_WIDTH * FB_HEIGHT) as usize],
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

    /// The surface as RGBA8 words (minifb-ready).
    pub fn frame_pixels(&self) -> Vec<u32> {
        self.fb
            .iter()
            .map(|v| {
                let r = ((v >> 11) & 0x1F) as u32;
                let g = ((v >> 5) & 0x3F) as u32;
                let b = (v & 0x1F) as u32;
                0xFF00_0000 | (r << 19) | (g << 10) | (b << 3)
            })
            .collect()
    }

    /// Single pixel with full clipping.
    fn set_px(&mut self, x: i32, y: i32, c: u16) {
        if x < 0 || x >= FB_WIDTH as i32 || y < 0 || y >= FB_HEIGHT as i32 {
            return;
        }
        self.fb[(y as u32 * FB_WIDTH + x as u32) as usize] = c;
    }

    /// Rect fill with the same RGB565 value; dirty only when non-empty.
    pub fn fill(&mut self, x: i32, y: i32, w: i32, h: i32, color: u16) {
        self.fill_count += 1;
        let x0 = x.max(0);
        let y0 = y.max(0);
        let x1 = (x + w).min(FB_WIDTH as i32);
        let y1 = (y + h).min(FB_HEIGHT as i32);
        if x1 > x0 && y1 > y0 {
            for yy in y0..y1 {
                for xx in x0..x1 {
                    self.fb[(yy as u32 * FB_WIDTH + xx as u32) as usize] = color;
                }
            }
            self.dirty = true;
        }
    }

    /// Single point; dirty set unconditionally (reference behavior).
    fn point(&mut self, x: i32, y: i32, c: u16) {
        self.set_px(x, y, c);
        self.dirty = true;
    }

    /// Verbatim RGB565 surface copy with clipping.
    fn blit_rgb565(
        &mut self,
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
                self.fb[(dj as u32 * FB_WIDTH + di as u32) as usize] =
                    src[(v * src_w + u) as usize];
            }
        }
        self.dirty = true;
    }

    /// `+0x12C fill(obj, x, y, w, h, color)`.
    pub fn op_fill(&mut self, x: i32, y: i32, w: i32, h: i32, color: u32) {
        self.fill(x, y, w, h, color as u16);
    }

    /// `+0x15C fill2(obj, x, y, w, h)` — color resolved by the caller from
    /// the canvas object or the global fill color.
    pub fn op_fill2(&mut self, x: i32, y: i32, w: i32, h: i32, color: u16) {
        self.fill2_count += 1;
        self.fill(x, y, w, h, color);
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
    pub fn op_flush(&mut self) {
        self.flush_count += 1;
    }

    /// `+0x198 draw(obj, bmp, w, h, _, dx, dy)` — palette or RGB565 blit
    /// with color-key transparency. `read_u8`/`read_u16` fetch guest bytes;
    /// failures skip pixels like the reference's per-pixel reads.
    pub fn op_draw<F>(
        &mut self,
        bmp: &[u8],
        sw: i32,
        sh: i32,
        key: u32,
        pal_mode: bool,
        w: i32,
        h: i32,
        dx: i32,
        dy: i32,
        mut read_pal: F,
    ) where
        F: FnMut(u32) -> Option<u16>,
    {
        self.draw_count += 1;
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
                    if let Some(px) = read_pal(idx as u32) {
                        self.point(dxx, dyy, px);
                    }
                }
            }
        } else {
            let tr = (key & 0xFFFF) as u16;
            let src: Vec<u16> = bmp
                .chunks_exact(2)
                .map(|p| u16::from_le_bytes([p[0], p[1]]))
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
                        self.point(dxx, dyy, px);
                    }
                }
            } else {
                self.blit_rgb565(&src, sw, sh, w, h, dx, dy);
            }
        }
    }

    /// `+0x154 text(obj, str, x, y, font)` — UTF-16 string with transparent
    /// background. ASCII uses the embedded 8x8 font (advance 8); other
    /// characters use the embedded Unicode font (advance 16). Returns the
    /// total advance.
    pub fn op_text(&mut self, chars: &[u16], x: i32, y: i32, color: u16) -> i32 {
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
                    for row in 0..8usize {
                        for col in 0..8usize {
                            // font8x8 encodes pixels LSB-first (col 0 = 0x01).
                            if glyph[row] & (1 << col) != 0 {
                                self.set_px(cx + col as i32, y + row as i32, color);
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
                                        self.set_px(cx + col as i32, y + row as i32, color);
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
        g.take_dirty();
        g.fill(-10, -10, 15, 15, 0xF800);
        assert!(g.take_dirty());
        assert_eq!(g.frame_pixels()[0], 0xFFF80000 | (0x1F << 19));
        // Empty rect leaves the dirty flag alone.
        g.take_dirty();
        g.fill(500, 500, 10, 10, 0);
        assert!(!g.take_dirty());
    }

    #[test]
    fn rgb_conversion_truncates() {
        assert_eq!(rgb888_to_565(0xFFFFFF), 0xFFFF);
        assert_eq!(rgb888_to_565(0x000000), 0x0000);
        assert_eq!(rgb888_to_565(0xF80000), ((0xF8) as u16) << 8);
    }

    #[test]
    fn text_advances_and_draws() {
        let mut g = Gdi::new();
        let advance = g.op_text(&[0x41, 0], 10, 10, 0x07E0); // "A"
        assert_eq!(advance, 8);
        assert!(g.take_dirty());
        // The glyph drew at least one green pixel somewhere on row band 10..18.
        let hit = (0..8).any(|row| {
            (0..8).any(|col| g.frame_pixels()[(10 + row) * 240 + 10 + col] != 0xFF000000)
        });
        assert!(hit, "'A' should draw pixels");
    }
}
