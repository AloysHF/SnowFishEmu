//! Headless bring-up harness: boot a `.app`, pump events, report state.
//!
//! Used to validate guest bring-up without a window: prints the wrapper
//! set, handler registration, service-call counts, and the final guest
//! state byte.

use anyhow::{Context, Result};
use clap::Parser;
use snowfishemu_core::SnowFishMachine;
use std::path::PathBuf;

#[derive(Parser)]
struct Args {
    /// Path to a `.app` package.
    path: PathBuf,
    /// Optional sibling resource file served for `res`/`.pak` paths.
    #[arg(long)]
    res: Option<PathBuf>,
    /// Instruction budget per guest call.
    #[arg(long, default_value = "200000000")]
    budget: u64,
    /// Number of frames to pump after boot.
    #[arg(long, default_value = "5")]
    frames: u32,
    /// Queue a few key presses after boot (indices into the key map).
    #[arg(long)]
    keys: Vec<u8>,
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
    let mut m = SnowFishMachine::load(container, res)?;
    m.boot(args.budget)?;
    // Mimic the platform bring-up: 5 idle ticks, LSK down+up, OK down+up,
    // then the remaining frames (key indices per the core's key table).
    // --no-keys skips the replay for pure idle runs.
    let lsk = 10u8;
    let ok = 5u8;
    for i in 0..args.frames {
        // Platform bring-up: LSK+OK at frame 5; periodic OK presses every
        // 10 frames afterwards (menus advance on OK/5).
        if i == 5 && !args.no_keys {
            m.set_key(lsk, true)?;
            m.run_frame(args.budget)?;
            m.set_key(lsk, false)?;
            m.run_frame(args.budget)?;
            m.set_key(ok, true)?;
            m.run_frame(args.budget)?;
            m.set_key(ok, false)?;
            m.run_frame(args.budget)?;
            continue;
        }
        if i > 6 && (i - 6) % 10 == 0 && !args.no_keys {
            m.set_key(ok, true)?;
            m.run_frame(args.budget)?;
            m.set_key(ok, false)?;
        }
        m.run_frame(args.budget)?;
        eprintln!("frame {} state={:?}", i, m.state());
        if let Some(dir) = &args.dump_fb {
            let px = m.frame_pixels();
            let mut out = Vec::new();
            for p in &px {
                out.extend_from_slice(&p.to_le_bytes());
            }
            let _ = std::fs::write(dir.join(format!("frame_{i:03}.raw")), &out);
        }
    }
    let mut key_frame = args.frames;
    for k in &args.keys {
        m.set_key(*k, true)?;
        m.run_frame(args.budget)?;
        m.set_key(*k, false)?;
        m.run_frame(args.budget)?;
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
        let mut it = spec.split(':');
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
