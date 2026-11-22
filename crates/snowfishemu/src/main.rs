//! SnowFishEmu standalone frontend: loads a `.app` package, boots the
//! guest, and presents the 240x320 RGB565 surface in a scaled window.

use anyhow::{Context, Result};
use clap::Parser;
use snowfishemu_core::plat::{FB_HEIGHT, FB_WIDTH};
use snowfishemu_core::SnowFishMachine;
use std::path::PathBuf;

/// MTK key codes for the frontend's key map, mirroring the platform's
/// logical indices (see core::machine::KEY_MAP for index → code).
mod keys {
    /// Logical key indices into the core's KEY_MAP.
    pub const UP: u8 = 2;
    pub const DOWN: u8 = 8;
    pub const LEFT: u8 = 4;
    pub const RIGHT: u8 = 6;
    pub const SOFT_L: u8 = 10;
    pub const SOFT_R: u8 = 25;
    pub const OK: u8 = 5;
}

#[derive(Parser)]
struct Args {
    /// Path to a `.app` package.
    path: PathBuf,
    /// Sibling resource container(s) whose packs back `res`/`.pak` paths.
    /// Repeat the option to serve more than one container.
    #[arg(long)]
    res: Vec<PathBuf>,
    /// Answer the purchase service as a registered handset (the same
    /// switch as sfa_headless `--free-play` / `SF_FREE_PLAY=1`).
    #[arg(long)]
    free_play: bool,
    /// Answer the SDK's HTTP transactions locally (the same switch as
    /// sfa_headless `--offline-http` / `SF_OFFLINE_HTTP=1`).
    #[arg(long)]
    offline_http: bool,
    /// Window scale factor (default: fit 240x320 into ~640x854).
    #[arg(long)]
    scale: Option<u32>,
    /// Instruction budget per guest call.
    #[arg(long, default_value = "200000000")]
    budget: u64,
}

fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();
    let args = Args::parse();
    let container =
        std::fs::read(&args.path).with_context(|| format!("read {}", args.path.display()))?;
    let mut res = Vec::new();
    for path in &args.res {
        res.push(std::fs::read(path).with_context(|| format!("read {}", path.display()))?);
    }
    let mut machine = SnowFishMachine::load(container, &res)?;
    // The flags only ever turn the switches ON: `SF_FREE_PLAY` /
    // `SF_OFFLINE_HTTP` are honoured by the core at load, and a flag adds
    // the switch on top rather than overriding an env var back off.
    if args.free_play {
        machine.set_free_play(true);
    }
    if args.offline_http {
        machine.set_offline_http(true);
    }
    machine.boot(args.budget)?;

    let scale = args.scale.unwrap_or(2).max(1) as usize;
    let width = FB_WIDTH as usize * scale;
    let height = FB_HEIGHT as usize * scale;
    // The Windows backend retains the buffer pointer for paint messages.
    let mut pixels = vec![0; height * width];
    let mut window = minifb::Window::new(
        "SnowFishEmu",
        width,
        height,
        minifb::WindowOptions {
            scale: minifb::Scale::X1,
            resize: false,
            ..minifb::WindowOptions::default()
        },
    )
    .context("create window")?;

    // Frame pacing ~20 fps like the platform tick.
    let frame_ms = 50u64;
    let mut frame_start = std::time::Instant::now();
    while window.is_open() && !window.is_key_down(minifb::Key::Escape) {
        pump_keys(&mut window, &mut machine)?;

        machine.run_frame(args.budget)?;
        let frame = machine.take_dirty().then(|| machine.frame_pixels());
        present_frame(frame.as_deref(), &mut pixels, scale, |pixels| {
            window.update_with_buffer(pixels, width, height)
        })?;
        let elapsed = frame_start.elapsed().as_millis() as u64;
        if elapsed < frame_ms {
            std::thread::sleep(std::time::Duration::from_millis(frame_ms - elapsed));
        }
        frame_start = std::time::Instant::now();
    }
    Ok(())
}

fn present_frame(
    frame: Option<&[u32]>,
    pixels: &mut [u32],
    scale: usize,
    update_window: impl FnOnce(&[u32]) -> minifb::Result<()>,
) -> minifb::Result<()> {
    if let Some(frame) = frame {
        let source_width = FB_WIDTH as usize;
        for (y, row) in pixels.chunks_exact_mut(source_width * scale).enumerate() {
            let start = (y / scale) * source_width;
            let source = &frame[start..start + source_width];
            for (&pixel, block) in source.iter().zip(row.chunks_exact_mut(scale)) {
                block.fill(pixel);
            }
        }
    }
    // Updating the window also pumps input and close messages on clean frames.
    update_window(pixels)
}

thread_local! {
    static KEY_STATE: std::cell::RefCell<std::collections::HashMap<minifb::Key, bool>> =
        std::cell::RefCell::new(std::collections::HashMap::new());
}

/// Sends press/release edges for the mapped keys.
fn pump_keys(window: &mut minifb::Window, machine: &mut SnowFishMachine) -> Result<()> {
    const MAP: &[(minifb::Key, u8)] = &[
        (minifb::Key::Up, keys::UP),
        (minifb::Key::Down, keys::DOWN),
        (minifb::Key::Left, keys::LEFT),
        (minifb::Key::Right, keys::RIGHT),
        (minifb::Key::Enter, keys::OK),
        (minifb::Key::Z, keys::SOFT_L),
        (minifb::Key::X, keys::SOFT_R),
        (minifb::Key::Key1, 0),
        (minifb::Key::Key2, 1),
        (minifb::Key::Key3, 2),
        (minifb::Key::Key4, 3),
        (minifb::Key::Key5, 4),
        (minifb::Key::Key6, 5),
        (minifb::Key::Key7, 6),
        (minifb::Key::Key8, 7),
        (minifb::Key::Key9, 8),
        (minifb::Key::Key0, 9),
    ];
    for (key, index) in MAP {
        let down = window.is_key_down(*key);
        let was = KEY_STATE.with(|s| s.borrow().get(key).copied().unwrap_or(false));
        if down != was {
            machine.set_key(*index, down)?;
            KEY_STATE.with(|s| s.borrow_mut().insert(*key, down));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn static_frame_updates_the_window_with_the_same_buffer() {
        let scale = 2;
        let width = FB_WIDTH as usize * scale;
        let mut pixels = vec![0; width * FB_HEIGHT as usize * scale];
        let buffer_address = pixels.as_ptr();
        let mut frame = vec![0; FB_WIDTH as usize * FB_HEIGHT as usize];
        frame[0] = 0xf800;
        frame[1] = 0x001f;
        let mut updates = 0;
        let mut window_update = |buffer: &[u32]| {
            assert_eq!(buffer.as_ptr(), buffer_address);
            assert_eq!(&buffer[..4], &[0xf800, 0xf800, 0x001f, 0x001f]);
            assert_eq!(&buffer[width..width + 4], &buffer[..4]);
            updates += 1;
            Ok(())
        };

        present_frame(Some(&frame), &mut pixels, scale, &mut window_update).unwrap();
        for _ in 0..2 {
            present_frame(None, &mut pixels, scale, &mut window_update).unwrap();
        }
        assert_eq!(updates, 3);
    }
}
