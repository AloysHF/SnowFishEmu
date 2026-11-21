use snowfishemu_core::SnowFishMachine;

const BUDGET: u64 = 200_000_000;
const SDK_SCREEN: u32 = 0x13_1744;
const SCENE: u32 = 0x12_E580;
const LEVEL: u32 = 0x12_E5F4;
const PLAY_PHASE: u32 = 0x12_E610;
const CLAW_STATE: u32 = 0x12_F244;
const OPTIONS_VISIBLE: u32 = 0x12_F53C;
const LEFT_SOFT_KEY: u8 = 10;
const RIGHT_SOFT_KEY: u8 = 25;
const OK: u8 = 5;

fn title_machine() -> SnowFishMachine {
    let path = std::env::var_os("SF_GOLD_MINER_APP").expect("set SF_GOLD_MINER_APP");
    let mut machine = SnowFishMachine::load(std::fs::read(path).unwrap(), &[]).unwrap();
    machine.boot(BUDGET).unwrap();
    for _ in 0..30 {
        machine.run_frame(BUDGET).unwrap();
    }
    assert_eq!(machine.mem.read_u32(SCENE).unwrap(), 1, "title scene");
    machine
}

fn press(machine: &mut SnowFishMachine, key: u8) {
    machine.set_key(key, true).unwrap();
    machine.run_frame(BUDGET).unwrap();
    machine.set_key(key, false).unwrap();
    machine.run_frame(BUDGET).unwrap();
}

fn assert_first_level(machine: &SnowFishMachine) {
    assert_eq!(machine.mem.read_u32(SDK_SCREEN).unwrap(), 0, "SDK overlay");
    assert_eq!(machine.mem.read_u32(SCENE).unwrap(), 2, "mine scene");
    assert_eq!(machine.mem.read_u32(PLAY_PHASE).unwrap(), 1, "active play");
    assert_eq!(machine.mem.read_u32(LEVEL).unwrap(), 1, "first level");
}

#[test]
#[ignore = "requires the QVGA SnowFish V2 Gold Miner package (set SF_GOLD_MINER_APP)"]
fn title_start_enters_first_level_with_one_press() {
    for key in [RIGHT_SOFT_KEY, OK] {
        let mut machine = title_machine();
        let title = machine.frame_pixels();
        press(&mut machine, key);
        assert_first_level(&machine);
        for _ in 0..4 {
            machine.run_frame(BUDGET).unwrap();
        }
        assert_ne!(machine.frame_pixels(), title);
        assert_eq!(machine.mem.read_u32(CLAW_STATE).unwrap(), 4, "claw ready");
        press(&mut machine, OK);
        assert_eq!(machine.mem.read_u32(CLAW_STATE).unwrap(), 1, "claw sent");
    }
}

#[test]
#[ignore = "requires the QVGA SnowFish V2 Gold Miner package (set SF_GOLD_MINER_APP)"]
fn options_start_keeps_the_local_level_menu() {
    let mut machine = title_machine();
    press(&mut machine, LEFT_SOFT_KEY);
    assert_eq!(machine.mem.read_u8(OPTIONS_VISIBLE).unwrap(), 1);
    assert_eq!(machine.mem.read_u32(SCENE).unwrap(), 1, "title scene");
    press(&mut machine, OK);
    machine.run_frame(BUDGET).unwrap();
    assert_eq!(machine.mem.read_u32(SDK_SCREEN).unwrap(), 0, "SDK overlay");
    assert_eq!(machine.mem.read_u32(SCENE).unwrap(), 2, "mine scene");
    assert_eq!(
        machine.mem.read_u32(PLAY_PHASE).unwrap(),
        0x32d,
        "level menu"
    );
    press(&mut machine, OK);
    assert_first_level(&machine);
}
