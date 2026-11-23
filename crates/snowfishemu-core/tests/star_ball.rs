use snowfishemu_core::{plat::FB_WIDTH, SnowFishMachine};

const BUDGET: u64 = 200_000_000;
const SCENE: u32 = 0x60_5e8c;
const LIVES: u32 = 0x60_5e9c;
const BALL_X: u32 = 0x60_5688;
const BALL_Y: u32 = 0x60_56a4;
const BALL_VERTICAL_SPEED: u32 = 0x60_56d4;
const PADDLE_X: u32 = 0x60_582c;
const MENU_SELECTION: u32 = 0x60_614c;
const LEVEL_SELECTION: u32 = 0x60_6160;
const MENU: u32 = 3;
const LEVEL_SELECT: u32 = 4;
const PLAYING: u32 = 5;
const UP: u8 = 2;
const LEFT: u8 = 4;
const OK: u8 = 5;
const RIGHT: u8 = 6;
const DOWN: u8 = 8;

fn start_game() -> SnowFishMachine {
    let path = std::env::var_os("SF_STAR_BALL_APP").expect("set SF_STAR_BALL_APP");
    let mut machine = SnowFishMachine::load(std::fs::read(path).unwrap(), &[]).unwrap();
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

#[test]
#[ignore = "requires the QVGA StarBall package (set SF_STAR_BALL_APP)"]
fn last_ball_replenishes_the_paddles_and_play_continues_offline() {
    let mut machine = start_game();
    let read = |m: &SnowFishMachine, addr| m.mem.read_u32(addr).unwrap();

    run_to(&mut machine, 80, &[(30, OK), (60, OK)]);
    assert_eq!(read(&machine, SCENE), PLAYING);
    let ball_y = read(&machine, BALL_Y);
    run_to(&mut machine, 94, &[(90, OK)]);
    assert!(
        read(&machine, BALL_Y) < ball_y,
        "the third Enter launches the ball"
    );

    for (end, presses, lives) in [(115, &[][..], 2), (145, &[(120, OK)][..], 1)] {
        run_to(&mut machine, end, presses);
        assert_eq!(read(&machine, SCENE), PLAYING);
        assert_eq!(read(&machine, LIVES), lives, "one miss costs one life");
        assert_eq!(
            read(&machine, BALL_VERTICAL_SPEED),
            0,
            "the next ball waits for a serve"
        );
    }
    run_to(&mut machine, 175, &[(150, OK)]);
    assert_eq!(read(&machine, SCENE), PLAYING, "the current round resumes");
    assert_eq!(
        read(&machine, LIVES),
        3,
        "the native callback replenishes the paddles"
    );
    assert_eq!(read(&machine, BALL_VERTICAL_SPEED), 0);
    let paddle_x = read(&machine, PADDLE_X);
    let ball_x = read(&machine, BALL_X);
    run_to(&mut machine, 228, &[(213, RIGHT)]);
    assert!(
        read(&machine, PADDLE_X) > paddle_x,
        "Right moves the paddle"
    );
    assert!(
        read(&machine, BALL_X) > ball_x,
        "the ready ball follows the paddle"
    );

    let ball_y = read(&machine, BALL_Y);
    run_to(&mut machine, 244, &[(240, OK)]);
    assert_eq!(read(&machine, SCENE), PLAYING);
    assert!(
        read(&machine, BALL_Y) < ball_y,
        "Enter launches the new ball"
    );
}

#[test]
#[ignore = "requires the QVGA StarBall package (set SF_STAR_BALL_APP)"]
fn first_paddle_return_keeps_the_round_in_play() {
    let mut machine = start_game();
    let read = |m: &SnowFishMachine, addr| m.mem.read_u32(addr).unwrap();
    run_to(&mut machine, 90, &[(30, OK), (60, OK)]);
    let serve_x = read(&machine, BALL_X);
    run_to(&mut machine, 92, &[(90, OK)]);

    let steer = if read(&machine, BALL_X) < serve_x {
        LEFT
    } else {
        RIGHT
    };
    run_to(
        &mut machine,
        99,
        &[(92, steer), (94, steer), (96, steer), (98, steer)],
    );
    assert!((read(&machine, BALL_VERTICAL_SPEED) as i32) > 0);
    run_to(&mut machine, 102, &[(100, steer)]);
    assert!(
        (read(&machine, BALL_VERTICAL_SPEED) as i32) < 0,
        "the paddle returns the ball"
    );
    run_to(&mut machine, 110, &[]);
    assert_eq!(
        read(&machine, SCENE),
        PLAYING,
        "the rally continues after the return"
    );
    assert_eq!(
        read(&machine, LIVES),
        3,
        "the first return keeps all three lives"
    );
}

#[test]
#[ignore = "requires the QVGA StarBall package (set SF_STAR_BALL_APP)"]
fn dpad_selects_a_menu_item_and_changes_the_level() {
    let mut machine = start_game();
    run_to(&mut machine, 39, &[(30, OK)]);
    assert_eq!(machine.mem.read_u32(SCENE).unwrap(), MENU);
    assert_eq!(machine.mem.read_u32(MENU_SELECTION).unwrap(), 0);

    for (frame, key, selected) in [(40, DOWN, 1), (50, DOWN, 2), (60, UP, 1)] {
        run_to(&mut machine, frame + 5, &[(frame, key)]);
        assert_eq!(machine.mem.read_u32(MENU_SELECTION).unwrap(), selected);
    }
    run_to(&mut machine, 76, &[(70, OK)]);
    assert_eq!(machine.mem.read_u32(SCENE).unwrap(), LEVEL_SELECT);

    for (frame, key, selected) in [(80, RIGHT, 1), (90, LEFT, 0), (100, DOWN, 4), (110, UP, 0)] {
        run_to(&mut machine, frame + 5, &[(frame, key)]);
        assert_eq!(machine.mem.read_u32(LEVEL_SELECTION).unwrap(), selected);
    }
    run_to(&mut machine, 130, &[(120, OK)]);
    assert_eq!(machine.mem.read_u32(SCENE).unwrap(), PLAYING);
}

#[test]
#[ignore = "requires the QVGA StarBall package (set SF_STAR_BALL_APP)"]
fn level_selection_outlines_only_the_selected_number() {
    let mut machine = start_game();
    run_to(&mut machine, 78, &[(30, OK), (40, DOWN), (60, OK)]);
    assert_eq!(machine.mem.read_u32(SCENE).unwrap(), LEVEL_SELECT);

    let assert_outline = |machine: &SnowFishMachine, x, y| {
        let pixels = machine.frame_pixels();
        let pixel = |x, y| pixels[y * FB_WIDTH as usize + x];
        let red = 0xfff8_0000;
        assert_eq!(
            pixels.iter().filter(|&&color| color == red).count(),
            72,
            "the red cursor draws one 22 by 16 outline"
        );
        assert_eq!(pixel(x, y), red);
        assert_eq!(pixel(x + 21, y + 15), red);
        assert_ne!(pixel(x + 1, y + 1), red);
    };
    assert_outline(&machine, 35, 61);
    let pixels = machine.frame_pixels();
    let width = FB_WIDTH as usize;
    let number_ink = 0xffe0_d8d8;
    for left in [46, 96, 146, 196] {
        assert!(
            (62..74).any(|y| pixels[y * width + left..y * width + left + 9].contains(&number_ink)),
            "the first row still shows its numbers"
        );
    }

    run_to(&mut machine, 92, &[(85, RIGHT)]);
    assert_outline(&machine, 85, 61);
    run_to(&mut machine, 108, &[(100, DOWN)]);
    assert_outline(&machine, 85, 83);
}
