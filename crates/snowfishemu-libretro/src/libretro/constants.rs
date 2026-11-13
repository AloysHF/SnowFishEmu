// libretro constants used by the SnowFishEmu core.
//
// Keyboard keysym names mirror libretro.h (RETROK_a, RETROK_q, ...) so they
// stay greppable against the C header, hence the module-level lint allow.
#![allow(non_upper_case_globals)]

/// libretro API version implemented by this core.
pub const RETRO_API_VERSION: u32 = 1;

/// Device types.
pub const RETRO_DEVICE_NONE: u32 = 0;
pub const RETRO_DEVICE_JOYPAD: u32 = 1;
pub const RETRO_DEVICE_KEYBOARD: u32 = 3;
pub const RETRO_DEVICE_ANALOG: u32 = 5;

/// RetroPad button identifiers.
pub const RETRO_DEVICE_ID_JOYPAD_B: u32 = 0;
pub const RETRO_DEVICE_ID_JOYPAD_Y: u32 = 1;
pub const RETRO_DEVICE_ID_JOYPAD_SELECT: u32 = 2;
pub const RETRO_DEVICE_ID_JOYPAD_START: u32 = 3;
pub const RETRO_DEVICE_ID_JOYPAD_UP: u32 = 4;
pub const RETRO_DEVICE_ID_JOYPAD_DOWN: u32 = 5;
pub const RETRO_DEVICE_ID_JOYPAD_LEFT: u32 = 6;
pub const RETRO_DEVICE_ID_JOYPAD_RIGHT: u32 = 7;
pub const RETRO_DEVICE_ID_JOYPAD_A: u32 = 8;
pub const RETRO_DEVICE_ID_JOYPAD_X: u32 = 9;
pub const RETRO_DEVICE_ID_JOYPAD_L: u32 = 10;
pub const RETRO_DEVICE_ID_JOYPAD_R: u32 = 11;

/// Keyboard keysyms used by RETRO_DEVICE_KEYBOARD (SDL 1.2-compatible).
pub const RETROK_RETURN: u32 = 13;
pub const RETROK_0: u32 = 48;
pub const RETROK_1: u32 = 49;
pub const RETROK_2: u32 = 50;
pub const RETROK_3: u32 = 51;
pub const RETROK_4: u32 = 52;
pub const RETROK_5: u32 = 53;
pub const RETROK_6: u32 = 54;
pub const RETROK_7: u32 = 55;
pub const RETROK_8: u32 = 56;
pub const RETROK_9: u32 = 57;
pub const RETROK_x: u32 = 120;
pub const RETROK_z: u32 = 122;
pub const RETROK_UP: u32 = 273;
pub const RETROK_DOWN: u32 = 274;
pub const RETROK_RIGHT: u32 = 275;
pub const RETROK_LEFT: u32 = 276;

/// Analog stick identifiers (index selects the stick, id selects the axis).
pub const RETRO_DEVICE_INDEX_ANALOG_LEFT: u32 = 0;
pub const RETRO_DEVICE_INDEX_ANALOG_RIGHT: u32 = 1;
pub const RETRO_DEVICE_ID_ANALOG_X: u32 = 0;
pub const RETRO_DEVICE_ID_ANALOG_Y: u32 = 1;

/// Environment callback commands used by this core.
/// Show a transient on-screen message.
pub const RETRO_ENVIRONMENT_SET_MESSAGE: u32 = 6;
pub const RETRO_ENVIRONMENT_SET_PERFORMANCE_LEVEL: u32 = 8;
pub const RETRO_ENVIRONMENT_SET_PIXEL_FORMAT: u32 = 10;
pub const RETRO_ENVIRONMENT_SET_INPUT_DESCRIPTORS: u32 = 11;
pub const RETRO_ENVIRONMENT_GET_VARIABLE: u32 = 15;
pub const RETRO_ENVIRONMENT_SET_VARIABLES: u32 = 16;
pub const RETRO_ENVIRONMENT_GET_VARIABLE_UPDATE: u32 = 17;
pub const RETRO_ENVIRONMENT_GET_LOG_INTERFACE: u32 = 27;
pub const RETRO_ENVIRONMENT_SET_MEMORY_MAPS: u32 = 36 | 0x1_0000;

/// Region identifiers.
pub const RETRO_REGION_NTSC: u32 = 0;

/// Memory types.
pub const RETRO_MEMORY_MASK: u32 = 0xFF;
pub const RETRO_MEMORY_SAVE_RAM: u32 = 0;
pub const RETRO_MEMORY_SYSTEM_RAM: u32 = 2;
pub const RETRO_MEMORY_VIDEO_RAM: u32 = 3;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_match_libretro_h() {
        assert_eq!(RETRO_ENVIRONMENT_SET_MESSAGE, 6);
        assert_eq!(RETROK_RETURN, 13);
        assert_eq!(RETROK_0, 48);
        assert_eq!(RETROK_9, 57);
        assert_eq!(RETROK_x, 120);
        assert_eq!(RETROK_z, 122);
        assert_eq!(RETROK_UP, 273);
        assert_eq!(RETROK_DOWN, 274);
        assert_eq!(RETROK_RIGHT, 275);
        assert_eq!(RETROK_LEFT, 276);
    }
}
