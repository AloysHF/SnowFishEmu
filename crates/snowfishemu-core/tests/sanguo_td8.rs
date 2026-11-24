use std::path::PathBuf;

use snowfishemu_core::{plat::FB_WIDTH, res::sibling_res_candidates, SnowFishMachine};

#[test]
#[ignore = "requires the QVGA Three Kingdoms package and its sibling resources (set SF_SANGUO_TD8_APP)"]
fn country_and_upgrade_selection_render_correctly() {
    let app = PathBuf::from(std::env::var_os("SF_SANGUO_TD8_APP").expect("set SF_SANGUO_TD8_APP"));
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
    let budget = 200_000_000;
    let presses = [120, 300, 350, 400, 450];
    machine.boot(budget).unwrap();
    for frame in 0..455 {
        if presses.contains(&frame) {
            machine.set_key(10, true).unwrap();
        }
        if presses.contains(&(frame - 1)) {
            machine.set_key(10, false).unwrap();
        }
        machine.run_frame(budget).unwrap();
    }

    let pixels = machine.frame_pixels();
    let pixel = |x, y| pixels[y * FB_WIDTH as usize + x];
    let count_color = |xs: std::ops::Range<usize>, ys: std::ops::Range<usize>, color| {
        ys.flat_map(|y| xs.clone().map(move |x| pixel(x, y)))
            .filter(|&p| p == color)
            .count()
    };
    let softkey_ink = 0xff50_6488;
    assert_eq!(
        count_color(1..19, 310..320, softkey_ink),
        63,
        "confirm label"
    );
    assert_eq!(
        count_color(221..239, 310..320, softkey_ink),
        70,
        "cancel label"
    );
    assert_eq!(
        count_color(1..19, 280..305, softkey_ink),
        0,
        "left atlas spill"
    );
    assert_eq!(
        count_color(221..239, 280..305, softkey_ink),
        0,
        "right atlas spill"
    );

    let marker_ink = 0xff30_8860;
    assert!(
        count_color(104..123, 78..97, marker_ink) > 100,
        "city marker"
    );
    assert_eq!(
        count_color(85..104, 78..97, marker_ink),
        0,
        "adjacent atlas tile"
    );
    assert_eq!(
        pixel(133, 126),
        0xff78_d8c8,
        "map is visible through the portrait"
    );

    for frame in 455..555 {
        if frame == 500 {
            machine.set_key(10, true).unwrap();
        }
        if frame == 501 {
            machine.set_key(10, false).unwrap();
        }
        machine.run_frame(budget).unwrap();
    }

    let pixels = machine.frame_pixels();
    let width = FB_WIDTH as usize;
    let background = 0xfff8_fcc8;
    let top_row = &pixels[212 * width..213 * width];
    assert!(
        top_row[103..139].iter().all(|&p| p == background),
        "the centered backing covers two 16-pixel glyphs and four pixels of padding"
    );
    assert_ne!(top_row[102], background, "left end of the backing");
    assert_ne!(top_row[139], background, "right end of the backing");
    let label_ink = 0xffd8_4880;
    for x in [104, 120] {
        let ink = (213..229)
            .flat_map(|y| &pixels[y * width + x..y * width + x + 16])
            .filter(|&&p| p == label_ink)
            .count();
        assert!(ink > 20, "each character uses the guest's RGB888 pen");
    }
}
