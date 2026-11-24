//! Regression: the titles' background music is standard MIDI. Universe
//! Storm carries its tune as an LZ-compressed block inside the package
//! image; decoding it through the platform's LZ decoder must produce a
//! length-prefixed SMF file (the format the +0x38 music slot hands to the
//! audio engine).

use snowfishemu_core::lz;

#[test]
#[ignore]
fn universe_storm_music_block_decodes_to_smf() {
    let path = std::env::var("SF_UNIVERSE_STORM_APP").expect("SF_UNIVERSE_STORM_APP");
    let package = std::fs::read(path).unwrap();
    // compat::universe_storm_lz_source: the audio LZ stream lives at guest
    // 0x12_2b94, 2217 bytes; the package file offset is src - CODE_BASE + header.
    let offset = 0x12_2b94usize - 0x0010_1000 + 0x20;
    let stream = &package[offset..offset + 2217];
    let out = lz::decompress(stream, 1 << 20).expect("lz decode");
    println!("decompressed {} bytes", out.len());
    println!("first 32 bytes: {:02x?}", &out[..out.len().min(32)]);
    let head = String::from_utf8_lossy(&out[..out.len().min(16)]).into_owned();
    println!("head as ascii: {head:?}");
    let mthd = out.windows(4).position(|w| w == b"MThd");
    println!("MThd at offset: {mthd:?}");
    assert!(mthd.is_some(), "expected a MIDI header in the audio block");
}
