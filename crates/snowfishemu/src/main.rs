//! SnowFishEmu standalone frontend: loads a `.app` package, boots the
//! guest, and presents the 240x320 RGB565 surface in a scaled window.

use anyhow::{Context, Result};
use clap::Parser;
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
    /// Optional sibling resource file served for `res`/`.pak` paths.
    #[arg(long)]
    res: Option<PathBuf>,
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
    let res = match &args.res {
        Some(p) => Some(std::fs::read(p).with_context(|| format!("read {}", p.display()))?),
        None => None,
    };
    let mut machine = SnowFishMachine::load(container, res)?;
    machine.boot(args.budget)?;

    let scale = args.scale.unwrap_or(2).max(1);
    let width = 240 * scale as usize;
    let height = 320 * scale as usize;
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
        if machine.take_dirty() {
            let px = machine.frame_pixels();
            // Scale by pixel replication into the window buffer.
            let mut scaled = Vec::with_capacity(height * width);
            for y in 0..height {
                let src_y = y / scale as usize;
                let row_start = src_y * 240;
                for x in 0..width {
                    let p = px[row_start + x / scale as usize];
                    scaled.push(p);
                }
            }
            window.update_with_buffer(&scaled, width, height)?;
        }
        let elapsed = frame_start.elapsed().as_millis() as u64;
        if elapsed < frame_ms {
            std::thread::sleep(std::time::Duration::from_millis(frame_ms - elapsed));
        }
        frame_start = std::time::Instant::now();
    }
    Ok(())
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
