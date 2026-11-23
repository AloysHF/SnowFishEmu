use std::{ops::Range, path::PathBuf};

use snowfishemu_core::{plat::FB_WIDTH, res::sibling_res_candidates, SnowFishMachine};

const BUDGET: u64 = 200_000_000;
const OK: u8 = 5;
const SOFT_LEFT: u8 = 10;
const TEXT_INK: u32 = 0xfff8_fcf8;

fn count_color(pixels: &[u32], xs: Range<usize>, ys: Range<usize>, color: u32) -> usize {
    let width = FB_WIDTH as usize;
    ys.flat_map(|y| &pixels[y * width + xs.start..y * width + xs.end])
        .filter(|&&pixel| pixel == color)
        .count()
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

fn boot_game() -> SnowFishMachine {
    let app = PathBuf::from(std::env::var_os("SF_YZXYL_APP").expect("set SF_YZXYL_APP"));
    let dir = app.parent().unwrap();
    let names = std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|entry| entry.ok()?.file_name().into_string().ok());
    let resources: Vec<_> = sibling_res_candidates(
        app.file_stem().and_then(|stem| stem.to_str()),
        app.file_name().unwrap().to_str().unwrap(),
        names,
    )
    .into_iter()
    .map(|name| std::fs::read(dir.join(name)).unwrap())
    .collect();
    let mut machine = SnowFishMachine::load(std::fs::read(app).unwrap(), &resources).unwrap();
    machine.boot(BUDGET).unwrap();
    machine
}

#[test]
#[ignore = "requires the QVGA yzxyl package and its sibling resources (set SF_YZXYL_APP)"]
fn music_prompt_story_and_first_room_render() {
    let mut machine = boot_game();
    run_to(&mut machine, 1, &[]);

    let startup = machine.frame_pixels();
    let label_ink = 0xff30_a4c8;
    assert!(
        count_color(&startup, 65..176, 138..158, label_ink) > 400,
        "the first frame shows the music prompt"
    );
    assert!(
        count_color(&startup, 0..24, 298..320, label_ink) > 60,
        "Yes"
    );
    assert!(
        count_color(&startup, 212..240, 298..320, label_ink) > 60,
        "No"
    );
    assert!(
        count_color(&startup, 97..144, 183..240, 0xff00_84f0) > 200,
        "the blue music note is visible"
    );

    run_to(&mut machine, 70, &[(30, SOFT_LEFT), (60, OK)]);
    let story = machine.frame_pixels();
    for row in [136, 153, 170] {
        assert!(
            count_color(&story, 0..240, row..row + 16, TEXT_INK) > 300,
            "the opening story has text on row {row}"
        );
    }
    assert!(
        count_color(&story, 144..160, 170..186, TEXT_INK) > 0,
        "the closing ellipsis is visible"
    );

    run_to(
        &mut machine,
        200,
        &[
            (90, OK),
            (120, SOFT_LEFT),
            (150, SOFT_LEFT),
            (180, SOFT_LEFT),
        ],
    );
    let room = machine.frame_pixels();
    assert!(
        count_color(&room, 22..215, 35..145, 0xfff8_fce8) > 4_500,
        "room walls"
    );
    assert!(
        count_color(&room, 22..215, 210..290, 0xffd0_b088) > 3_000,
        "room floor"
    );
}

#[test]
#[ignore = "requires the QVGA yzxyl package and its sibling resources (set SF_YZXYL_APP)"]
fn first_room_notices_fit_inside_their_panel() {
    let mut machine = boot_game();
    run_to(&mut machine, 110, &[(30, SOFT_LEFT), (60, OK), (90, OK)]);

    let prompt = machine.frame_pixels();
    let border = 0xffd0_9c08;
    for row in [144, 175] {
        assert_eq!(
            count_color(&prompt, 20..220, row..row + 1, border),
            200,
            "the prompt panel encloses a full line of text"
        );
    }
    assert!(
        count_color(&prompt, 65..115, 145..175, TEXT_INK) > 150,
        "the dialogue cue starts left of center"
    );
    assert!(
        count_color(&prompt, 151..155, 163..167, TEXT_INK) > 0,
        "the cue includes its final full-width period"
    );
    assert_eq!(count_color(&prompt, 48..192, 175..193, TEXT_INK), 0);

    run_to(&mut machine, 145, &[(120, OK)]);
    let tutorial = machine.frame_pixels();
    for row in [125, 194] {
        assert_eq!(count_color(&tutorial, 20..220, row..row + 1, border), 200);
    }
    for (row, minimum) in [(134, 500), (151, 500), (168, 50)] {
        assert!(
            count_color(&tutorial, 5..235, row..row + 16, TEXT_INK) > minimum,
            "the movement tutorial renders row {row} inside its panel"
        );
    }
}
