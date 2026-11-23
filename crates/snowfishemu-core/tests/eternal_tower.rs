use std::ops::Range;

use snowfishemu_core::{plat::FB_WIDTH, SnowFishMachine};

const BUDGET: u64 = 200_000_000;
const LEFT_SOFT_KEY: u8 = 10;
const OK: u8 = 5;

fn boot_game() -> SnowFishMachine {
    let app = std::env::var_os("SF_ETERNAL_TOWER_APP").expect("set SF_ETERNAL_TOWER_APP");
    let mut machine = SnowFishMachine::load(std::fs::read(app).unwrap(), &[]).unwrap();
    machine.set_free_play(true);
    machine.set_offline_http(true);
    machine.boot(BUDGET).unwrap();
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

fn ink_bounds(pixels: &[u32], rows: Range<usize>, color: u32) -> [usize; 4] {
    let width = FB_WIDTH as usize;
    let [mut left, mut top, mut right, mut bottom] = [width, rows.end, 0, 0];
    for y in rows {
        for (x, &pixel) in pixels[y * width..(y + 1) * width].iter().enumerate() {
            if pixel == color {
                left = left.min(x);
                top = top.min(y);
                right = right.max(x);
                bottom = bottom.max(y);
            }
        }
    }
    [left, top, right, bottom]
}

#[test]
#[ignore = "requires the QVGA Eternal Tower package (set SF_ETERNAL_TOWER_APP)"]
fn music_prompt_and_title_art_use_their_anchors() {
    let mut machine = boot_game();
    run_to(&mut machine, 26, &[]);

    let prompt = machine.frame_pixels();
    assert_eq!(
        ink_bounds(&prompt, 120..200, 0xfff8_fcf8),
        [76, 152, 169, 167],
        "the music question is centered around its text anchor"
    );
    assert_eq!(
        ink_bounds(&prompt, 300..320, 0xfff8_fcf8),
        [24, 304, 214, 319],
        "the Yes and No labels fit inside their soft key positions"
    );

    run_to(&mut machine, 55, &[(30, LEFT_SOFT_KEY)]);

    let title = machine.frame_pixels();
    assert_eq!(
        ink_bounds(&title, 214..250, 0xfff8_ec38),
        [27, 215, 212, 248],
        "the complete title logo straddles the screen center"
    );
    assert_eq!(
        ink_bounds(&title, 280..300, 0xfff8_d428),
        [79, 282, 160, 297],
        "the selected Start Game artwork is centered between the arrows"
    );
}

#[test]
#[ignore = "requires the QVGA Eternal Tower package (set SF_ETERNAL_TOWER_APP)"]
fn offline_play_survives_the_trial_deadline() {
    const SDK_SCREEN: u32 = 0x14_6314;
    const TRIAL_ELAPSED: u32 = 0x14_a03c;
    const TOWER_SCENE: u32 = 0x14_9ec4;
    const SCENE_MODE: u32 = 0x14_9ef8;
    const PLAYER_X: u32 = 0x14_343c;
    const LEFT: u8 = 4;

    let mut machine = boot_game();
    let mut presses = vec![
        (30, LEFT_SOFT_KEY),
        (60, LEFT_SOFT_KEY),
        (90, LEFT_SOFT_KEY),
        (120, LEFT_SOFT_KEY),
    ];
    presses.extend((150..=360).step_by(15).map(|frame| (frame, OK)));
    run_to(&mut machine, 700, &presses);
    assert_eq!(machine.mem.read_u32(TOWER_SCENE).unwrap(), 4);
    assert_eq!(
        machine.mem.read_u32(SDK_SCREEN).unwrap(),
        0,
        "the tower stays active past the old tariff wait"
    );
    assert_eq!(
        machine.mem.read_u32(TRIAL_ELAPSED).unwrap(),
        0,
        "the offline trial countdown stays inactive"
    );

    // A snapshot made in timed mode can already have an expired counter.
    machine.mem.write_u32(TRIAL_ELAPSED, 60).unwrap();
    let confirm = (720..=819)
        .step_by(11)
        .map(|frame| (frame, OK))
        .collect::<Vec<_>>();
    run_to(&mut machine, 821, &confirm);
    assert_eq!(
        machine.mem.read_u32(SDK_SCREEN).unwrap(),
        0,
        "an expired trial counter does not open a tariff dialog"
    );
    assert_eq!(
        machine.mem.read_u8(SCENE_MODE).unwrap(),
        0x32,
        "Enter closes the dialogue and restores movement"
    );

    let x = machine.mem.read_u32(PLAYER_X).unwrap();
    run_to(
        &mut machine,
        844,
        &[(821, LEFT), (825, LEFT), (829, LEFT), (833, LEFT)],
    );
    assert_eq!(machine.mem.read_u32(PLAYER_X).unwrap(), x - 28);
}
