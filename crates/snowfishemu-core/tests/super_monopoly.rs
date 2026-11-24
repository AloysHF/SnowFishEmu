use std::collections::HashSet;

use snowfishemu_core::{plat::FB_WIDTH, SnowFishMachine};

#[test]
#[ignore = "requires the QVGA Super Monopoly package (set SF_SUPER_MONOPOLY_APP)"]
fn starts_a_new_game_without_dispatching_a_data_pointer() {
    const BUDGET: u64 = 200_000_000;
    let path = std::env::var_os("SF_SUPER_MONOPOLY_APP").expect("set SF_SUPER_MONOPOLY_APP");
    let mut machine = SnowFishMachine::load(std::fs::read(path).unwrap(), &[]).unwrap();
    machine.boot(BUDGET).unwrap();

    let presses = [(30, 10), (60, 10), (90, 5), (120, 5), (150, 10), (180, 5)];
    for frame in 0..200 {
        for (when, key) in presses {
            if frame == when || frame == when + 2 {
                machine.set_key(key, frame == when).unwrap();
            }
        }
        machine.run_frame(BUDGET).unwrap_or_else(|error| {
            panic!("Super Monopoly must reach its board. Frame {frame}: {error}")
        });
    }

    let pixels = machine.frame_pixels();
    let board = &pixels[120 * FB_WIDTH as usize..280 * FB_WIDTH as usize];
    let grass = board.iter().filter(|&&pixel| {
        let [_, red, green, blue] = pixel.to_be_bytes();
        green > 180 && green > red && red > 100 && blue < 130
    });
    assert!(
        grass.count() > 15_000,
        "the board's grass tiles must render"
    );
    assert!(
        board.iter().collect::<HashSet<_>>().len() > 70,
        "the board must include the map and sprites"
    );
}
