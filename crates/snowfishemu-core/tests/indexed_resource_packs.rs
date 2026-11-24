use std::collections::HashSet;
use std::path::PathBuf;

use snowfishemu_core::{res::sibling_res_candidates, SnowFishMachine};

fn assert_indexed_resource_title(package_env: &str) {
    const BUDGET: u64 = 200_000_000;

    let path = PathBuf::from(std::env::var_os(package_env).expect("set the package path"));
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
    for _ in 0..200 {
        game.run_frame(BUDGET).unwrap();
    }
    let colors: HashSet<_> = game.frame_pixels().into_iter().collect();
    assert!(
        colors.len() > 100,
        "the indexed resources must load and render the title artwork"
    );
}

#[test]
#[ignore = "requires the QVGA Juedai Shuangjiao package and its sibling resources (set SF_JDSJ_APP)"]
fn juedai_shuangjiao_loads_its_indexed_resources() {
    assert_indexed_resource_title("SF_JDSJ_APP");
}
