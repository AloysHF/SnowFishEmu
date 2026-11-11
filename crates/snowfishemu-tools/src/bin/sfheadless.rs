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
    /// Dump the framebuffer to a BMP-ish raw file after the run.
    #[arg(long)]
    dump_fb: Option<PathBuf>,
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
    for i in 0..args.frames {
        m.run_frame(args.budget)?;
        eprintln!("frame {} state={:?}", i, m.state());
    }
    for k in &args.keys {
        m.set_key(*k, true)?;
        m.run_frame(args.budget)?;
        m.set_key(*k, false)?;
        m.run_frame(args.budget)?;
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
    if let Some(dump) = args.dump_fb {
        let mut out = Vec::new();
        for px in &fb {
            out.extend_from_slice(&px.to_le_bytes());
        }
        std::fs::write(&dump, &out)?;
        eprintln!("framebuffer written to {}", dump.display());
    }
    Ok(())
}
