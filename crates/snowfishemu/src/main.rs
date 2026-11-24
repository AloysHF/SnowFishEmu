//! SnowFishEmu standalone frontend: loads a `.app` package, boots the
//! guest, and presents the 240x320 RGB565 surface in a scaled window.
//! A headless mode with scripted key presses drives the corpus capture
//! recipes (`scripts/batch-screenshots.ps1`).

use anyhow::{bail, Context, Result};
use clap::Parser;
use log::{debug, info};
use snowfishemu_core::audio::AUDIO_SAMPLE_RATE;
use snowfishemu_core::plat::{FB_HEIGHT, FB_WIDTH};
use snowfishemu_core::SnowFishMachine;
use std::collections::{HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// MTK key codes for the frontend's key map, mirroring the platform's
/// logical indices (see core::machine::KEY_MAP for index → code).
mod keys {
    /// Logical key indices into the core's KEY_MAP.
    pub const UP: u8 = 13;
    pub const DOWN: u8 = 14;
    pub const LEFT: u8 = 15;
    pub const RIGHT: u8 = 16;
    pub const SOFT_L: u8 = 10;
    pub const MENU: u8 = 11;
    pub const SOFT_R: u8 = 25;
    pub const OK: u8 = 5;
}

/// Instruction budget per guest call. The corpus capture recipes in
/// docs/Game-Compatibility.md are verified against this limit.
const DEFAULT_INSTRUCTION_LIMIT: u64 = 200_000_000;

#[derive(Parser)]
struct Args {
    /// Path to a `.app` package.
    path: PathBuf,
    /// Turn the registered-handset purchase answers off for diagnostics
    /// (they are on by default; `SF_FREE_PLAY=0` is the environment
    /// opt-out).
    #[arg(long)]
    no_free_play: bool,
    /// Turn the local HTTP responder off for diagnostics (it is on by
    /// default; `SF_OFFLINE_HTTP=0` is the environment opt-out).
    #[arg(long)]
    no_offline_http: bool,
    /// Run without opening a window.
    #[arg(long)]
    headless: bool,
    /// Number of frames to run in headless mode.
    #[arg(long, default_value_t = 60)]
    frames: u32,
    /// Instruction budget per guest call.
    #[arg(long, default_value_t = DEFAULT_INSTRUCTION_LIMIT)]
    instruction_limit: u64,
    /// Take a PNG screenshot after running headlessly, then exit.
    #[arg(short = 'S', long, value_name = "PATH")]
    screenshot: Option<PathBuf>,
    /// Number of frames to run before taking a screenshot.
    #[arg(long, default_value_t = 30)]
    screenshot_frames: u32,
    /// Scripted phone-key press at an exact frame, `frame:key` with the
    /// core's key indices (10 = left soft key, 5 = OK, 25 = right soft
    /// key). Repeatable. Headless and screenshot runs only.
    #[arg(long)]
    press: Vec<String>,
    /// Window scale factor (default: 2).
    #[arg(long)]
    scale: Option<u32>,
    /// Enable verbose logging.
    #[arg(short, long)]
    verbose: bool,
}

/// One scripted press: fire the key at `frame` for two guest frames.
struct Press {
    frame: u32,
    key: u8,
}

fn parse_presses(specs: &[String]) -> Result<Vec<Press>> {
    let mut script: Vec<Press> = specs
        .iter()
        .map(|spec| {
            let (frame, key) = spec
                .split_once(':')
                .with_context(|| format!("--press wants frame:key, got {spec:?}"))?;
            Ok(Press {
                frame: frame
                    .parse()
                    .with_context(|| format!("bad frame in {spec:?}"))?,
                key: key
                    .parse()
                    .with_context(|| format!("bad key in {spec:?}"))?,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    script.sort_by_key(|press| press.frame);
    Ok(script)
}

fn main() -> Result<()> {
    let args = Args::parse();
    let log_level = if args.verbose {
        log::LevelFilter::Debug
    } else {
        log::LevelFilter::Info
    };
    env_logger::Builder::new().filter_level(log_level).init();
    let presses = parse_presses(&args.press)?;
    let container =
        std::fs::read(&args.path).with_context(|| format!("read {}", args.path.display()))?;
    let res = discover_res_containers(&args.path)?;
    let mut machine = SnowFishMachine::load(container, &res)?;
    // Both provisioning switches are on by default (the core installs the
    // registered-handset answers and the HTTP responder at load);
    // `--no-free-play` / `--no-offline-http` turn one off for diagnostics.
    if args.no_free_play {
        machine.set_free_play(false);
    }
    if args.no_offline_http {
        machine.set_offline_http(false);
    }
    machine.boot(args.instruction_limit)?;

    if let Some(path) = &args.screenshot {
        capture_screenshot(
            &mut machine,
            &presses,
            args.screenshot_frames,
            args.instruction_limit,
            path,
        )?;
        return Ok(());
    }

    if args.headless {
        run_scripted_frames(&mut machine, &presses, args.frames, args.instruction_limit)?;
        info!("Headless run completed: {} frames", args.frames);
        return Ok(());
    }

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
    let mut audio_output = StandaloneAudio::try_new();
    let mut frame_start = std::time::Instant::now();
    while window.is_open() && !window.is_key_down(minifb::Key::Escape) {
        pump_keys(&mut window, &mut machine)?;

        machine.run_frame(args.instruction_limit)?;
        if let Some(audio) = &mut audio_output {
            let revision = machine.audio_revision();
            let samples = machine.take_audio_samples(AUDIO_SAMPLE_RATE as usize / 20);
            audio.push(revision, samples);
        }
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

/// Pump `frames` guest frames, firing the scripted presses at their
/// scheduled frames (each press consumes two frames: down, then up).
/// Reads the sibling resource containers the shared discovery rule names.
///
/// The rule lives in `snowfishemu_core::res` and matches the libretro
/// core's lookup: an exact `<stem>.pak`/`<stem>.res` sibling wins, else
/// every sibling whose name mentions `.pak` or `res` is served. Non-`.app`
/// files the fuzzy scan picks up are rejected by the package parser when
/// the machine loads them.
fn discover_res_containers(content_path: &Path) -> Result<Vec<Vec<u8>>> {
    let Some(dir) = content_path.parent() else {
        return Ok(Vec::new());
    };
    let entries: Vec<String> = std::fs::read_dir(dir)
        .with_context(|| format!("read directory {}", dir.display()))?
        .flatten()
        .filter_map(|entry| entry.file_name().into_string().ok())
        .collect();
    let stem = content_path.file_stem().and_then(|s| s.to_str());
    let content_name = content_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    let candidates = snowfishemu_core::res::sibling_res_candidates(stem, content_name, entries);
    let mut served = Vec::new();
    let mut containers = Vec::new();
    for name in &candidates {
        let path = dir.join(name);
        if !path.is_file() {
            continue;
        }
        containers.push(std::fs::read(&path).with_context(|| format!("read {}", path.display()))?);
        served.push(name.as_str());
    }
    if !served.is_empty() {
        info!("serving sibling resource containers: {}", served.join(", "));
    }
    Ok(containers)
}

fn run_scripted_frames(
    machine: &mut SnowFishMachine,
    presses: &[Press],
    frames: u32,
    instruction_limit: u64,
) -> Result<()> {
    let total = presses
        .last()
        .map_or(frames, |press| frames.max(press.frame + 2));
    let mut next = 0usize;
    let mut i = 0u32;
    while i < total {
        if next < presses.len() && presses[next].frame <= i {
            let press = &presses[next];
            next += 1;
            machine.set_key(press.key, true)?;
            machine.run_frame(instruction_limit)?;
            machine.set_key(press.key, false)?;
            machine.run_frame(instruction_limit)?;
            debug!("frame {i} press={}", press.key);
            i += 2;
            continue;
        }
        machine.run_frame(instruction_limit)?;
        i += 1;
    }
    Ok(())
}

/// Save the presented frame as a PNG, with the blank-frame guard and the
/// `frame nonzero=N colors=N` statistic the batch script parses.
fn capture_screenshot(
    machine: &mut SnowFishMachine,
    presses: &[Press],
    frames: u32,
    instruction_limit: u64,
    path: &Path,
) -> Result<()> {
    run_scripted_frames(machine, presses, frames, instruction_limit)?;

    let pixels = machine.frame_pixels();
    let colors: HashSet<u32> = pixels.iter().copied().collect();
    if colors.len() <= 1 {
        bail!("guest did not render a screenshot within {frames} frames");
    }
    let nonzero = pixels.iter().filter(|pixel| **pixel != 0).count();
    info!("frame nonzero={nonzero} colors={}", colors.len());
    // The presentation words carry 5-6-5 RGB expanded from the guest's
    // RGB565 surface; pack them back into RGB8 bytes for the PNG.
    let rgb: Vec<u8> = pixels
        .iter()
        .flat_map(|pixel| {
            let pixel = *pixel;
            let r = (pixel >> 19) & 0x1F;
            let g = (pixel >> 10) & 0x3F;
            let b = (pixel >> 3) & 0x1F;
            [
                (r * 255 / 31) as u8,
                (g * 255 / 63) as u8,
                (b * 255 / 31) as u8,
            ]
        })
        .collect();
    image::save_buffer(path, &rgb, FB_WIDTH, FB_HEIGHT, image::ColorType::Rgb8)
        .with_context(|| format!("failed to save screenshot: {}", path.display()))?;
    info!("Screenshot saved to: {}", path.display());
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
        (minifb::Key::M, keys::MENU),
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
            machine.set_phone_key(*index, down)?;
            KEY_STATE.with(|s| s.borrow_mut().insert(*key, down));
        }
    }
    Ok(())
}

/// Interleaved samples the ring buffer keeps ahead of the device (2 seconds).
const MAX_RING_SAMPLES: usize = AUDIO_SAMPLE_RATE as usize * 2 * 2;

/// Ring-buffered stereo audio output backed by rodio. The window loop
/// drains the machine's music engine into the ring each frame; the device
/// pulls from the ring on its own clock. No device (or a failing one) just
/// means silence.
struct StandaloneAudio {
    _sink: rodio::MixerDeviceSink,
    _player: rodio::Player,
    ring: Arc<Mutex<VecDeque<f32>>>,
    revision: u64,
}

impl StandaloneAudio {
    /// Create the output device, or return None when no device is available.
    fn try_new() -> Option<Self> {
        let sink = rodio::DeviceSinkBuilder::open_default_sink().ok()?;
        let ring = Arc::new(Mutex::new(VecDeque::new()));
        let player = rodio::Player::connect_new(sink.mixer());
        player.append(RingSource { ring: ring.clone() });
        Some(Self {
            _sink: sink,
            _player: player,
            ring,
            revision: 0,
        })
    }

    fn push(&mut self, revision: u64, samples: Vec<i16>) {
        let mut ring = self.ring.lock().unwrap();
        if self.revision != revision {
            ring.clear();
            self.revision = revision;
        }
        ring.extend(samples.into_iter().map(|sample| sample as f32 / 32768.0));
        let overflow = ring.len().saturating_sub(MAX_RING_SAMPLES);
        if overflow > 0 {
            ring.drain(..overflow);
        }
    }
}

/// Pull-style source that reads from the shared ring buffer.
struct RingSource {
    ring: Arc<Mutex<VecDeque<f32>>>,
}

impl Iterator for RingSource {
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        Some(self.ring.lock().unwrap().pop_front().unwrap_or(0.0))
    }
}

impl rodio::Source for RingSource {
    fn current_span_len(&self) -> Option<usize> {
        None
    }

    fn channels(&self) -> rodio::ChannelCount {
        rodio::ChannelCount::new(2).expect("stereo")
    }

    fn sample_rate(&self) -> rodio::SampleRate {
        rodio::SampleRate::new(AUDIO_SAMPLE_RATE).expect("44100")
    }

    fn total_duration(&self) -> Option<std::time::Duration> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_game_path_as_positional_argument() {
        let args = Args::try_parse_from(["snowfishemu", "game.app"]).unwrap();

        assert_eq!(args.path, PathBuf::from("game.app"));
        assert!(Args::try_parse_from(["snowfishemu"]).is_err());
    }

    #[test]
    fn parses_screenshot_and_press_schedule() {
        let args = Args::try_parse_from([
            "snowfishemu",
            "game.app",
            "-S",
            "shot.png",
            "--screenshot-frames",
            "200",
            "--press",
            "30:10",
            "--press",
            "65:5",
        ])
        .unwrap();

        assert_eq!(args.screenshot, Some(PathBuf::from("shot.png")));
        assert_eq!(args.screenshot_frames, 200);
        let presses = parse_presses(&args.press).unwrap();
        assert_eq!(presses.len(), 2);
        assert_eq!(presses[0].frame, 30);
        assert_eq!(presses[0].key, 10);
        assert_eq!(presses[1].key, 5);
    }

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
