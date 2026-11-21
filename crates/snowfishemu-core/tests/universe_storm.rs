use snowfishemu_core::{lz, LzError, SnowFishMachine};

const BUDGET: u64 = 200_000_000;
const APP_CONTEXT: u32 = 0x14_6d94;
const LEFT_SOFT_KEY: u8 = 10;
const OK: u8 = 5;
const UP: u8 = 2;
const DOWN: u8 = 8;
const RIGHT: u8 = 6;

fn advance(machine: &mut SnowFishMachine, frames: u32) {
    for _ in 0..frames {
        machine.run_frame(BUDGET).unwrap();
    }
}

fn press(machine: &mut SnowFishMachine, key: u8) {
    machine.set_key(key, true).unwrap();
    advance(machine, 1);
    machine.set_key(key, false).unwrap();
    advance(machine, 1);
}

fn boot() -> SnowFishMachine {
    let path = std::env::var_os("SF_UNIVERSE_STORM_APP").expect("set SF_UNIVERSE_STORM_APP");
    let mut machine = SnowFishMachine::load(std::fs::read(path).unwrap(), &[]).unwrap();
    machine.set_free_play(true);
    machine.set_offline_http(true);
    machine.boot(BUDGET).unwrap();
    machine
}

#[test]
#[ignore = "requires the QVGA SnowFish V2 Universe Storm package (set SF_UNIVERSE_STORM_APP)"]
fn title_opens_readable_menu_and_new_game() {
    let mut machine = boot();
    advance(&mut machine, 30);
    press(&mut machine, LEFT_SOFT_KEY);
    advance(&mut machine, 28);

    let app = machine.mem.read_u32(APP_CONTEXT).unwrap();
    let menu = machine.mem.read_u32(app + 0x74).unwrap();
    assert_eq!(machine.mem.read_u32(menu).unwrap(), 2, "title scene");
    press(&mut machine, OK);
    advance(&mut machine, 26);
    assert_eq!(machine.mem.read_u32(menu).unwrap(), 3, "main menu scene");

    let frame = machine.frame_pixels();
    let glyph_pixels = (133..147)
        .flat_map(|y| &frame[y * 240 + 89..y * 240 + 151])
        .filter(|&&pixel| pixel == 0xfff8_fcf8)
        .count();
    assert!(
        (100..400).contains(&glyph_pixels),
        "New Game label should show white glyphs over the button, got {glyph_pixels} pixels"
    );

    press(&mut machine, DOWN);
    assert_eq!(machine.mem.read_u32(menu + 0x1b4).unwrap(), 1);
    press(&mut machine, UP);
    assert_eq!(machine.mem.read_u32(menu + 0x1b4).unwrap(), 0);
    advance(&mut machine, 6);
    press(&mut machine, OK);
    advance(&mut machine, 30);
    assert_eq!(machine.mem.read_u32(menu).unwrap(), 4, "game scene");

    let game = machine.mem.read_u32(menu + 0xf24).unwrap();
    assert_eq!(machine.mem.read_u32(game + 0x1b4).unwrap(), 1, "in flight");
    let ship_x = machine.mem.read_u32(game + 0xc).unwrap();
    let frame = machine.frame_pixels();
    press(&mut machine, RIGHT);
    assert!(machine.mem.read_u32(game + 0xc).unwrap() > ship_x);
    assert_ne!(machine.frame_pixels(), frame, "the playfield advances");
    advance(&mut machine, 180);
}

#[test]
#[ignore = "requires the QVGA SnowFish V2 Universe Storm package (set SF_UNIVERSE_STORM_APP)"]
fn first_level_loads_when_sdk_writes_overlap_the_bitmap_archive() {
    let mut machine = boot();
    // A billing state write for another SDK layout can corrupt this block.
    machine.mem.write_u32(0x13_31a0, 1).unwrap();
    let block = machine.dump_memory(0x13_2ba1, 1667).unwrap();
    assert_eq!(lz::decompress(&block, 0x4000), Err(LzError::SourceOverrun));

    advance(&mut machine, 30);
    press(&mut machine, LEFT_SOFT_KEY);
    advance(&mut machine, 28);
    press(&mut machine, OK);
    advance(&mut machine, 36);
    press(&mut machine, OK);
    advance(&mut machine, 30);

    let app = machine.mem.read_u32(APP_CONTEXT).unwrap();
    let menu = machine.mem.read_u32(app + 0x74).unwrap();
    assert_eq!(machine.mem.read_u32(menu).unwrap(), 4, "game scene");
    let game = machine.mem.read_u32(menu + 0xf24).unwrap();
    assert_eq!(machine.mem.read_u32(game + 0x1b4).unwrap(), 1, "in flight");
}
