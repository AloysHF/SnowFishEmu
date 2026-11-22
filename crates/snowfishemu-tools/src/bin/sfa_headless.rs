//! Headless bring-up harness: boot a `.app`, pump events, report state.
//!
//! Used to validate guest bring-up without a window: prints the wrapper
//! set, handler registration, service-call counts, and the final guest
//! state byte.

use anyhow::{Context, Result};
use clap::Parser;
use snowfishemu_core::SnowFishMachine;
use std::path::{Path, PathBuf};

/// Default per-callback instruction budget, shared with the standalone
/// frontend's `--instruction-limit`.
const DEFAULT_INSTRUCTION_LIMIT: u64 = 200_000_000;

#[derive(Parser)]
struct Args {
    /// Path to a `.app` package.
    path: PathBuf,
    /// Maximum guest instructions per callback.
    #[arg(long, default_value_t = DEFAULT_INSTRUCTION_LIMIT)]
    instruction_limit: u64,
    /// Number of frames to pump after boot.
    #[arg(long, default_value = "5")]
    frames: u32,
    /// Queue a few key presses after boot (indices into the key map).
    #[arg(long)]
    keys: Vec<u8>,
    /// Press a key at an exact frame, `frame:key` (same indices as `--keys`).
    /// Repeatable; a press holds the key for one frame and releases it on the
    /// next, so a scripted run advances two frames per press.
    #[arg(long = "press")]
    press: Vec<String>,
    /// Skip the automatic bring-up key replay (pure idle run).
    #[arg(long)]
    no_keys: bool,
    /// Directory for per-frame framebuffer dumps.
    #[arg(long)]
    dump_fb: Option<PathBuf>,
    /// Dump a guest memory range after the run: `addr:len:path` (hex addr,
    /// hex len). Repeatable, e.g. `--dump-mem 60d5ec:400:heap.bin`.
    #[arg(long = "dump-mem")]
    dump_mem: Vec<String>,
    /// Answer the purchase service as a registered, billing-capable handset
    /// so trial titles reach their own play state (same as `SF_FREE_PLAY=1`).
    #[arg(long)]
    free_play: bool,
    /// Answer the SDK's carrier-facing HTTP request locally instead of
    /// leaving the transaction pending (same as `SF_OFFLINE_HTTP=1`).
    #[arg(long)]
    offline_http: bool,
    /// Side-load a local file to answer download requests whose path contains
    /// the `--serve-match` substring (default `.pak`). Same as `SF_SERVE_ASSET`.
    /// The bytes are a real local artifact, not fabricated server data.
    #[arg(long)]
    serve_asset: Option<PathBuf>,
    /// Substring a request path must contain for `--serve-asset` to be served
    /// (default `.pak`). Same as `SF_SERVE_MATCH`.
    #[arg(long, default_value = ".pak")]
    serve_match: String,
    /// Suppress the per-frame app-activate/resume event (diagnostic only).
    #[arg(long)]
    no_app_pump: bool,
}

fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();
    let args = Args::parse();
    let container =
        std::fs::read(&args.path).with_context(|| format!("read {}", args.path.display()))?;
    let res = discover_res_containers(&args.path)?;
    let mut m = SnowFishMachine::load(container, &res)?;
    if args.free_play {
        m.set_free_play(true);
    }
    if args.offline_http {
        m.set_offline_http(true);
    }
    if let Some(path) = &args.serve_asset {
        let bytes = std::fs::read(path).with_context(|| format!("read {}", path.display()))?;
        m.set_serve_asset(bytes);
        m.set_serve_asset_match(args.serve_match.clone());
    }
    if args.no_app_pump {
        m.set_app_pump(false);
    }
    for (name, len) in m.resource_packs() {
        eprintln!("resource pack: {name} ({len} bytes)");
    }
    m.boot(args.instruction_limit)?;
    // An explicit `--press frame:key` schedule takes over the frame loop;
    // the bring-up replay below is only a convenience for idle runs.
    let mut script: Vec<(u32, u8)> = Vec::new();
    for spec in &args.press {
        let (frame, key) = spec
            .split_once(':')
            .with_context(|| format!("--press wants frame:key, got {spec:?}"))?;
        script.push((
            frame
                .parse()
                .with_context(|| format!("bad frame in {spec:?}"))?,
            key.parse()
                .with_context(|| format!("bad key in {spec:?}"))?,
        ));
    }
    script.sort_unstable();
    let scripted = !script.is_empty();
    let total = script
        .last()
        .map_or(args.frames, |(frame, _)| args.frames.max(frame + 2));

    let dump_frame = |m: &SnowFishMachine, frame: u32| {
        if let Some(dir) = &args.dump_fb {
            let px = m.frame_pixels();
            let mut out = Vec::with_capacity(px.len() * 4);
            for p in &px {
                out.extend_from_slice(&p.to_le_bytes());
            }
            let _ = std::fs::write(dir.join(format!("frame_{frame:03}.raw")), &out);
        }
    };

    // Mimic the platform bring-up: 5 idle ticks, LSK down+up, OK down+up,
    // then the remaining frames (key indices per the core's key table).
    // --no-keys skips the replay for pure idle runs.
    let lsk = 10u8;
    let ok = 5u8;
    let mut i = 0u32;
    let mut next = 0usize;
    while i < total {
        if scripted && next < script.len() && script[next].0 <= i {
            let key = script[next].1;
            next += 1;
            m.set_key(key, true)?;
            m.run_frame(args.instruction_limit)?;
            m.set_key(key, false)?;
            m.run_frame(args.instruction_limit)?;
            eprintln!("frame {i} press={key} state={:?}", m.state());
            dump_frame(&m, i);
            i += 2;
            continue;
        }
        if !scripted && !args.no_keys {
            // Platform bring-up: LSK+OK at frame 5; periodic OK presses every
            // 10 frames afterwards (menus advance on OK/5).
            if i == 5 {
                m.set_key(lsk, true)?;
                m.run_frame(args.instruction_limit)?;
                m.set_key(lsk, false)?;
                m.run_frame(args.instruction_limit)?;
                m.set_key(ok, true)?;
                m.run_frame(args.instruction_limit)?;
                m.set_key(ok, false)?;
                m.run_frame(args.instruction_limit)?;
                i += 4;
                continue;
            }
            if i > 6 && (i - 6).is_multiple_of(10) {
                m.set_key(ok, true)?;
                m.run_frame(args.instruction_limit)?;
                m.set_key(ok, false)?;
            }
        }
        m.run_frame(args.instruction_limit)?;
        eprintln!("frame {i} state={:?}", m.state());
        dump_frame(&m, i);
        i += 1;
    }
    let mut key_frame = total;
    for k in &args.keys {
        m.set_key(*k, true)?;
        m.run_frame(args.instruction_limit)?;
        m.set_key(*k, false)?;
        m.run_frame(args.instruction_limit)?;
        if let Some(dir) = &args.dump_fb {
            let px = m.frame_pixels();
            let mut out = Vec::new();
            for p in &px {
                out.extend_from_slice(&p.to_le_bytes());
            }
            let _ = std::fs::write(dir.join(format!("frame_{key_frame:03}.raw")), &out);
        }
        key_frame += 2;
    }
    eprintln!(
        "gdi counters [fill,text,draw,clear,fill2,flush] = {:?}",
        m.gdi_counters()
    );
    let fb = m.frame_pixels();
    // Also report the guest framebuffer window (apps may draw there
    // directly instead of using the GDI bridge).
    let guest_fb = m.guest_fb_pixels();
    eprintln!(
        "guest fb nonzero: {} / {}",
        guest_fb.iter().filter(|&&p| p & 0xFFFFFF != 0).count(),
        guest_fb.len()
    );
    eprintln!(
        "done: frames={} fb={}px nonzero={}",
        m.frame_count(),
        fb.len(),
        fb.iter().filter(|&&p| p & 0xFFFFFF != 0).count()
    );

    for spec in &args.dump_mem {
        // Split into three parts only: the path itself may contain a drive
        // letter and therefore more colons.
        let mut it = spec.splitn(3, ':');
        let (Some(addr), Some(len), Some(path)) = (it.next(), it.next(), it.next()) else {
            eprintln!("--dump-mem wants addr:len:path, got {spec:?}");
            continue;
        };
        let (Ok(addr), Ok(len)) = (u32::from_str_radix(addr, 16), u32::from_str_radix(len, 16))
        else {
            eprintln!("--dump-mem address/length must be hex: {spec:?}");
            continue;
        };
        match m.dump_memory(addr, len) {
            Some(bytes) => {
                std::fs::write(path, &bytes)?;
                eprintln!("dumped {len:#x} bytes at {addr:#010x} -> {path}");
            }
            None => eprintln!("--dump-mem range {addr:#010x}+{len:#x} is unmapped"),
        }
    }

    Ok(())
}

/// Reads the sibling resource containers the shared discovery rule names
/// (`snowfishemu_core::res`, the same lookup the libretro core performs).
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
    let mut containers = Vec::new();
    for name in &candidates {
        let path = dir.join(name);
        if !path.is_file() {
            continue;
        }
        containers.push(std::fs::read(&path).with_context(|| format!("read {}", path.display()))?);
    }
    Ok(containers)
}
