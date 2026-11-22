// libretro API implementation for the SnowFish/SFA emulator.

#![allow(static_mut_refs)]
#![allow(clippy::not_unsafe_ptr_arg_deref)]

use super::callbacks;
use super::constants::*;
use super::options;
use super::types::*;
use snowfishemu_core::SnowFishMachine;
use std::ffi::{c_char, c_void, CStr};
use std::path::{Path, PathBuf};
use std::ptr;

const DISPLAY_WIDTH: u32 = 240;
const DISPLAY_HEIGHT: u32 = 320;
/// Fixed portrait display bounds.
const MAX_DISPLAY_WIDTH: u32 = DISPLAY_WIDTH;
const MAX_DISPLAY_HEIGHT: u32 = DISPLAY_HEIGHT;
/// Frontend pacing rate reported through the AV info.
///
/// The rate only decides how often the frontend calls `retro_run`; the
/// guest still advances its own 20 Hz screen tick on every
/// [`PACING_FPS`] / [`GUEST_TICK_RATE`] th call. It must be a rate the
/// frontend can pace on its own: RetroArch derives the automatic swap
/// interval from this value and caps it at 4, so reporting the 20 Hz
/// guest rate made the game run at the display refresh rate. 60 Hz matches
/// the common display refresh rates exactly, and the audio stream always
/// flows at this rate (silence-padded — this platform exposes no audio
/// service), so audio-sync pacing keeps the speed exact everywhere else.
const PACING_FPS: u32 = 60;
/// Guest screen tick rate: the platform's ~50 ms frame tick.
const GUEST_TICK_RATE: u32 = 20;
/// Stereo frames submitted to the frontend per `retro_run` call. The
/// constant silent flow keeps the frontend pacing on its audio clock even
/// though this platform has no audio service.
const PACING_SAMPLES_PER_RUN: usize = (AUDIO_SAMPLE_RATE / PACING_FPS) as usize;
/// Reported audio sample rate (no guest audio exists; silence only).
const AUDIO_SAMPLE_RATE: u32 = 44_100;
const PERFORMANCE_LEVEL: u32 = 3;
/// Instruction budget per guest call (boot, event dispatch, frame tick).
/// Guest calls normally return at the stop sentinel long before this; the
/// budget only bounds a runaway guest.
const INSTRUCTION_BUDGET: u64 = 100_000_000;
/// Fixed capacity the frontend allocates for save states. The guest memory
/// map is bounded by the loader constants (at most ~5.25 MiB of regions),
/// comfortably below this bound.
const SAVE_STATE_MAX_SIZE: usize = 8 * 1024 * 1024;

/// Converts frontend frames into guest screen ticks.
///
/// Each `retro_run` call contributes `GUEST_TICK_RATE / PACING_FPS` of a
/// guest tick; the tick runs when the accumulated credit reaches a whole
/// tick. This keeps the guest rate exact at `GUEST_TICK_RATE` ticks per
/// second regardless of how often the frontend paces the core.
struct GuestTickPacer {
    /// Outstanding credit in 1/PACING_FPS frontend-frame units.
    credit: u32,
}

impl GuestTickPacer {
    fn new() -> Self {
        Self { credit: 0 }
    }

    /// Record one frontend frame and return the due guest tick count.
    fn advance(&mut self) -> u32 {
        self.credit += GUEST_TICK_RATE;
        let mut ticks = 0;
        while self.credit >= PACING_FPS {
            self.credit -= PACING_FPS;
            ticks += 1;
        }
        ticks
    }
}

/// Guest key indices driven by the frontends, mirroring the standalone
/// frontend's key map (see the core's `KEY_MAP` for index to device code).
mod keys {
    pub const UP: u8 = 13;
    pub const DOWN: u8 = 14;
    pub const LEFT: u8 = 15;
    pub const RIGHT: u8 = 16;
    pub const OK: u8 = 5;
    pub const SOFT_L: u8 = 10;
    pub const MENU: u8 = 11;
    pub const SOFT_R: u8 = 25;
}

/// The guest key indices the libretro input mapping can press, in a fixed
/// order so the pad and keyboard mappings can be merged per index.
const MAPPED_KEYS: [u8; 17] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 14, 15, 16, 25];

/// Maps a digit (0-9) onto its guest key index: digits 1-9 are indices 0-8
/// and digit 0 is index 9, matching the standalone frontend's keypad.
fn digit_index(digit: u32) -> u8 {
    if digit == 0 {
        9
    } else {
        (digit - 1) as u8
    }
}

/// Loaded emulator state shared by the libretro entry points.
struct Emulator {
    /// The loaded package bytes; kept for `retro_reset` and as the file
    /// service backing store of a reloaded machine.
    container: Vec<u8>,
    /// The sibling resource containers whose packs back `res`/`.pak` file
    /// opens and the download the guest would otherwise make.
    res_data: Vec<Vec<u8>>,
    machine: SnowFishMachine,
    /// Set when a guest call faulted; the last valid frame keeps being
    /// presented and the guest stops advancing.
    stopped: bool,
    /// Last reported press state per guest key index. The core queues an
    /// event per `set_key` change, so polls must report edges only.
    key_state: [bool; 32],
    /// Guest tick credit accumulated from the frontend pacing rate.
    tick_pacer: GuestTickPacer,
}

/// Global emulator instance.
static mut EMULATOR: Option<Emulator> = None;

/// Run `body`, turning a panic into `fallback`.
///
/// A panic must never unwind across the C ABI: the frontend is a host process
/// (RetroArch, a mobile app) that would be torn down with us. Swallowing the
/// panic keeps the session recoverable and leaves a log entry behind.
fn guard<T>(fallback: T, body: impl FnOnce() -> T) -> T {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(body)) {
        Ok(value) => value,
        Err(payload) => {
            let detail = payload
                .downcast_ref::<&str>()
                .map(|text| (*text).to_owned())
                .or_else(|| payload.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "unknown panic".to_owned());
            log::error!("SnowFishEmu libretro entry point panicked: {detail}");
            fallback
        }
    }
}

// ============================================================
// Callback registration
// ============================================================

#[no_mangle]
pub extern "C" fn retro_set_environment(cb: retro_environment_t) {
    callbacks::set_environment(cb);
    // Declare core options as early as possible so the frontend can show them
    // before any content is loaded.
    options::set_core_options();
}

#[no_mangle]
pub extern "C" fn retro_set_video_refresh(cb: retro_video_refresh_t) {
    callbacks::set_video_refresh(cb);
}

#[no_mangle]
pub extern "C" fn retro_set_audio_sample(cb: retro_audio_sample_t) {
    callbacks::set_audio_sample(cb);
}

#[no_mangle]
pub extern "C" fn retro_set_audio_sample_batch(cb: retro_audio_sample_batch_t) {
    callbacks::set_audio_sample_batch(cb);
}

#[no_mangle]
pub extern "C" fn retro_set_input_poll(cb: retro_input_poll_t) {
    callbacks::set_input_poll(cb);
}

#[no_mangle]
pub extern "C" fn retro_set_input_state(cb: retro_input_state_t) {
    callbacks::set_input_state(cb);
}

#[no_mangle]
pub extern "C" fn retro_set_controller_port_device(_port: u32, _device: u32) {
    // The feature-phone keypad is exposed as a single RetroPad.
}

// ============================================================
// Lifecycle
// ============================================================

#[no_mangle]
pub extern "C" fn retro_api_version() -> u32 {
    RETRO_API_VERSION
}

#[no_mangle]
pub extern "C" fn retro_init() {
    guard((), || {
        callbacks::init_log();
        super::logger::init();
        log::info!("SnowFishEmu libretro core initialized");
    })
}

#[no_mangle]
pub extern "C" fn retro_deinit() {
    guard((), || {
        unsafe {
            EMULATOR = None;
        }
        log::info!("SnowFishEmu libretro core deinitialized");
    })
}

#[no_mangle]
pub extern "C" fn retro_get_system_info(info: *mut retro_system_info) {
    guard((), || unsafe {
        (*info) = retro_system_info {
            library_name: c"SnowFishEmu".as_ptr(),
            library_version: concat!(env!("CARGO_PKG_VERSION"), "\0").as_ptr() as *const c_char,
            valid_extensions: c"app".as_ptr(),
            need_fullpath: true,
            block_extract: false,
        };
    })
}

#[no_mangle]
pub extern "C" fn retro_get_system_av_info(info: *mut retro_system_av_info) {
    guard((), || unsafe {
        (*info) = retro_system_av_info {
            geometry: retro_game_geometry {
                base_width: DISPLAY_WIDTH,
                base_height: DISPLAY_HEIGHT,
                max_width: MAX_DISPLAY_WIDTH,
                max_height: MAX_DISPLAY_HEIGHT,
                aspect_ratio: DISPLAY_WIDTH as f32 / DISPLAY_HEIGHT as f32,
            },
            timing: retro_system_timing {
                fps: PACING_FPS as f64,
                sample_rate: AUDIO_SAMPLE_RATE as f64,
            },
        };
    })
}

// ============================================================
// Content lifecycle
// ============================================================

#[no_mangle]
pub extern "C" fn retro_load_game(info: *const retro_game_info) -> bool {
    guard(false, || unsafe {
        if info.is_null() {
            return fail(MISSING_CONTENT_PATH);
        }
        let game_info = &*info;
        if game_info.path.is_null() {
            return fail(MISSING_CONTENT_PATH);
        }
        let path = match CStr::from_ptr(game_info.path).to_str() {
            Ok(path) => path,
            Err(error) => {
                return fail(&format!(
                    "SnowFish: the content path is not valid UTF-8 ({error})."
                ));
            }
        };

        let pixel_format = retro_pixel_format::RETRO_PIXEL_FORMAT_XRGB8888;
        if !callbacks::environment(
            RETRO_ENVIRONMENT_SET_PIXEL_FORMAT,
            &pixel_format as *const _ as *mut c_void,
        ) {
            return fail("SnowFish: this frontend does not support XRGB8888 video.");
        }

        register_input_descriptors();

        callbacks::environment(
            RETRO_ENVIRONMENT_SET_PERFORMANCE_LEVEL,
            &PERFORMANCE_LEVEL as *const _ as *mut c_void,
        );

        match load_machine(Path::new(path)) {
            Ok((container, res_data, machine)) => {
                log::info!("Game loaded: {path}");
                EMULATOR = Some(Emulator {
                    container,
                    res_data,
                    machine,
                    stopped: false,
                    key_state: [false; 32],
                    tick_pacer: GuestTickPacer::new(),
                });
                if let Some(emulator) = EMULATOR.as_mut() {
                    apply_core_options();
                    register_memory_maps(emulator);
                }
                true
            }
            Err(error) => fail(&format!(
                "SnowFish: failed to load this game.\n{error:#}\n\nSupported content: a .app application file."
            )),
        }
    })
}

const MISSING_CONTENT_PATH: &str =
    "The frontend passed no content path; this core loads content from a file path only.";

/// Report a load failure to both the log and the frontend's on-screen message.
fn fail(message: &str) -> bool {
    log::error!("{message}");
    callbacks::show_message(message, 240);
    false
}

/// Reads the package and boots the guest.
///
/// Returns the kept container and resource bytes together with the machine:
/// the container stays in the core so `retro_reset` can rebuild a machine
/// without a new content load.
fn load_machine(path: &Path) -> anyhow::Result<(Vec<u8>, Vec<Vec<u8>>, SnowFishMachine)> {
    let container = std::fs::read(path)
        .map_err(|error| anyhow::anyhow!("cannot read {}: {error}", path.display()))?;
    let res_data: Vec<Vec<u8>> = find_res_files(path)
        .into_iter()
        .filter_map(|res_path| match std::fs::read(&res_path) {
            Ok(data) => {
                log::info!("Serving sibling resource file {}", res_path.display());
                Some(data)
            }
            Err(error) => {
                log::warn!("Ignoring resource file {}: {error}", res_path.display());
                None
            }
        })
        .collect();
    let machine = rebuild_machine(&container, &res_data)?;
    Ok((container, res_data, machine))
}

/// Builds a freshly booted machine from the kept package bytes.
fn rebuild_machine(container: &[u8], res_data: &[Vec<u8>]) -> anyhow::Result<SnowFishMachine> {
    let mut machine = SnowFishMachine::load(container.to_vec(), res_data)
        .map_err(|error| anyhow::anyhow!("{error:#}"))?;
    machine
        .boot(INSTRUCTION_BUDGET)
        .map_err(|error| anyhow::anyhow!("boot failed: {error:#}"))?;
    Ok(machine)
}

/// Locates the sibling resource containers that back `res`/`.pak` file
/// opens.
///
/// A libretro frontend only provides the content path, so the lookup
/// follows the shared discovery rule in `snowfishemu_core::res`: an exact
/// `<content-stem>.pak`/`<content-stem>.res` sibling wins, else every
/// sibling whose name mentions `.pak` or `res` is served. A title may ship
/// more than one container (each carries its own pack), so every match is
/// served rather than only an unambiguous one.
fn find_res_files(content_path: &Path) -> Vec<PathBuf> {
    let Some(dir) = content_path.parent() else {
        return Vec::new();
    };
    let stem = content_path.file_stem().and_then(|s| s.to_str());
    let content_name = content_path.file_name().and_then(|s| s.to_str());
    let entries: Vec<PathBuf> = std::fs::read_dir(dir)
        .ok()
        .map(|entries| entries.flatten().map(|entry| entry.path()).collect())
        .unwrap_or_default();
    let names: Vec<String> = entries
        .iter()
        .filter_map(|path| path.file_name().and_then(|name| name.to_str()))
        .map(String::from)
        .collect();
    snowfishemu_core::res::sibling_res_candidates(stem, content_name.unwrap_or_default(), names)
        .into_iter()
        .map(|name| dir.join(name))
        .filter(|path| path.is_file())
        .collect()
}

#[no_mangle]
pub extern "C" fn retro_load_game_special(
    _type: u32,
    _info: *const retro_game_info,
    _num: usize,
) -> bool {
    false
}

#[no_mangle]
pub extern "C" fn retro_unload_game() {
    guard((), || {
        unsafe {
            EMULATOR = None;
        }
        log::info!("Game unloaded");
    })
}

#[no_mangle]
pub extern "C" fn retro_run() {
    guard((), || unsafe {
        let Some(emulator) = EMULATOR.as_mut() else {
            log::warn!("retro_run called before loading a game");
            return;
        };

        if options::core_options_changed() {
            apply_core_options();
        }

        callbacks::input_poll();
        update_phone_keys(emulator);

        if !emulator.stopped {
            for _ in 0..emulator.tick_pacer.advance() {
                if let Err(error) = emulator.machine.run_frame(INSTRUCTION_BUDGET) {
                    log::warn!("guest frame stopped: {error:#}");
                    emulator.stopped = true;
                    break;
                }
            }
        }

        // Present the last valid guest framebuffer even after a stop. The
        // surface is a fixed 240x320 RGB565 window converted to XRGB8888.
        let pixels: Vec<u32> = emulator.machine.frame_pixels();
        callbacks::video_refresh(
            pixels.as_ptr() as *const c_void,
            DISPLAY_WIDTH,
            DISPLAY_HEIGHT,
            (DISPLAY_WIDTH * 4) as usize,
        );

        // Keep the sample flow at exactly the reported pacing rate: the
        // frontend paces the core by audio sync, so a quiet guest must
        // still submit the full pacing batch or the frontend would fall
        // back to display-rate pacing and the game would run too fast.
        // The music engine's samples ride on top of that flow; a batch
        // shorter than the pacing count is silence-padded.
        let mut samples = emulator.machine.take_audio_samples(PACING_SAMPLES_PER_RUN);
        samples.resize(PACING_SAMPLES_PER_RUN * 2, 0);
        callbacks::audio_sample_batch(samples.as_ptr(), PACING_SAMPLES_PER_RUN);
    })
}

fn input_descriptors() -> [retro_input_descriptor; 14] {
    [
        retro_input_descriptor {
            port: 0,
            device: RETRO_DEVICE_JOYPAD,
            index: 0,
            id: RETRO_DEVICE_ID_JOYPAD_UP,
            description: c"D-Pad Up".as_ptr(),
        },
        retro_input_descriptor {
            port: 0,
            device: RETRO_DEVICE_JOYPAD,
            index: 0,
            id: RETRO_DEVICE_ID_JOYPAD_DOWN,
            description: c"D-Pad Down".as_ptr(),
        },
        retro_input_descriptor {
            port: 0,
            device: RETRO_DEVICE_JOYPAD,
            index: 0,
            id: RETRO_DEVICE_ID_JOYPAD_LEFT,
            description: c"D-Pad Left".as_ptr(),
        },
        retro_input_descriptor {
            port: 0,
            device: RETRO_DEVICE_JOYPAD,
            index: 0,
            id: RETRO_DEVICE_ID_JOYPAD_RIGHT,
            description: c"D-Pad Right".as_ptr(),
        },
        retro_input_descriptor {
            port: 0,
            device: RETRO_DEVICE_JOYPAD,
            index: 0,
            id: RETRO_DEVICE_ID_JOYPAD_A,
            description: c"Confirm".as_ptr(),
        },
        retro_input_descriptor {
            port: 0,
            device: RETRO_DEVICE_JOYPAD,
            index: 0,
            id: RETRO_DEVICE_ID_JOYPAD_B,
            description: c"Confirm".as_ptr(),
        },
        retro_input_descriptor {
            port: 0,
            device: RETRO_DEVICE_JOYPAD,
            index: 0,
            id: RETRO_DEVICE_ID_JOYPAD_START,
            description: c"Confirm".as_ptr(),
        },
        retro_input_descriptor {
            port: 0,
            device: RETRO_DEVICE_JOYPAD,
            index: 0,
            id: RETRO_DEVICE_ID_JOYPAD_X,
            description: c"Left Soft Key".as_ptr(),
        },
        retro_input_descriptor {
            port: 0,
            device: RETRO_DEVICE_JOYPAD,
            index: 0,
            id: RETRO_DEVICE_ID_JOYPAD_Y,
            description: c"Right Soft Key".as_ptr(),
        },
        retro_input_descriptor {
            port: 0,
            device: RETRO_DEVICE_JOYPAD,
            index: 0,
            id: RETRO_DEVICE_ID_JOYPAD_L,
            description: c"Left Soft Key".as_ptr(),
        },
        retro_input_descriptor {
            port: 0,
            device: RETRO_DEVICE_JOYPAD,
            index: 0,
            id: RETRO_DEVICE_ID_JOYPAD_R,
            description: c"Right Soft Key".as_ptr(),
        },
        retro_input_descriptor {
            port: 0,
            device: RETRO_DEVICE_JOYPAD,
            index: 0,
            id: RETRO_DEVICE_ID_JOYPAD_L2,
            description: c"Menu".as_ptr(),
        },
        retro_input_descriptor {
            port: 0,
            device: RETRO_DEVICE_JOYPAD,
            index: 0,
            id: RETRO_DEVICE_ID_JOYPAD_SELECT,
            description: c"Number Layer (hold)".as_ptr(),
        },
        retro_input_descriptor {
            port: 0,
            device: 0,
            index: 0,
            id: 0,
            description: ptr::null(),
        },
    ]
}

fn register_input_descriptors() {
    let descriptors = input_descriptors();
    callbacks::environment(
        RETRO_ENVIRONMENT_SET_INPUT_DESCRIPTORS,
        descriptors.as_ptr() as *mut c_void,
    );
}

fn register_memory_maps(emulator: &mut Emulator) {
    let spans = emulator.machine.mem.region_spans();
    let mut descriptors: Vec<retro_memory_descriptor> = Vec::with_capacity(spans.len() + 1);
    for (base, len) in spans {
        let Some(ptr) = emulator.machine.mem.region_pointer(base) else {
            continue;
        };
        descriptors.push(retro_memory_descriptor {
            flags: 0,
            ptr: ptr.cast(),
            offset: 0,
            start: base as usize,
            select: 0,
            disconnect: 0,
            len,
            addrspace: c"SnowFish".as_ptr(),
        });
    }
    descriptors.push(retro_memory_descriptor {
        flags: 0,
        ptr: ptr::null_mut(),
        offset: 0,
        start: 0,
        select: 0,
        disconnect: 0,
        len: 0,
        addrspace: ptr::null(),
    });

    let map = retro_memory_map {
        descriptors: descriptors.as_ptr(),
        num_descriptors: descriptors.len() as u32,
    };
    if callbacks::environment(
        RETRO_ENVIRONMENT_SET_MEMORY_MAPS,
        &map as *const _ as *mut c_void,
    ) {
        log::info!("Registered guest memory descriptors");
    } else {
        log::warn!("Frontend did not accept guest memory descriptors");
    }
}

/// Apply the frontend core option selections.
///
/// The only option affects logging, which is process-wide presentation
/// state; no emulator state changes with the selections.
fn apply_core_options() {
    let options = options::read_core_options(options::get_core_option);
    super::logger::set_debug_logging(options.debug_logging);
    log::info!(
        "Core options applied: debug_logging={}",
        options.debug_logging
    );
}

/// Analog stick deflection (of the 0x7fff full range) required before a
/// direction counts, matching the standalone gamepad mapper's 0.5 deadzone.
const ANALOG_DEADZONE: i16 = 0x4000;

/// Map RetroPad buttons and analog sticks onto guest key states.
///
/// Returns one entry per mapped guest key (in [`MAPPED_KEYS`] order) so
/// every key is written each poll and releases propagate. Pure so the shared
/// RetroPad layout can be unit-tested without a frontend; kept in lockstep
/// with the standalone `pump_keys` mapping.
fn retro_pad_key_states(
    is_pressed: impl Fn(u32) -> bool,
    analog: impl Fn(u32, u32) -> i16,
) -> Vec<(u8, bool)> {
    // libretro analog axes report +Y downwards; the guest D-pad does not.
    let stick_up = analog(RETRO_DEVICE_INDEX_ANALOG_LEFT, RETRO_DEVICE_ID_ANALOG_Y)
        < -ANALOG_DEADZONE
        || analog(RETRO_DEVICE_INDEX_ANALOG_RIGHT, RETRO_DEVICE_ID_ANALOG_Y) < -ANALOG_DEADZONE;
    let stick_down = analog(RETRO_DEVICE_INDEX_ANALOG_LEFT, RETRO_DEVICE_ID_ANALOG_Y)
        > ANALOG_DEADZONE
        || analog(RETRO_DEVICE_INDEX_ANALOG_RIGHT, RETRO_DEVICE_ID_ANALOG_Y) > ANALOG_DEADZONE;
    let stick_left = analog(RETRO_DEVICE_INDEX_ANALOG_LEFT, RETRO_DEVICE_ID_ANALOG_X)
        < -ANALOG_DEADZONE
        || analog(RETRO_DEVICE_INDEX_ANALOG_RIGHT, RETRO_DEVICE_ID_ANALOG_X) < -ANALOG_DEADZONE;
    let stick_right = analog(RETRO_DEVICE_INDEX_ANALOG_LEFT, RETRO_DEVICE_ID_ANALOG_X)
        > ANALOG_DEADZONE
        || analog(RETRO_DEVICE_INDEX_ANALOG_RIGHT, RETRO_DEVICE_ID_ANALOG_X) > ANALOG_DEADZONE;

    let up = is_pressed(RETRO_DEVICE_ID_JOYPAD_UP) || stick_up;
    let down = is_pressed(RETRO_DEVICE_ID_JOYPAD_DOWN) || stick_down;
    let left = is_pressed(RETRO_DEVICE_ID_JOYPAD_LEFT) || stick_left;
    let right = is_pressed(RETRO_DEVICE_ID_JOYPAD_RIGHT) || stick_right;

    // Number layer: holding Select turns the pad into a numeric keypad. The
    // layout is grouped for easy recall — directions are digits 1-4 (up,
    // down, left, right), face buttons are 5-8 (A, B, X, Y), Start is 9, and
    // either shoulder is 0. Directions accept the D-pad and the sticks
    // alike. Select itself sends nothing while the layer is engaged, and
    // the pad's normal keys are suppressed until it is released.
    if is_pressed(RETRO_DEVICE_ID_JOYPAD_SELECT) {
        let chords = [
            (1, up),
            (2, down),
            (3, left),
            (4, right),
            (5, is_pressed(RETRO_DEVICE_ID_JOYPAD_A)),
            (6, is_pressed(RETRO_DEVICE_ID_JOYPAD_B)),
            (7, is_pressed(RETRO_DEVICE_ID_JOYPAD_X)),
            (8, is_pressed(RETRO_DEVICE_ID_JOYPAD_Y)),
            (9, is_pressed(RETRO_DEVICE_ID_JOYPAD_START)),
            (
                0,
                is_pressed(RETRO_DEVICE_ID_JOYPAD_L) || is_pressed(RETRO_DEVICE_ID_JOYPAD_R),
            ),
        ];
        return chords
            .into_iter()
            .map(|(digit, pressed)| (digit_index(digit), pressed))
            .collect();
    }

    // Confirm on any of the three natural face/start buttons.
    let confirm = is_pressed(RETRO_DEVICE_ID_JOYPAD_A)
        || is_pressed(RETRO_DEVICE_ID_JOYPAD_B)
        || is_pressed(RETRO_DEVICE_ID_JOYPAD_START);
    // X plus L duplicate the left soft key; Y plus R duplicate the right.
    let soft_left = is_pressed(RETRO_DEVICE_ID_JOYPAD_X) || is_pressed(RETRO_DEVICE_ID_JOYPAD_L);
    let soft_right = is_pressed(RETRO_DEVICE_ID_JOYPAD_Y) || is_pressed(RETRO_DEVICE_ID_JOYPAD_R);

    MAPPED_KEYS
        .iter()
        .map(|&index| {
            let pressed = match index {
                keys::UP => up,
                keys::DOWN => down,
                keys::LEFT => left,
                keys::RIGHT => right,
                keys::OK => confirm,
                keys::SOFT_L => soft_left,
                keys::SOFT_R => soft_right,
                keys::MENU => is_pressed(RETRO_DEVICE_ID_JOYPAD_L2),
                _ => false,
            };
            (index, pressed)
        })
        .collect()
}

/// Map polled keyboard keys onto guest key states.
///
/// The layout mirrors the standalone frontend's key map one-to-one so the
/// same physical keys drive the same guest keys in both frontends: digits
/// 1-9/0, the arrow keys, Enter, Z/X for the soft keys and M for the menu.
/// Direction keys have their own indices. OK shares its index with digit 6.
fn keyboard_key_states(is_pressed: impl Fn(u32) -> bool) -> Vec<(u8, bool)> {
    let digit = |d: u32| is_pressed(RETROK_0 + d);
    MAPPED_KEYS
        .iter()
        .map(|&index| {
            let pressed = match index {
                0 => digit(1),
                1 => digit(2),
                2 => digit(3),
                3 => digit(4),
                4 => digit(5),
                keys::OK => is_pressed(RETROK_RETURN) || digit(6),
                6 => digit(7),
                7 => digit(8),
                8 => digit(9),
                9 => digit(0),
                keys::SOFT_L => is_pressed(RETROK_z),
                keys::SOFT_R => is_pressed(RETROK_x),
                keys::MENU => is_pressed(RETROK_m),
                keys::UP => is_pressed(RETROK_UP),
                keys::DOWN => is_pressed(RETROK_DOWN),
                keys::LEFT => is_pressed(RETROK_LEFT),
                keys::RIGHT => is_pressed(RETROK_RIGHT),
                _ => false,
            };
            (index, pressed)
        })
        .collect()
}

/// Map RetroPad buttons and the keyboard to guest key indices.
///
/// Keyboard and pad combine as a logical OR per key, matching the standalone
/// frontend's keyboard-plus-gamepad behavior. Only edges reach the core: the
/// machine queues an event per press/release change, so repeating an
/// unchanged state every poll would flood the event queue.
fn update_phone_keys(emulator: &mut Emulator) {
    let joypad = |id: u32| callbacks::input_state(0, RETRO_DEVICE_JOYPAD, 0, id) != 0;
    let analog = |index: u32, id: u32| callbacks::input_state(0, RETRO_DEVICE_ANALOG, index, id);
    let pad = retro_pad_key_states(joypad, analog);
    let keyboard =
        keyboard_key_states(|key| callbacks::input_state(0, RETRO_DEVICE_KEYBOARD, 0, key) != 0);

    // OR the pad and keyboard states per guest key, remembering which keys
    // this poll can report on (MAPPED_KEYS order, seen flag).
    let mut pressed_now = [false; 32];
    let mut seen = [false; 32];
    for (index, pressed) in pad.into_iter().chain(keyboard) {
        pressed_now[index as usize] |= pressed;
        seen[index as usize] = true;
    }
    for index in 0..32usize {
        if !seen[index] || pressed_now[index] == emulator.key_state[index] {
            continue;
        }
        let key = index as u8;
        if let Err(error) = emulator.machine.set_phone_key(key, pressed_now[index]) {
            log::warn!("key {key} rejected: {error}");
        }
        emulator.key_state[index] = pressed_now[index];
    }
}

#[no_mangle]
pub extern "C" fn retro_reset() {
    guard((), || unsafe {
        let Some(emulator) = EMULATOR.as_mut() else {
            log::warn!("retro_reset called before loading a game");
            return;
        };
        match rebuild_machine(&emulator.container, &emulator.res_data) {
            Ok(fresh) => {
                // Restore the fresh boot state into the current machine's
                // allocations (same package, same region layout): guest
                // memory pointers registered with the frontend stay valid
                // across the reset.
                let blob = fresh.serialize();
                if let Err(error) = emulator.machine.deserialize(&blob) {
                    log::error!("Failed to reset game: {error}");
                    return;
                }
                emulator.stopped = false;
                emulator.key_state = [false; 32];
                emulator.tick_pacer = GuestTickPacer::new();
                apply_core_options();
                log::info!("Game reset");
            }
            Err(error) => log::error!("Failed to reset game: {error:#}"),
        }
    })
}

#[no_mangle]
pub extern "C" fn retro_get_region() -> u32 {
    RETRO_REGION_NTSC
}

// ============================================================
// Save states
// ============================================================
//
// The core produces a versioned, content-checked blob (see the core's save
// module); the fixed-size buffer below is the frontend's allocation target.

#[no_mangle]
pub extern "C" fn retro_serialize_size() -> usize {
    guard(0, || unsafe {
        if EMULATOR.is_none() {
            0
        } else {
            SAVE_STATE_MAX_SIZE
        }
    })
}

#[no_mangle]
pub extern "C" fn retro_serialize(data: *mut c_void, size: usize) -> bool {
    guard(false, || unsafe {
        let Some(emulator) = EMULATOR.as_ref() else {
            return false;
        };
        if data.is_null() {
            return false;
        }
        let blob = emulator.machine.serialize();
        if blob.len() > size {
            log::error!(
                "Save state needs {} bytes but the frontend buffer holds {size}",
                blob.len()
            );
            return false;
        }
        let buffer = std::slice::from_raw_parts_mut(data as *mut u8, size);
        buffer[..blob.len()].copy_from_slice(&blob);
        buffer[blob.len()..].fill(0);
        true
    })
}

#[no_mangle]
pub extern "C" fn retro_unserialize(data: *const c_void, size: usize) -> bool {
    guard(false, || unsafe {
        let Some(emulator) = EMULATOR.as_mut() else {
            return false;
        };
        if data.is_null() || size == 0 {
            return false;
        }
        let buffer = std::slice::from_raw_parts(data as *const u8, size);
        match emulator.machine.deserialize(buffer) {
            Ok(()) => {
                emulator.stopped = false;
                // apply_core_options re-applies the logging selection, which
                // the save-state codec does not carry.
                apply_core_options();
                log::info!("Game state restored");
                true
            }
            Err(error) => {
                log::error!("Failed to restore game state: {error}");
                false
            }
        }
    })
}

// ============================================================
// Optional API stubs required for frontend symbol resolution
// ============================================================

#[no_mangle]
pub extern "C" fn retro_cheat_reset() {
    // Cheats are not supported.
}

#[no_mangle]
pub extern "C" fn retro_cheat_set(_index: u32, _enabled: bool, _code: *const c_char) {
    // Cheats are not supported.
}

#[no_mangle]
pub extern "C" fn retro_get_memory_data(_id: u32) -> *mut c_void {
    // This platform has no SRAM/VRAM abstraction; guest memory is exposed
    // through the registered memory maps instead.
    ptr::null_mut()
}

#[no_mangle]
pub extern "C" fn retro_get_memory_size(_id: u32) -> usize {
    0
}

#[cfg(test)]
mod tests {
    use super::*;
    use snowfishemu_core::SAVE_STATE_FORMAT;
    use std::ffi::CString;
    use std::sync::Mutex;

    /// Captured video frame: width, height, pitch, and raw XRGB8888 bytes.
    type VideoFrame = (u32, u32, usize, Vec<u8>);

    static LAST_VIDEO: Mutex<Option<VideoFrame>> = Mutex::new(None);
    static AUDIO_FRAMES: Mutex<usize> = Mutex::new(0);
    /// Serializes the real-content tests: the libretro core is a process-wide
    /// singleton, so concurrent content loads would clobber each other.
    static CONTENT_LOCK: Mutex<()> = Mutex::new(());

    unsafe extern "C" fn test_environment(cmd: u32, _data: *mut c_void) -> bool {
        matches!(
            cmd,
            RETRO_ENVIRONMENT_SET_PIXEL_FORMAT
                | RETRO_ENVIRONMENT_SET_INPUT_DESCRIPTORS
                | RETRO_ENVIRONMENT_SET_PERFORMANCE_LEVEL
                | RETRO_ENVIRONMENT_SET_MEMORY_MAPS
                | RETRO_ENVIRONMENT_SET_VARIABLES
        )
    }

    unsafe extern "C" fn test_video_refresh(
        data: *const c_void,
        width: u32,
        height: u32,
        pitch: usize,
    ) {
        let bytes = if data.is_null() {
            Vec::new()
        } else {
            let len = pitch
                .saturating_mul(height as usize)
                .min(DISPLAY_WIDTH as usize * DISPLAY_HEIGHT as usize * 4);
            std::slice::from_raw_parts(data as *const u8, len).to_vec()
        };
        *LAST_VIDEO.lock().unwrap() = Some((width, height, pitch, bytes));
    }

    unsafe extern "C" fn test_input_poll() {}

    unsafe extern "C" fn test_input_state(_port: u32, _device: u32, _index: u32, _id: u32) -> i16 {
        0
    }

    unsafe extern "C" fn test_audio_batch(data: *const i16, frames: usize) -> usize {
        // The guest produces no audio; the core must still submit the
        // constant silence flow for pacing.
        assert!(!data.is_null());
        *AUDIO_FRAMES.lock().unwrap() += frames;
        frames
    }

    #[test]
    fn system_info_reports_metadata_and_app_extensions() {
        let mut info = std::mem::MaybeUninit::<retro_system_info>::zeroed();
        retro_get_system_info(info.as_mut_ptr());
        let info = unsafe { info.assume_init() };
        assert_eq!(
            unsafe { CStr::from_ptr(info.library_name) }
                .to_str()
                .unwrap(),
            "SnowFishEmu"
        );
        assert_eq!(
            unsafe { CStr::from_ptr(info.valid_extensions) }
                .to_str()
                .unwrap(),
            "app"
        );
        assert_eq!(
            unsafe { CStr::from_ptr(info.library_version) }
                .to_str()
                .unwrap(),
            env!("CARGO_PKG_VERSION")
        );
        assert!(info.need_fullpath);
        assert!(!info.block_extract);
    }

    #[test]
    fn av_info_defaults_to_the_portrait_display_without_content() {
        let mut av = std::mem::MaybeUninit::<retro_system_av_info>::zeroed();
        retro_get_system_av_info(av.as_mut_ptr());
        let av = unsafe { av.assume_init() };
        assert_eq!(av.geometry.base_width, DISPLAY_WIDTH);
        assert_eq!(av.geometry.base_height, DISPLAY_HEIGHT);
        assert_eq!(av.geometry.max_width, MAX_DISPLAY_WIDTH);
        assert_eq!(av.geometry.max_height, MAX_DISPLAY_HEIGHT);
        assert!(
            (av.geometry.aspect_ratio - DISPLAY_WIDTH as f32 / DISPLAY_HEIGHT as f32).abs() < 1e-6
        );
        assert_eq!(av.timing.fps, PACING_FPS as f64);
        assert_eq!(av.timing.sample_rate, AUDIO_SAMPLE_RATE as f64);
    }

    /// The guest must advance its 20 Hz screen tick on every third frontend
    /// frame while the core reports a 60 Hz pacing rate the frontend can
    /// actually pace.
    #[test]
    fn pacer_advances_guest_ticks_at_the_guest_tick_rate() {
        let mut pacer = GuestTickPacer::new();
        let pattern: Vec<u32> = (0..9).map(|_| pacer.advance()).collect();
        assert_eq!(pattern, [0, 0, 1, 0, 0, 1, 0, 0, 1]);

        let mut pacer = GuestTickPacer::new();
        let ticks: u32 = (0..60).map(|_| pacer.advance()).sum();
        assert_eq!(ticks, 20);
    }

    #[test]
    fn input_descriptors_cover_retropad_mapping() {
        let descriptors = input_descriptors();
        let ids: Vec<u32> = descriptors[..12]
            .iter()
            .map(|descriptor| descriptor.id)
            .collect();
        assert_eq!(
            ids,
            [
                RETRO_DEVICE_ID_JOYPAD_UP,
                RETRO_DEVICE_ID_JOYPAD_DOWN,
                RETRO_DEVICE_ID_JOYPAD_LEFT,
                RETRO_DEVICE_ID_JOYPAD_RIGHT,
                RETRO_DEVICE_ID_JOYPAD_A,
                RETRO_DEVICE_ID_JOYPAD_B,
                RETRO_DEVICE_ID_JOYPAD_START,
                RETRO_DEVICE_ID_JOYPAD_X,
                RETRO_DEVICE_ID_JOYPAD_Y,
                RETRO_DEVICE_ID_JOYPAD_L,
                RETRO_DEVICE_ID_JOYPAD_R,
                RETRO_DEVICE_ID_JOYPAD_L2,
            ]
        );
        assert_eq!(descriptors[12].id, RETRO_DEVICE_ID_JOYPAD_SELECT);
        assert!(!descriptors[12].description.is_null());
        assert!(descriptors[13].description.is_null());
    }

    /// The shared RetroPad layout must reach every non-digit guest key:
    /// soft keys, confirm, menu, and the D-pad.
    #[test]
    fn retro_pad_mapping_reaches_every_non_digit_guest_key() {
        let zero = |_index: u32, _id: u32| 0;
        let mut reached = std::collections::HashSet::new();
        for button in [
            RETRO_DEVICE_ID_JOYPAD_UP,
            RETRO_DEVICE_ID_JOYPAD_DOWN,
            RETRO_DEVICE_ID_JOYPAD_LEFT,
            RETRO_DEVICE_ID_JOYPAD_RIGHT,
            RETRO_DEVICE_ID_JOYPAD_A,
            RETRO_DEVICE_ID_JOYPAD_B,
            RETRO_DEVICE_ID_JOYPAD_X,
            RETRO_DEVICE_ID_JOYPAD_Y,
            RETRO_DEVICE_ID_JOYPAD_START,
            RETRO_DEVICE_ID_JOYPAD_L,
            RETRO_DEVICE_ID_JOYPAD_R,
            RETRO_DEVICE_ID_JOYPAD_L2,
        ] {
            let states = retro_pad_key_states(|id| id == button, zero);
            for (key, pressed) in states {
                if pressed {
                    reached.insert(key);
                }
            }
        }
        assert_eq!(
            reached,
            [
                keys::UP,
                keys::DOWN,
                keys::LEFT,
                keys::RIGHT,
                keys::OK,
                keys::SOFT_L,
                keys::SOFT_R,
                keys::MENU,
            ]
            .into_iter()
            .collect()
        );
    }

    #[test]
    fn retro_pad_mapping_matches_the_shared_layout() {
        // A face button, B, and Start all confirm (guest 5).
        for button in [
            RETRO_DEVICE_ID_JOYPAD_A,
            RETRO_DEVICE_ID_JOYPAD_B,
            RETRO_DEVICE_ID_JOYPAD_START,
        ] {
            let states = retro_pad_key_states(|id| id == button, |_index, _id| 0);
            assert_eq!(
                states
                    .into_iter()
                    .filter(|(_, pressed)| *pressed)
                    .collect::<Vec<_>>(),
                [(keys::OK, true)]
            );
        }

        // X and L are the left soft key, Y and R the right.
        for (button, guest) in [
            (RETRO_DEVICE_ID_JOYPAD_X, keys::SOFT_L),
            (RETRO_DEVICE_ID_JOYPAD_L, keys::SOFT_L),
            (RETRO_DEVICE_ID_JOYPAD_Y, keys::SOFT_R),
            (RETRO_DEVICE_ID_JOYPAD_R, keys::SOFT_R),
            (RETRO_DEVICE_ID_JOYPAD_L2, keys::MENU),
        ] {
            let states = retro_pad_key_states(|id| id == button, |_index, _id| 0);
            assert_eq!(
                states
                    .into_iter()
                    .filter(|(_, pressed)| *pressed)
                    .collect::<Vec<_>>(),
                [(guest, true)]
            );
        }

        // D-pad buttons map onto the phone's arrow indices.
        for (button, guest) in [
            (RETRO_DEVICE_ID_JOYPAD_UP, keys::UP),
            (RETRO_DEVICE_ID_JOYPAD_DOWN, keys::DOWN),
            (RETRO_DEVICE_ID_JOYPAD_LEFT, keys::LEFT),
            (RETRO_DEVICE_ID_JOYPAD_RIGHT, keys::RIGHT),
        ] {
            let states = retro_pad_key_states(|id| id == button, |_index, _id| 0);
            assert_eq!(
                states
                    .into_iter()
                    .filter(|(_, pressed)| *pressed)
                    .collect::<Vec<_>>(),
                [(guest, true)]
            );
        }
    }

    #[test]
    fn retro_pad_analog_sticks_drive_directions() {
        let none = |_id: u32| false;
        // libretro +Y is downwards: full up deflection reaches only the up key.
        let states = retro_pad_key_states(none, |index, id| {
            if index == RETRO_DEVICE_INDEX_ANALOG_LEFT && id == RETRO_DEVICE_ID_ANALOG_Y {
                -i16::MAX
            } else {
                0
            }
        });
        assert_eq!(
            states
                .into_iter()
                .filter(|(_, pressed)| *pressed)
                .collect::<Vec<_>>(),
            [(keys::UP, true)]
        );

        // Full +X deflection reaches only the right key.
        let states = retro_pad_key_states(none, |index, id| {
            if index == RETRO_DEVICE_INDEX_ANALOG_RIGHT && id == RETRO_DEVICE_ID_ANALOG_X {
                i16::MAX
            } else {
                0
            }
        });
        assert_eq!(
            states
                .into_iter()
                .filter(|(_, pressed)| *pressed)
                .collect::<Vec<_>>(),
            [(keys::RIGHT, true)]
        );

        // Values inside the deadzone report nothing.
        let states = retro_pad_key_states(none, |_index, _id| ANALOG_DEADZONE - 1);
        assert!(states.into_iter().all(|(_, pressed)| !pressed));
    }

    /// The keyboard layout must mirror the standalone key map and reach
    /// every mapped guest key, digits included.
    #[test]
    fn keyboard_mapping_reaches_every_mapped_guest_key() {
        let none = |_key: u32| false;
        let idle = keyboard_key_states(none);
        assert!(idle.into_iter().all(|(_, pressed)| !pressed));

        let mut reached = std::collections::HashSet::new();
        for keysym in (RETROK_0..=RETROK_9).chain([
            RETROK_RETURN,
            RETROK_z,
            RETROK_x,
            RETROK_m,
            RETROK_UP,
            RETROK_DOWN,
            RETROK_LEFT,
            RETROK_RIGHT,
        ]) {
            for (key, pressed) in keyboard_key_states(|key| key == keysym) {
                if pressed {
                    reached.insert(key);
                }
            }
        }
        assert_eq!(reached, MAPPED_KEYS.into_iter().collect());
    }

    #[test]
    fn keyboard_mapping_matches_the_standalone_layout() {
        // Digits 1-9 map to guest indices 0-8 and 0 to 9.
        for keysym_digit in 1..=9u32 {
            let states = keyboard_key_states(|key| key == RETROK_0 + keysym_digit);
            assert_eq!(
                states
                    .into_iter()
                    .filter(|(_, pressed)| *pressed)
                    .collect::<Vec<_>>(),
                [((keysym_digit - 1) as u8, true)]
            );
        }
        let states = keyboard_key_states(|key| key == RETROK_0);
        assert_eq!(
            states
                .into_iter()
                .filter(|(_, pressed)| *pressed)
                .collect::<Vec<_>>(),
            [(9u8, true)]
        );

        // Enter confirms; Z/X are the soft keys; arrows drive the D-pad.
        for (keysym, guest) in [
            (RETROK_RETURN, keys::OK),
            (RETROK_z, keys::SOFT_L),
            (RETROK_x, keys::SOFT_R),
            (RETROK_m, keys::MENU),
            (RETROK_UP, keys::UP),
            (RETROK_DOWN, keys::DOWN),
            (RETROK_LEFT, keys::LEFT),
            (RETROK_RIGHT, keys::RIGHT),
        ] {
            let states = keyboard_key_states(|key| key == keysym);
            assert_eq!(
                states
                    .into_iter()
                    .filter(|(_, pressed)| *pressed)
                    .collect::<Vec<_>>(),
                [(guest, true)]
            );
        }
    }

    /// Keyboard and pad must combine as a logical OR per guest key, the same
    /// merge the standalone frontend applies.
    #[test]
    fn keyboard_and_pad_combine_per_key() {
        // Digit 3 and the D-pad Up have independent states.
        let keyboard = keyboard_key_states(|key| key == RETROK_0 + 3);
        let pad = retro_pad_key_states(|id| id == RETRO_DEVICE_ID_JOYPAD_UP, |_, _| 0);
        let mut merged = std::collections::HashMap::new();
        for (key, pressed) in keyboard.into_iter().chain(pad) {
            *merged.entry(key).or_insert(false) |= pressed;
        }
        assert!(merged[&keys::UP]);
        assert!(merged[&digit_index(3)]);

        // Digit 6 and Enter both drive the confirm key.
        let keyboard = keyboard_key_states(|key| key == RETROK_RETURN);
        let pad = retro_pad_key_states(
            |id| id == RETRO_DEVICE_ID_JOYPAD_SELECT || id == RETRO_DEVICE_ID_JOYPAD_B,
            |_, _| 0,
        );
        let mut merged = std::collections::HashMap::new();
        for (key, pressed) in keyboard.into_iter().chain(pad) {
            *merged.entry(key).or_insert(false) |= pressed;
        }
        assert!(merged[&keys::OK]);
        assert!(!merged[&keys::SOFT_L]);
    }

    /// Holding Select turns the pad into a grouped digit layer — directions
    /// 1-4, face buttons 5-8, Start 9, shoulders 0 — so digits are reachable
    /// from a pad in both frontends.
    #[test]
    fn number_layer_maps_digits_in_grouped_order() {
        let layer = |id: u32| id == RETRO_DEVICE_ID_JOYPAD_SELECT;
        let chords = [
            (RETRO_DEVICE_ID_JOYPAD_UP, 1u32),
            (RETRO_DEVICE_ID_JOYPAD_DOWN, 2),
            (RETRO_DEVICE_ID_JOYPAD_LEFT, 3),
            (RETRO_DEVICE_ID_JOYPAD_RIGHT, 4),
            (RETRO_DEVICE_ID_JOYPAD_A, 5),
            (RETRO_DEVICE_ID_JOYPAD_B, 6),
            (RETRO_DEVICE_ID_JOYPAD_X, 7),
            (RETRO_DEVICE_ID_JOYPAD_Y, 8),
            (RETRO_DEVICE_ID_JOYPAD_START, 9),
            (RETRO_DEVICE_ID_JOYPAD_L, 0),
            (RETRO_DEVICE_ID_JOYPAD_R, 0),
        ];
        for (button, digit) in chords {
            let states = retro_pad_key_states(|id| layer(id) || id == button, |_, _| 0);
            let pressed: Vec<(u8, bool)> = states.into_iter().filter(|(_, down)| *down).collect();
            assert_eq!(
                pressed,
                [(digit_index(digit), true)],
                "button {button} -> digit {digit}"
            );
        }
    }

    #[test]
    fn number_layer_suppresses_the_normal_pad_keys() {
        // Select + X (the soft-key chord) must emit digit 7 only, never the
        // left soft key.
        let states = retro_pad_key_states(
            |id| id == RETRO_DEVICE_ID_JOYPAD_SELECT || id == RETRO_DEVICE_ID_JOYPAD_X,
            |_, _| 0,
        );
        let pressed: Vec<(u8, bool)> = states.into_iter().filter(|(_, down)| *down).collect();
        assert_eq!(pressed, [(digit_index(7), true)]);

        // Select alone sends nothing (it is a pure modifier).
        let states = retro_pad_key_states(|id| id == RETRO_DEVICE_ID_JOYPAD_SELECT, |_, _| 0);
        assert!(states.into_iter().all(|(_, down)| !down));
    }

    #[test]
    fn number_layer_direction_chords_accept_the_sticks() {
        // Stick up past the deadzone while Select is held produces digit 1.
        let states = retro_pad_key_states(
            |id| id == RETRO_DEVICE_ID_JOYPAD_SELECT,
            |index, id| {
                if index == RETRO_DEVICE_INDEX_ANALOG_LEFT && id == RETRO_DEVICE_ID_ANALOG_Y {
                    -i16::MAX
                } else {
                    0
                }
            },
        );
        let pressed: Vec<(u8, bool)> = states.into_iter().filter(|(_, down)| *down).collect();
        assert_eq!(pressed, [(digit_index(1), true)]);
    }

    #[test]
    fn load_game_rejects_null_and_missing_paths() {
        retro_set_environment(test_environment);
        assert!(!retro_load_game(ptr::null()));

        let missing = CString::new("does-not-exist.app").unwrap();
        let info = retro_game_info {
            path: missing.as_ptr(),
            data: ptr::null(),
            size: 0,
            meta: ptr::null(),
        };
        assert!(!retro_load_game(&info));
        unsafe {
            assert!(EMULATOR.is_none());
        }
    }

    #[test]
    fn lifecycle_without_content_is_safe() {
        retro_set_environment(test_environment);
        retro_set_video_refresh(test_video_refresh);
        retro_set_input_poll(test_input_poll);
        retro_set_input_state(test_input_state);
        retro_init();
        retro_run();
        retro_reset();
        assert_eq!(retro_serialize_size(), 0);
        assert!(!retro_serialize(ptr::null_mut(), 0));
        assert!(!retro_unserialize(ptr::null(), 0));
        retro_deinit();
    }

    /// The save-state buffer must always hold the largest possible snapshot:
    /// the guest memory map is bounded by the loader's fixed region sizes.
    #[test]
    fn save_state_max_size_bounds_the_snapshot() {
        // Sum of the loader's worst-case region sizes (2 MiB code page,
        // 1 MiB stack, 1 MiB app data, 256 KiB platform table, 1 MiB heap),
        // with generous room for the codec envelope and bookkeeping.
        let region_max: usize = (2 + 1 + 1 + 1) * 1024 * 1024 + 256 * 1024;
        assert!(
            SAVE_STATE_MAX_SIZE >= region_max + 1024 * 1024,
            "SAVE_STATE_MAX_SIZE must exceed the largest possible guest memory map"
        );
    }

    #[test]
    fn android_profile_matches_the_core_identity() {
        let profile = include_str!("../../android.json");

        // The cdylib is named after the [lib] target, and the profile must
        // install exactly that file so the frontend can dlopen it.
        assert!(profile.contains("\"so_name\": \"libsnowfishemu.so\""));
        // Keep the profile in step with retro_get_system_info() and the
        // state format; both are checked above and in the save-state tests.
        assert!(profile.contains("\"supported_extensions\": [\"app\"]"));
        assert!(profile.contains(&format!("\"save_state_format\": {SAVE_STATE_FORMAT}")));
    }

    #[test]
    fn metadata_manifests_match_implemented_features() {
        let core_info = include_str!("../../snowfishemu_libretro.info");
        let libretro_manifest = include_str!("../../Cargo.toml");
        let workspace_manifest = include_str!("../../../../Cargo.toml");

        assert!(libretro_manifest.contains("name = \"snowfishemu-libretro\""));
        assert!(libretro_manifest.contains("name = \"snowfishemu\""));
        assert!(libretro_manifest.contains("\"cdylib\", \"staticlib\""));
        assert!(workspace_manifest.contains("\"crates/snowfishemu-libretro\""));
        assert!(core_info.contains("corename = \"snowfishemu\""));
        assert!(core_info.contains("savestate = \"true\""));
        assert!(core_info.contains("cheats = \"false\""));
        assert!(core_info.contains("input_descriptors = \"true\""));
        assert!(core_info.contains("memory_descriptors = \"true\""));
        assert!(core_info.contains("core_options = \"true\""));
        // The .info display version tracks the workspace version reported
        // through retro_get_system_info().
        assert!(core_info.contains(&format!(
            "display_version = \"{}\"",
            env!("CARGO_PKG_VERSION")
        )));
    }

    #[test]
    fn res_file_lookup_prefers_the_stem_named_siblings() {
        let root =
            std::env::temp_dir().join(format!("snowfishemu-libretro-res-{}", std::process::id()));
        let content = root.join("game.app");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(&content, b"").unwrap();

        // No sibling files: no resource.
        assert!(find_res_files(&content).is_empty());

        // A single "res"-named sibling matches.
        std::fs::write(root.join("res.bin"), b"x").unwrap();
        assert_eq!(find_res_files(&content), vec![root.join("res.bin")]);

        // The stem-named candidate wins over the loose match.
        std::fs::write(root.join("game.res"), b"y").unwrap();
        assert_eq!(find_res_files(&content), vec![root.join("game.res")]);

        // Without a stem match every candidate is served: a title may
        // ship several resource containers.
        std::fs::remove_file(root.join("game.res")).unwrap();
        std::fs::write(root.join("other.pak"), b"z").unwrap();
        let found = find_res_files(&content);
        assert!(found.contains(&root.join("res.bin")));
        assert!(found.contains(&root.join("other.pak")));

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    #[ignore = "requires local .app assets (set SF_GAME_DIR)"]
    fn real_content_boots_and_renders_frames() {
        let _content_lock = CONTENT_LOCK.lock().unwrap();
        let game_dir = std::env::var_os("SF_GAME_DIR").expect("SF_GAME_DIR is not set");
        let game_path = std::path::PathBuf::from(game_dir)
            .join("黄金矿工")
            .join("hjkg_QVGA_SNOWFISHV2.app");
        assert!(game_path.is_file(), "missing {}", game_path.display());
        let game_path = CString::new(game_path.to_string_lossy().as_bytes()).unwrap();

        retro_set_environment(test_environment);
        retro_set_video_refresh(test_video_refresh);
        retro_set_audio_sample_batch(test_audio_batch);
        retro_set_input_poll(test_input_poll);
        retro_set_input_state(test_input_state);
        retro_init();

        let info = retro_game_info {
            path: game_path.as_ptr(),
            data: ptr::null(),
            size: 0,
            meta: ptr::null(),
        };
        assert!(
            retro_load_game(&info),
            "failed to load {}",
            game_path.to_string_lossy()
        );

        for _ in 0..120 {
            retro_run();
        }

        // Debug aid: dump the last presented frame for offline inspection.
        if let Ok(dir) = std::env::var("SF_FRAME_DUMP") {
            let last = LAST_VIDEO.lock().unwrap();
            if let Some((_, _, _, bytes)) = last.as_ref() {
                let _ = std::fs::write(std::path::Path::new(&dir).join("core_last.raw"), &**bytes);
            }
        }

        for _ in 0..30 {
            retro_run();
        }

        {
            let last = LAST_VIDEO.lock().unwrap();
            let (width, height, _, bytes) = last.as_ref().expect("video refresh was never called");
            assert_eq!(*width, DISPLAY_WIDTH);
            assert_eq!(*height, DISPLAY_HEIGHT);
            assert_eq!(bytes.len(), (DISPLAY_WIDTH * DISPLAY_HEIGHT * 4) as usize);
        }

        retro_reset();
        for _ in 0..5 {
            retro_run();
        }
        assert!(LAST_VIDEO.lock().unwrap().is_some());

        let state_size = retro_serialize_size();
        assert_eq!(state_size, SAVE_STATE_MAX_SIZE);
        let mut state = vec![0u8; state_size];
        assert!(retro_serialize(state.as_mut_ptr().cast(), state.len()));
        assert!(retro_unserialize(state.as_ptr().cast(), state.len()));
        for _ in 0..5 {
            retro_run();
        }
        assert!(LAST_VIDEO.lock().unwrap().is_some());

        retro_unload_game();
        retro_deinit();
    }
}
