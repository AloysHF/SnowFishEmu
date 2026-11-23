//! Display bridge (platform GDI slots).
//!
//! Drawing updates the RGB565 framebuffer in guest memory. A single dirty
//! flag drives presentation.

use crate::font::FONT_ASCII8X8;
use crate::mem::{GuestMem, MemError};
use crate::plat::{FB_HEIGHT, FB_WIDTH, GDI_CLIP_ADDR};

pub(crate) const FONT_HEIGHT: usize = 16;

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
    /// The label cursor: `(origin, next line top)`. The SDK's dialog
    /// renderer issues one `+0x154` call per line with the *same* block
    /// origin, so a call that repeats the previous call's origin draws on
    /// the next row instead of over it. A call with a new origin restarts
    /// the block; the cursor clears on flush.
    pub(crate) text_cursor: Option<(i32, i32, i32)>,
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
            text_cursor: None,
        }
    }

    /// Clear the label cursor (start of a frame).
    pub fn reset_text_cursor(&mut self) {
        self.text_cursor = None;
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

    /// Replace the clipping rectangle, intersecting it with the display.
    pub(crate) fn set_clip(
        mem: &mut GuestMem,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
    ) -> Result<(), MemError> {
        let left = x.clamp(0, FB_WIDTH as i32);
        let top = y.clamp(0, FB_HEIGHT as i32);
        let right = x.saturating_add(w).clamp(left, FB_WIDTH as i32);
        let bottom = y.saturating_add(h).clamp(top, FB_HEIGHT as i32);
        // Store insets so zero-initialized memory (including older snapshots)
        // means the full display.
        for (i, inset) in [
            left,
            top,
            FB_WIDTH as i32 - right,
            FB_HEIGHT as i32 - bottom,
        ]
        .into_iter()
        .enumerate()
        {
            mem.write_u16(GDI_CLIP_ADDR + i as u32 * 2, inset as u16)?;
        }
        Ok(())
    }

    fn clip_bounds(mem: &GuestMem) -> [i32; 4] {
        let inset = |i| i32::from(mem.read_u16(GDI_CLIP_ADDR + i).unwrap_or_default());
        [
            inset(0).min(FB_WIDTH as i32),
            inset(2).min(FB_HEIGHT as i32),
            (FB_WIDTH as i32 - inset(4)).max(0),
            (FB_HEIGHT as i32 - inset(6)).max(0),
        ]
    }

    /// Single pixel clipped to the display, written into guest memory.
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
        let [left, top, right, bottom] = Self::clip_bounds(mem);
        let x0 = x.max(left);
        let y0 = y.max(top);
        let x1 = x.saturating_add(w).min(right);
        let y1 = y.saturating_add(h).min(bottom);
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

    /// Draws a one-pixel-wide line between inclusive endpoints.
    pub(crate) fn line(
        &mut self,
        mem: &mut GuestMem,
        (x0, y0): (i32, i32),
        (x1, y1): (i32, i32),
        color: u16,
    ) {
        let steep = y0.abs_diff(y1) > x0.abs_diff(x1);
        let axes = |x, y| if steep { (y, x) } else { (x, y) };
        let start = axes(x0, y0);
        let end = axes(x1, y1);
        let ((x0, y0), (x1, y1)) = if start.0 <= end.0 {
            (start, end)
        } else {
            (end, start)
        };
        let [left, top, right, bottom] = Self::clip_bounds(mem);
        let (clip_start, clip_top) = axes(left, top);
        let (clip_end, clip_bottom) = axes(right, bottom);
        // Clip the iteration on the major axis without shifting the line.
        let width = i128::from(x0.abs_diff(x1).max(1));
        let height = i128::from(y0.abs_diff(y1));
        let direction = if y1 >= y0 { 1 } else { -1 };
        for x in x0.max(clip_start)..=x1.min(clip_end - 1) {
            let step = i128::from(x) - i128::from(x0);
            let y = i128::from(y0) + direction * (step * height + width / 2) / width;
            if (i128::from(clip_top)..i128::from(clip_bottom)).contains(&y) {
                let (x, y) = axes(x, y as i32);
                self.point(mem, x, y, color);
            }
        }
    }

    /// Draws a one-pixel border inside the given rectangle.
    pub(crate) fn rect_outline(
        &mut self,
        mem: &mut GuestMem,
        (x, y): (i32, i32),
        (width, height): (i32, i32),
        color: u16,
    ) {
        if width <= 0 || height <= 0 {
            return;
        }
        let right = x.saturating_add(width - 1);
        let bottom = y.saturating_add(height - 1);
        self.line(mem, (x, y), (right, y), color);
        self.line(mem, (x, bottom), (right, bottom), color);
        self.line(mem, (x, y), (x, bottom), color);
        self.line(mem, (right, y), (right, bottom), color);
    }

    /// Fill a rectangle with an explicit RGB565 color.
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

    /// Blits a region from a palette or RGB565 bitmap with an optional key.
    #[allow(clippy::too_many_arguments)]
    pub fn op_draw(
        &mut self,
        mem: &mut crate::mem::GuestMem,
        bmp: &[u8],
        (sw, sh): (i32, i32),
        key: Option<u16>,
        palette: Option<u32>,
        (sx, sy): (i32, i32),
        (w, h): (i32, i32),
        (dx, dy): (i32, i32),
        transform: u32,
    ) {
        self.draw_count += 1;
        // Pre-read the palette (256 entries max) so pixel writes and palette
        // reads don't alias the same guest-memory borrow.
        let pal_cache: Option<[u16; 256]> = palette.map(|addr| {
            std::array::from_fn(|i| mem.read_u16(addr.wrapping_add(i as u32 * 2)).unwrap_or(0))
        });
        let [left, top, right, bottom] = Self::clip_bounds(mem);
        let flip_x = matches!(transform, 1 | 3);
        let flip_y = matches!(transform, 2 | 3);
        // Clip in draw order so a reflected source rectangle keeps its origin.
        let sx = if flip_x {
            sw.saturating_sub(sx).saturating_sub(w)
        } else {
            sx
        };
        let sy = if flip_y {
            sh.saturating_sub(sy).saturating_sub(h)
        } else {
            sy
        };
        let min_x = 0.max(sx.saturating_neg()).max(left.saturating_sub(dx));
        let min_y = 0.max(sy.saturating_neg()).max(top.saturating_sub(dy));
        let max_x = w.min(sw.saturating_sub(sx)).min(right.saturating_sub(dx));
        let max_y = h.min(sh.saturating_sub(sy)).min(bottom.saturating_sub(dy));
        for j in min_y..max_y {
            let y = j + sy;
            let row = (if flip_y { sh - 1 - y } else { y }) as usize * sw as usize;
            for i in min_x..max_x {
                let x = i + sx;
                let offset = row + (if flip_x { sw - 1 - x } else { x }) as usize;
                let (value, pixel) = match &pal_cache {
                    Some(colors) => {
                        let index = bmp[offset] as usize;
                        (index as u16, colors[index])
                    }
                    None => {
                        let pixel = u16::from_le_bytes([bmp[offset * 2], bmp[offset * 2 + 1]]);
                        (pixel, pixel)
                    }
                };
                if key != Some(value) {
                    self.point(mem, i + dx, j + dy, pixel);
                }
            }
        }
    }

    /// Advance width of one text cell: 8 for ASCII and control cells, 16
    /// for wide CJK code points, CJK punctuation, and the horizontal ellipsis.
    pub(crate) fn cell_width(ch: u16) -> i32 {
        match ch {
            0 => 0,
            0x2026 | 0x3000..=0x303F | 0x4E00..=u16::MAX => 16,
            _ => 8,
        }
    }

    /// Extent of a `+0x154` label as `(width, height)`, so the caller can
    /// keep it on the surface before drawing.
    pub fn text_extent(chars: &[u16]) -> (i32, i32) {
        let mut w = 0;
        let mut h = 8;
        for &ch in chars {
            if ch == 0 {
                break;
            }
            let cell = Self::cell_width(ch);
            w += cell;
            h = h.max(cell);
        }
        (w, h)
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
        let (advance, _) = self.op_text_block(mem, chars, x, y, color, None);
        advance
    }

    /// Draw a UTF-16 run, optionally wrapped at `right` (the x coordinate
    /// the next cell must not cross). This is the renderer behind both
    /// `+0x154` labels and `+0x14C` paragraph bodies. The guest may hand
    /// the platform a whole paragraph, so the renderer breaks lines at
    /// the requested edge. A line's height is the tallest
    /// cell in it (8 for ASCII, 16 for wide cells), matching the advance
    /// metrics the guest lays its origins out with. Returns the total
    /// advance and the y coordinate below the last finished line.
    pub fn op_text_block(
        &mut self,
        mem: &mut crate::mem::GuestMem,
        chars: &[u16],
        x: i32,
        y: i32,
        color: u16,
        right: Option<i32>,
    ) -> (i32, i32) {
        self.text_count += 1;
        let [left, top, clip_right, bottom] = Self::clip_bounds(mem);
        let visible = |x, y| (left..clip_right).contains(&x) && (top..bottom).contains(&y);
        let mut cx = x;
        let mut cy = y;
        // Height of the tallest cell placed on the line being drawn.
        let mut line_h = 8i32;
        let mut advance = 0i32;
        for &ch in chars {
            if ch == 0 {
                break;
            }
            let cell = Self::cell_width(ch);
            if let Some(right) = right {
                // Wrap before a cell that would cross the edge, and skip a
                // cell wider than the whole line so a pathological run
                // cannot loop forever.
                if cx + cell > right && cell <= right - x {
                    cx = x;
                    cy += line_h;
                    line_h = 8;
                }
            }
            let row0 = cy;
            match ch {
                0x00..=0x1F => cx += 8,
                0x20..=0x7E => {
                    let glyph = FONT_ASCII8X8[(ch - 0x20) as usize];
                    for (row, bits) in glyph.iter().enumerate() {
                        for col in 0..8usize {
                            // font8x8 encodes pixels LSB-first (col 0 = 0x01).
                            let x = cx + col as i32;
                            let y = row0 + row as i32;
                            if bits & (1 << col) != 0 && visible(x, y) {
                                self.set_px(mem, x, y, color);
                            }
                        }
                    }
                    cx += 8;
                }
                _ if cell == 8 => cx += 8,
                _ => {
                    if let Some(c) = char::from_u32(ch as u32) {
                        if let Some(glyph) = unifont::get_glyph(c) {
                            for row in 0..FONT_HEIGHT {
                                for col in 0..glyph.get_width() {
                                    let x = cx + col as i32;
                                    let y = row0 + row as i32;
                                    if glyph.get_pixel(col, row) && visible(x, y) {
                                        self.set_px(mem, x, y, color);
                                    }
                                }
                            }
                        }
                    }
                    cx += 16;
                }
            }
            line_h = line_h.max(cell);
            advance += cell;
        }
        self.dirty = true;
        (advance, cy + line_h)
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
    fn line_clips_to_canvas_and_keeps_one_pixel_per_step() {
        let mut g = Gdi::new();
        let mut mem = GuestMem::new(0x1000);
        Gdi::set_clip(&mut mem, 0, 0, 4, 5).unwrap();
        g.take_dirty();
        g.line(&mut mem, (5, 8), (-1, -4), 0xffff);
        assert!(g.take_dirty());
        let pixels = g.frame_pixels(&mem);
        let positions: Vec<_> = pixels
            .iter()
            .enumerate()
            .filter_map(|(i, &pixel)| (pixel == 0xfff8_fcf8).then_some(i))
            .collect();
        let stride = FB_WIDTH as usize;
        assert_eq!(
            positions,
            [
                1,
                stride + 2,
                2 * stride + 2,
                3 * stride + 3,
                4 * stride + 3
            ]
        );
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

    #[test]
    fn text_block_wraps_at_the_right_edge() {
        let mut g = Gdi::new();
        let mut mem = crate::mem::GuestMem::new(0x1000);
        // Five ASCII cells of 8 px starting at 36: three fit inside right=60,
        // the remaining two wrap to the next 8-px row (top at y=18).
        let chars: Vec<u16> = "ABCDE".chars().map(|c| c as u16).collect();
        let (_, end_y) = g.op_text_block(&mut mem, &chars, 36, 10, 0x07E0, Some(60));
        assert_eq!(end_y, 26);
        let px = g.frame_pixels(&mem);
        let first_row_hit = (0..8).any(|row| px[(10 + row) * 240 + 36] != 0xFF000000);
        let wrapped_hit = (0..8).any(|row| px[(18 + row) * 240 + 36] != 0xFF000000);
        assert!(first_row_hit, "first line should draw at y=10");
        assert!(wrapped_hit, "'E' should wrap to y=18");
    }

    #[test]
    fn text_block_line_height_follows_the_tallest_cell() {
        let mut g = Gdi::new();
        let mut mem = crate::mem::GuestMem::new(0x1000);
        // One ASCII cell then one wide (CJK-range) cell: line 1 finishes at
        // the ASCII height (8), and the wrapped wide cell occupies 8..24.
        let chars = [0x41u16, 0x4E00];
        let (_, end_y) = g.op_text_block(&mut mem, &chars, 0, 0, 0x0000, Some(16));
        assert_eq!(end_y, 24);
    }
}
