// SPDX-License-Identifier: MIT
//! N64 content and input policy for the pinned Mupen adapter. This module does
//! not instantiate a core or claim graphics/audio/checkpoint support.
use std::io::{self, Read};

pub const CORE_REVISION: &str = "15bada85906593376e8fd697f1b0b3af3a20f512";
const MAX_CARTRIDGE: usize = 64 * 1024 * 1024;

pub struct Cartridge {
    /// Canonical big-endian cartridge bytes, never written back over the source.
    pub bytes: Vec<u8>,
    pub sha256: String,
}
impl Cartridge {
    /// Recognize cartridge bytes rather than a suffix; every supported byte
    /// ordering has the same media fingerprint. It never allocates a GameId.
    pub fn read(reader: impl Read) -> io::Result<Self> {
        let mut bytes = Vec::new();
        reader
            .take(MAX_CARTRIDGE as u64 + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() < 4096 || bytes.len() > MAX_CARTRIDGE || !bytes.len().is_multiple_of(4) {
            return Err(io::Error::other("unsupported N64 cartridge length"));
        }
        match bytes[..4] {
            [0x80, 0x37, 0x12, 0x40] => {}
            [0x37, 0x80, 0x40, 0x12] => {
                for pair in bytes.chunks_exact_mut(2) {
                    pair.swap(0, 1);
                }
            }
            [0x40, 0x12, 0x37, 0x80] => {
                for word in bytes.chunks_exact_mut(4) {
                    word.reverse();
                }
            }
            _ => return Err(io::Error::other("not a supported N64 cartridge")),
        }
        Ok(Self {
            sha256: crate::storage::digest(&bytes),
            bytes,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[repr(usize)]
pub enum Button {
    A,
    B,
    Start,
    Z,
    L,
    R,
    Up,
    Down,
    Left,
    Right,
    CUp,
    CDown,
    CLeft,
    CRight,
}

#[derive(Clone, Default)]
pub struct Pad {
    buttons: [bool; 14],
    stick: [i16; 2],
}
impl Pad {
    pub fn pressed(&self, button: Button) -> bool {
        self.buttons[button as usize]
    }
    pub fn button(&mut self, button: Button, pressed: bool) {
        self.buttons[button as usize] = pressed;
    }
    /// Physical semantics: right/up positive; the libretro boundary flips Y.
    pub fn stick(&mut self, right: i16, up: i16) {
        self.stick = [right, up];
    }
    pub fn cancel(&mut self) {
        *self = Self::default();
    }
    /// Pinned core's default (non-alternate) mapping. C buttons stay digital,
    /// and L/R are independent from Z; no GameCube trigger/C-stick policy.
    pub fn input_state(&self, port: u32, device: u32, index: u32, id: u32) -> i16 {
        if port != 0 {
            return 0;
        }
        if device == 5 && index == 0 {
            return match id {
                0 => self.stick[0],
                1 => self.stick[1].saturating_neg(),
                _ => 0,
            };
        }
        if device != 1 || index != 0 {
            return 0;
        }
        let mapping = [
            Button::A,
            Button::B,
            Button::L,
            Button::Start,
            Button::Up,
            Button::Down,
            Button::Left,
            Button::Right,
            Button::CDown,
            Button::CUp,
            Button::CLeft,
            Button::CRight,
            Button::Z,
            Button::R,
        ];
        let value = |i: usize| i16::from(self.buttons[mapping[i] as usize]);
        if id == 256 {
            // RETRO_DEVICE_ID_JOYPAD_MASK
            return (0..mapping.len()).fold(0u16, |bits, i| bits | ((value(i) as u16) << i)) as i16;
        }
        if id == 0 {
            return value(0);
        } // libretro B -> N64 A
        if id == 1 {
            return value(1);
        } // libretro Y -> N64 B
        mapping.get(id as usize).map_or(0, |_| value(id as usize))
    }
}

/// Actual pinned RETRO_MEMORY_SAVE_RAM image. This is a packed libretro format,
/// not an arbitrary native .sra/.fla/.eep/.mpk import or GameCube memory card.
pub struct SaveImage<'a> {
    pub eeprom: &'a [u8],
    pub controller_paks: &'a [u8],
    pub sram: &'a [u8],
    pub flashram: &'a [u8],
}
pub const SAVE_IMAGE_BYTES: usize = 0x800 + 4 * 0x8000 + 0x8000 + 0x20000;
impl<'a> SaveImage<'a> {
    pub fn parse(bytes: &'a [u8]) -> io::Result<Self> {
        if bytes.len() != SAVE_IMAGE_BYTES {
            return Err(io::Error::other("wrong pinned Mupen save image length"));
        }
        Ok(Self {
            eeprom: &bytes[..0x800],
            controller_paks: &bytes[0x800..0x20800],
            sram: &bytes[0x20800..0x28800],
            flashram: &bytes[0x28800..],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn three_cartridge_orders_have_one_fingerprint_and_do_not_depend_on_filename() {
        let mut original = vec![0; 4096];
        original[..4].copy_from_slice(&[0x80, 0x37, 0x12, 0x40]);
        original[64..68].copy_from_slice(b"NTX!");
        let mut swapped = original.clone();
        for word in swapped.chunks_exact_mut(2) {
            word.swap(0, 1);
        }
        let mut little = original.clone();
        for word in little.chunks_exact_mut(4) {
            word.reverse();
        }
        let expected = Cartridge::read(&original[..]).unwrap();
        for input in [&swapped, &little] {
            let cart = Cartridge::read(&input[..]).unwrap();
            assert_eq!(cart.sha256, expected.sha256);
            assert_eq!(cart.bytes, original);
        }
        assert!(Cartridge::read(&b"fake.z64"[..]).is_err());
        original.push(0);
        assert!(Cartridge::read(&original[..]).is_err());
        original.truncate(4096);
        original[0] = 0;
        assert!(Cartridge::read(&original[..]).is_err());
    }
    #[test]
    fn independent_c_buttons_and_shoulders_survive_simultaneous_input_and_cancel() {
        let mut pad = Pad::default();
        for button in [
            Button::A,
            Button::Z,
            Button::L,
            Button::R,
            Button::CUp,
            Button::CRight,
        ] {
            pad.button(button, true);
        }
        pad.stick(12345, -23456);
        assert_eq!(pad.input_state(0, 5, 0, 0), 12345);
        assert_eq!(pad.input_state(0, 5, 0, 1), 23456);
        for id in [0, 2, 9, 11, 12, 13] {
            assert_eq!(pad.input_state(0, 1, 0, id), 1);
        }
        assert_eq!(pad.input_state(0, 1, 0, 10), 0);
        pad.button(Button::CRight, false);
        assert_eq!(pad.input_state(0, 1, 0, 9), 1);
        assert_eq!(
            pad.input_state(0, 1, 0, 256) as u16,
            (1 << 0) | (1 << 2) | (1 << 9) | (1 << 12) | (1 << 13)
        );
        assert_eq!(pad.input_state(1, 1, 0, 0), 0);
        pad.cancel();
        assert_eq!(pad.input_state(0, 1, 0, 256), 0);
        assert_eq!(pad.input_state(0, 5, 0, 0), 0);
    }
    #[test]
    fn pinned_save_regions_are_distinct_and_truncated_or_card_images_refuse() {
        let bytes: Vec<_> = (0..SAVE_IMAGE_BYTES).map(|i| (i / 2048) as u8).collect();
        let save = SaveImage::parse(&bytes).unwrap();
        assert_eq!(save.eeprom.len(), 2048);
        assert_eq!(save.controller_paks.len(), 131072);
        assert_eq!(save.sram.len(), 32768);
        assert_eq!(save.flashram.len(), 131072);
        assert_ne!(save.eeprom[0], save.sram[0]);
        assert!(SaveImage::parse(&bytes[..bytes.len() - 1]).is_err());
        assert!(SaveImage::parse(&[0; 68]).is_err());
    }
}
