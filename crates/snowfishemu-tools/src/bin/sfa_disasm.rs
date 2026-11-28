//! Disassemble a Thumb or ARM range from a `.app` package.

use anyhow::{bail, Context, Result};
use clap::Parser;
use snowfishemu_core::consts::CODE_BASE;
use snowfishemu_core::sfa::SfaImage;
use std::path::PathBuf;
use unarm::{parse_arm, parse_thumb, Options};

#[derive(Parser)]
#[command(about = "Disassemble a Thumb or ARM range from a SnowFish .app package")]
struct Args {
    /// Path to a `.app` package.
    path: PathBuf,
    /// Guest address to start at (defaults to the code base 0x101000).
    #[arg(long, value_parser = parse_u32)]
    address: Option<u32>,
    /// Number of bytes to disassemble (decimal or 0x-prefixed hex).
    #[arg(long, default_value_t = 0x100, value_parser = parse_u32)]
    length: u32,
    /// Disassemble Thumb instead of ARM.
    #[arg(long)]
    thumb: bool,
}

// Accept both decimal and 0x-prefixed hexadecimal values.
fn parse_u32(s: &str) -> Result<u32, String> {
    let text = s.trim();
    if let Some(hex) = text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
        u32::from_str_radix(hex, 16).map_err(|e| e.to_string())
    } else {
        text.parse::<u32>().map_err(|e| e.to_string())
    }
}

fn main() -> Result<()> {
    let args = Args::parse();
    let data =
        std::fs::read(&args.path).with_context(|| format!("read {}", args.path.display()))?;
    let img = SfaImage::parse(&data).map_err(anyhow::Error::from)?;

    // The parsed code segment is relocated, so literal pools show the
    // addresses the guest actually reads at runtime.
    let start = args.address.unwrap_or(CODE_BASE);
    if start < CODE_BASE {
        bail!("address {start:#010x} precedes the code base {CODE_BASE:#010x}");
    }
    let relative = (start - CODE_BASE) as usize;
    if relative >= img.code.len() {
        bail!(
            "address {start:#010x} is past the code segment (base {CODE_BASE:#010x}, size {:#x})",
            img.code.len()
        );
    }

    // Boot and the wrapper stublets are ARM code; games switch to Thumb
    // through the interworking branches. ARM addresses are word-aligned,
    // Thumb addresses halfword-aligned.
    let (mut offset, mut address) = if args.thumb {
        (relative & !1, start & !1)
    } else {
        (relative & !3, start & !3)
    };
    let end = (offset + args.length as usize).min(img.code.len());
    if offset >= end {
        bail!("disassembly range is outside the code segment");
    }

    let options = Options::default();
    if args.thumb {
        while offset + 2 <= end {
            let first = u16::from_le_bytes([img.code[offset], img.code[offset + 1]]);
            let second = if offset + 4 <= end {
                u16::from_le_bytes([img.code[offset + 2], img.code[offset + 3]])
            } else {
                0
            };
            let (ins, size) =
                parse_thumb(first as u32 | ((second as u32) << 16), address, &options);
            let size = size as usize;
            // Do not print a 4-byte instruction whose second halfword was
            // clipped by the range end.
            if offset + size > end {
                break;
            }
            let raw = if size == 4 {
                format!("{first:04X}{second:04X}")
            } else {
                format!("{first:04X}    ")
            };
            println!("{address:08X}: {raw}  {}", ins.display(&options));
            offset += size;
            address = address.wrapping_add(size as u32);
        }
    } else {
        while offset + 4 <= end {
            let word = u32::from_le_bytes([
                img.code[offset],
                img.code[offset + 1],
                img.code[offset + 2],
                img.code[offset + 3],
            ]);
            let ins = parse_arm(word, address, &options);
            println!("{address:08X}: {word:08X}  {}", ins.display(&options));
            offset += 4;
            address = address.wrapping_add(4);
        }
    }
    Ok(())
}
