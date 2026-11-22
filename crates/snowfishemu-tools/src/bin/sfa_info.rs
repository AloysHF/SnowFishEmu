//! Print `.app` package header information.

use anyhow::{Context, Result};
use clap::Parser;
use snowfishemu_core::sfa::SfaImage;
use std::path::PathBuf;

#[derive(Parser)]
struct Args {
    /// Path to a `.app` package.
    path: PathBuf,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let data =
        std::fs::read(&args.path).with_context(|| format!("read {}", args.path.display()))?;
    let img = SfaImage::parse(&data).map_err(anyhow::Error::from)?;
    let h = &img.header;
    println!("file:      {} ({} bytes)", args.path.display(), data.len());
    println!(
        "magic:     0x{:08X} ({})",
        h.magic,
        if h.magic == snowfishemu_core::sfa::MAGIC_SFA {
            "SFA"
        } else {
            "TFA"
        }
    );
    println!("version:   {}", h.version);
    println!("header:    0x{:X}", h.header_len);
    println!("code:      0x{:X} bytes", h.code_size);
    println!("bss:       0x{:X} bytes", h.bss_size);
    println!("image:     0x{:X} bytes", h.image_size());
    println!("relocs:    {} entries", h.reloc_count);
    Ok(())
}
