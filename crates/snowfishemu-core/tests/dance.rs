use std::collections::HashSet;

use snowfishemu_core::SnowFishMachine;

#[test]
#[ignore = "requires the QVGA Dance package (set SF_DANCE_APP)"]
fn startup_renders_a_visible_title() {
    const BUDGET: u64 = 200_000_000;
    let path = std::env::var_os("SF_DANCE_APP").expect("set SF_DANCE_APP");
    let mut machine = SnowFishMachine::load(std::fs::read(path).unwrap(), &[]).unwrap();
    machine.boot(BUDGET).unwrap();
    for _ in 0..200 {
        machine.run_frame(BUDGET).unwrap();
    }

    let colors: HashSet<_> = machine.frame_pixels().into_iter().collect();
    assert!(colors.len() > 100, "the game's title artwork must render");
}
