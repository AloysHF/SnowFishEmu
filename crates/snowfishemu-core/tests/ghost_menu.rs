use snowfishemu_core::{plat::FB_WIDTH, SnowFishMachine};

const BUDGET: u64 = 200_000_000;
const SCENE: u32 = 0x60_5208;
const PLAYING: u32 = 2;
const PAUSED: u32 = 3;
const DIALOGUE: u32 = 12;
const PLAYER_X: u32 = 0x60_583a;
const NPC_CHECK_COMPLETE: u32 = 0x60_6212;
const LEFT: u8 = 4;
const OK: u8 = 5;
const RIGHT: u8 = 6;
const DOWN: u8 = 8;
const SOFT_LEFT: u8 = 10;
const MENU: u8 = 11;
const SOFT_RIGHT: u8 = 25;

fn start_cave() -> SnowFishMachine {
    let path = std::env::var_os("SF_GHOST_APP").expect("set SF_GHOST_APP");
    let mut machine = SnowFishMachine::load(std::fs::read(path).unwrap(), &[]).unwrap();
    machine.boot(BUDGET).unwrap();
    run_to(&mut machine, 75, &[(50, SOFT_RIGHT), (65, SOFT_LEFT)]);
    machine
}

fn run_to(machine: &mut SnowFishMachine, end: u64, presses: &[(u64, u8)]) {
    for frame in machine.frame_count()..end {
        for &(when, key) in presses {
            if frame == when || frame == when + 1 {
                machine.set_key(key, frame == when).unwrap();
            }
        }
        machine.run_frame(BUDGET).unwrap();
    }
}

#[test]
#[ignore = "requires the QVGA Ghost Blows Out the Light package (set SF_GHOST_APP)"]
fn cave_menu_draws_separate_rows_and_thin_borders() {
    let mut machine = start_cave();
    run_to(&mut machine, 100, &[(95, MENU)]);
    let pixels = machine.frame_pixels();
    let width = FB_WIDTH as usize;
    let label_ink = 0xfff0_fcf8;
    for (label, y) in [("Continue", 134), ("Help", 152), ("Exit", 170)] {
        let hits = (y..y + 14)
            .flat_map(|row| &pixels[row * width + 95..row * width + 145])
            .filter(|&&pixel| pixel == label_ink)
            .count();
        assert!(hits > 50, "{label} is visible on its own menu row");
    }
    for y in [148, 166] {
        assert!(!pixels[y * width + 95..y * width + 145].contains(&label_ink));
    }

    let border = 0xffa0_2c70;
    let panel = 0xff18_1450;
    let pixel = |y| pixels[y * width + 10];
    assert_eq!(pixel(188), panel);
    assert_eq!(pixel(189), border);
    assert_eq!(pixel(190), panel, "the border is one pixel high");
    assert_ne!(pixel(194), border, "the border does not cover the cave");
}

#[test]
#[ignore = "requires the QVGA Ghost Blows Out the Light package (set SF_GHOST_APP)"]
fn cave_menu_stays_closed_after_a_quick_confirm() {
    let mut machine = start_cave();
    let scene = |m: &SnowFishMachine| m.mem.read_u32(SCENE).unwrap();
    assert_eq!(scene(&machine), PLAYING, "the cave opens in gameplay");

    run_to(&mut machine, 101, &[(96, MENU)]);
    assert_eq!(
        scene(&machine),
        PAUSED,
        "a tap between polls opens the menu"
    );
    let player_x = machine.mem.read_u16(PLAYER_X).unwrap();
    run_to(&mut machine, 124, &[(105, OK), (114, RIGHT)]);
    assert_eq!(scene(&machine), PLAYING, "Continue leaves the menu closed");
    assert!(machine.mem.read_u16(PLAYER_X).unwrap() > player_x);

    run_to(&mut machine, 135, &[(127, MENU)]);
    assert_eq!(scene(&machine), PAUSED, "the menu can be reopened");
    run_to(&mut machine, 147, &[(136, SOFT_LEFT)]);
    assert_eq!(scene(&machine), PLAYING, "the left soft key also resumes");
}

#[test]
#[ignore = "requires the QVGA Ghost Blows Out the Light package (set SF_GHOST_APP)"]
fn movement_resumes_after_the_first_npc_dialogue() {
    let mut machine = start_cave();
    let approach: Vec<_> = (75..=87)
        .step_by(6)
        .map(|frame| (frame, DOWN))
        .chain((102..=138).step_by(6).map(|frame| (frame, RIGHT)))
        .chain([(151, OK)])
        .collect();
    run_to(&mut machine, 157, &approach);
    assert_eq!(machine.mem.read_u32(SCENE).unwrap(), DIALOGUE);

    for frame in (160..=322).step_by(9) {
        run_to(&mut machine, frame + 9, &[(frame, OK)]);
        if machine.mem.read_u8(NPC_CHECK_COMPLETE).unwrap() == 1 {
            break;
        }
    }
    assert_eq!(machine.mem.read_u32(SCENE).unwrap(), PLAYING);
    let player_x = machine.mem.read_u16(PLAYER_X).unwrap();
    let frame = machine.frame_count();
    run_to(&mut machine, frame + 11, &[(frame + 2, LEFT)]);
    assert!(
        machine.mem.read_u16(PLAYER_X).unwrap() < player_x,
        "the player can walk away"
    );
    assert_eq!(machine.mem.read_u8(NPC_CHECK_COMPLETE).unwrap(), 1);
}
