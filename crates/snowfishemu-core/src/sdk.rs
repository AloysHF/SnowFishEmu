//! Adapters for SDK services that run inside the guest image.

use crate::consts::CODE_BASE;
use crate::mem::{GuestMem, MemError};
use crate::sfa::SfaImage;

/// Finds the SDK's font-string-width entry. The SDK returns before calling
/// the platform width slot when the font handle is null.
pub(crate) fn default_font_width_entry(image: &SfaImage) -> Option<u32> {
    const PRELUDE: [u32; 9] = [
        0xe351_0000,
        0x1350_0000,
        0xe92d_4070,
        0xe1a0_5000,
        0xe1a0_4001,
        0x13a0_6000,
        0x03a0_6001,
        0x0a00_0007,
        0xe1a0_0001,
    ];
    const EPILOGUE: [u32; 8] = [
        0xe1a0_1004,
        0xe1a0_3000,
        0xe1a0_2006,
        0xe1a0_0005,
        0xe8bd_4070,
        0xeaff_ffe6,
        0xe3a0_0000,
        0xe8bd_8070,
    ];
    let mut entries = image
        .code
        .windows(72)
        .step_by(4)
        .enumerate()
        .filter(|(_, bytes)| {
            // The only layout-dependent instruction calls the length helper.
            bytes[39] == 0xeb
                && bytes[..36]
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .zip(PRELUDE)
                    .all(|(word, opcode)| *word == opcode.to_le_bytes())
                && bytes[40..]
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .zip(EPILOGUE)
                    .all(|(word, opcode)| *word == opcode.to_le_bytes())
        })
        .map(|(index, _)| CODE_BASE + index as u32 * 4);
    let entry = entries.next()?;
    entries.next().is_none().then_some(entry)
}

/// The legacy SDK registers a local check in a state object instead of
/// passing a request descriptor to the platform.
pub(crate) struct LegacyCheck {
    state_slot: u32,
    code_end: u32,
}

impl LegacyCheck {
    // The registration routine stores the callback, context and pending
    // flag in the SDK state. Its trailing literal contains the state slot.
    const REGISTER: [u32; 16] = [
        0xe59f_c038,
        0xe59c_c000,
        0xe92d_4010,
        0xe59c_e17c,
        0xe58e_1004,
        0xe59d_1008,
        0xe3a0_4000,
        0xe58c_3064,
        0xe3a0_3001,
        0xe58c_4018,
        0xe58c_1068,
        0xe58e_2000,
        0xe58c_01b4,
        0xe58c_313c,
        0xe58c_4010,
        0xe8bd_8010,
    ];

    pub(crate) fn detect(image: &SfaImage) -> Option<Self> {
        let code_end = CODE_BASE + image.header.code_size;
        let image_end = CODE_BASE + image.header.image_size();
        let size = Self::REGISTER.len() * 4;
        let mut slots = image
            .code
            .windows(size + 4)
            .step_by(4)
            .filter(|bytes| {
                bytes
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .zip(Self::REGISTER)
                    .all(|(word, opcode)| *word == opcode.to_le_bytes())
            })
            .map(|bytes| u32::from_le_bytes(bytes[size..].try_into().unwrap()))
            .filter(|slot| slot.is_multiple_of(4) && (code_end..=image_end - 4).contains(slot));
        let state_slot = slots.next()?;
        slots.next().is_none().then_some(Self {
            state_slot,
            code_end,
        })
    }

    pub(crate) fn take_completion(
        &self,
        mem: &mut GuestMem,
    ) -> Result<Option<(u32, u32)>, MemError> {
        let state = mem.read_u32(self.state_slot)?;
        if state == 0 || mem.read_u32(state.wrapping_add(0x13c)).ok() != Some(1) {
            return Ok(None);
        }
        let callback = mem.read_u32(state + 0x64)?;
        if !callback.is_multiple_of(4) || !(CODE_BASE..self.code_end).contains(&callback) {
            return Ok(None);
        }
        let context = mem.read_u32(state + 0x68)?;
        // Release the event dispatcher before calling back into the guest.
        mem.write_u32(state + 0x13c, 0)?;
        mem.write_u32(state + 0x18, u32::MAX)?;
        mem.write_u32(state + 0x10, 0)?;
        Ok(Some((callback, context)))
    }
}

/// A tariff request registered by the SDK before it queries the handset.
pub(crate) struct TariffCheck {
    return_pc: u32,
    callback: u32,
    context: u32,
    pending: u32,
    screen: u32,
    code_end: u32,
}

impl TariffCheck {
    // Compare the registration and channel-unavailable paths. Literal
    // displacements and the two called functions depend on link layout.
    const REGISTER: [u32; 21] = [
        0xe59f_3000,
        0xe59d_c01c,
        0xe583_a000,
        0xe59f_3000,
        0xe583_c000,
        0xe59f_3000,
        0xe3a0_6000,
        0xe583_6000,
        0xe59f_3000,
        0xe1a0_2007,
        0xe59f_1000,
        0xe583_6000,
        0xe59f_0000,
        0xeb00_0000,
        0xeb00_0000,
        0xe150_0006,
        0x1a00_0003,
        0xe59f_3000,
        0xe3a0_2006,
        0xe583_2000,
        0xe8bd_85f0,
    ];

    pub(crate) fn detect(image: &SfaImage) -> Option<Self> {
        let code_end = CODE_BASE + image.header.code_size;
        let image_end = CODE_BASE + image.header.image_size();
        let read = |offset: usize| {
            Some(u32::from_le_bytes(
                image.code.get(offset..offset + 4)?.try_into().ok()?,
            ))
        };
        let mut checks =
            image
                .code
                .windows(Self::REGISTER.len() * 4)
                .step_by(4)
                .enumerate()
                .filter_map(|(index, bytes)| {
                    let matches = bytes.as_chunks::<4>().0.iter().zip(Self::REGISTER).all(
                        |(word, opcode)| {
                            let word = u32::from_le_bytes(*word);
                            let mask = match opcode {
                                0xe59f_0000 | 0xe59f_1000 | 0xe59f_3000 => 0xffff_f000,
                                0xeb00_0000 => 0xff00_0000,
                                _ => u32::MAX,
                            };
                            word & mask == opcode
                        },
                    );
                    if !matches {
                        return None;
                    }
                    let literal = |relative| {
                        let offset = index * 4 + relative;
                        let displacement = (read(offset)? & 0xfff) as usize;
                        let address = read(offset + 8 + displacement)?;
                        (address.is_multiple_of(4) && (code_end..=image_end - 4).contains(&address))
                            .then_some(address)
                    };
                    let callback = literal(0)?;
                    let context = literal(12)?;
                    let pending = literal(32)?;
                    let screen = literal(68)?;
                    if context != callback + 4 || pending >= callback || screen <= context {
                        return None;
                    }
                    Some(Self {
                        return_pc: CODE_BASE + index as u32 * 4 + 60,
                        callback,
                        context,
                        pending,
                        screen,
                        code_end,
                    })
                });
        let check = checks.next()?;
        checks.next().is_none().then_some(check)
    }

    pub(crate) fn is_channel_query(&self, pc: u32, mem: &GuestMem) -> bool {
        pc == self.return_pc && self.registered_callback(mem).is_some()
    }

    fn registered_callback(&self, mem: &GuestMem) -> Option<u32> {
        let callback = mem.read_u32(self.callback).ok()?;
        (callback.is_multiple_of(4) && (CODE_BASE..self.code_end).contains(&callback))
            .then_some(callback)
    }

    pub(crate) fn take_completion(
        &self,
        mem: &mut GuestMem,
    ) -> Result<Option<(u32, u32)>, MemError> {
        const CHANNEL_UNAVAILABLE: u32 = 6;
        if mem.read_u32(self.screen)? != CHANNEL_UNAVAILABLE {
            return Ok(None);
        }
        let Some(callback) = self.registered_callback(mem) else {
            return Ok(None);
        };
        let context = mem.read_u32(self.context)?;
        mem.write_u32(self.screen, 0)?;
        mem.write_u32(self.pending, 0)?;
        Ok(Some((callback, context)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::consts::HEAP_BASE;
    use crate::sfa::{HEADER_LEN_SFA, MAGIC_SFA};

    #[test]
    fn legacy_check_uses_the_relocated_state_and_registered_callback() {
        let mut image = Vec::new();
        let code_size: u32 = 0x80;
        let state_offset = code_size + 4;
        for word in [MAGIC_SFA, 1, 0, 0, HEADER_LEN_SFA, code_size, 16, 1] {
            image.extend_from_slice(&word.to_le_bytes());
        }
        image.extend_from_slice(&[0; 16]);
        for word in LegacyCheck::REGISTER {
            image.extend_from_slice(&word.to_le_bytes());
        }
        image.extend_from_slice(&state_offset.to_le_bytes());
        image.resize((HEADER_LEN_SFA + code_size) as usize, 0);
        image.extend_from_slice(&(16 + 64u32).to_le_bytes());
        let image = SfaImage::parse(&image).unwrap();
        let check = LegacyCheck::detect(&image).unwrap();
        let mut mem = GuestMem::new(image.header.image_size());
        let state = HEAP_BASE + 0x40;
        let callback = CODE_BASE + 0x70;
        mem.write_u32(CODE_BASE + state_offset, state).unwrap();
        mem.write_u32(state + 0x64, callback).unwrap();
        mem.write_u32(state + 0x68, HEAP_BASE + 0x200).unwrap();
        mem.write_u32(state + 0x13c, 1).unwrap();
        assert_eq!(
            check.take_completion(&mut mem).unwrap(),
            Some((callback, HEAP_BASE + 0x200))
        );
        assert_eq!(check.take_completion(&mut mem).unwrap(), None);
        assert_eq!(mem.read_u32(state + 0x18).unwrap(), u32::MAX);
    }

    #[test]
    fn tariff_channel_query_and_completion_use_the_sdk_registration() {
        let code_size = 0x100u32;
        let callback_slot = code_size + 4;
        let context_slot = code_size + 8;
        let pending_slot = code_size;
        let screen_slot = code_size + 12;
        let mut ops = TariffCheck::REGISTER.to_vec();
        ops.resize(0x40, 0);
        for (insn, slot, literal) in [
            (0, callback_slot, 21),
            (3, context_slot, 22),
            (8, pending_slot, 23),
            (17, screen_slot, 24),
        ] {
            ops[insn] |= ((literal - insn - 2) * 4) as u32;
            ops[literal] = slot;
        }
        let mut package = Vec::new();
        for word in [MAGIC_SFA, 1, 0, 0, HEADER_LEN_SFA, code_size, 16, 4] {
            package.extend_from_slice(&word.to_le_bytes());
        }
        for word in ops.into_iter().chain([84u32, 88, 92, 96]) {
            package.extend_from_slice(&word.to_le_bytes());
        }
        let image = SfaImage::parse(&package).unwrap();
        let check = TariffCheck::detect(&image).unwrap();
        let mut mem = GuestMem::new(image.header.image_size());
        let callback = CODE_BASE + 0xc0;
        mem.write_u32(CODE_BASE + callback_slot, callback).unwrap();
        mem.write_u32(CODE_BASE + context_slot, HEAP_BASE).unwrap();
        assert!(!check.is_channel_query(CODE_BASE + 8, &mem));
        assert!(check.is_channel_query(CODE_BASE + 60, &mem));
        assert_eq!(check.take_completion(&mut mem).unwrap(), None);
        mem.write_u32(CODE_BASE + screen_slot, 6).unwrap();
        mem.write_u32(CODE_BASE + pending_slot, 1).unwrap();
        assert_eq!(
            check.take_completion(&mut mem).unwrap(),
            Some((callback, HEAP_BASE))
        );
        assert_eq!(mem.read_u32(CODE_BASE + pending_slot).unwrap(), 0);
        assert_eq!(check.take_completion(&mut mem).unwrap(), None);
    }
}
