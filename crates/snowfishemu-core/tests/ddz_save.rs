use std::collections::HashSet;
use std::path::PathBuf;

use snowfishemu_core::{res::sibling_res_candidates, SnowFishMachine};

#[test]
#[ignore = "requires the QVGA ddz package and its sibling resources (set SF_DDZ_APP)"]
fn first_boot_reaches_the_title_without_a_save() {
    const BUDGET: u64 = 200_000_000;
    let path = PathBuf::from(std::env::var_os("SF_DDZ_APP").expect("set SF_DDZ_APP"));
    let dir = path.parent().unwrap();
    let names = std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|entry| entry.ok()?.file_name().into_string().ok());
    let resources: Vec<_> = sibling_res_candidates(
        path.file_stem().and_then(|stem| stem.to_str()),
        path.file_name().unwrap().to_str().unwrap(),
        names,
    )
    .into_iter()
    .map(|name| std::fs::read(dir.join(name)).unwrap())
    .collect();

    let mut game = SnowFishMachine::load(std::fs::read(path).unwrap(), &resources).unwrap();
    game.boot(BUDGET).unwrap();
    for frame in 0..200 {
        if frame == 30 {
            let audio_prompt: HashSet<_> = game.frame_pixels().into_iter().collect();
            assert!(
                audio_prompt.len() > 20,
                "a fresh boot must show the audio prompt"
            );
            game.set_key(10, true).unwrap();
        } else if frame == 32 {
            game.set_key(10, false).unwrap();
        }
        game.run_frame(BUDGET).unwrap();
    }
    let title: HashSet<_> = game.frame_pixels().into_iter().collect();
    assert!(
        title.len() > 100,
        "accepting audio must render the title artwork"
    );
}
