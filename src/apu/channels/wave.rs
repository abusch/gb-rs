use bitvec::{field::BitField, order::Lsb0, view::BitView};

use crate::apu::{frame_sequencer::FrameSequencer, Timer};

use super::LengthCounter;

#[derive(Debug)]
pub(crate) struct WaveChannel {
    // Wave table containing 32 4-bit samples
    wav: [u8; 16],
    enabled: bool,
    length_counter: LengthCounter,
    output_level: OutputLevel,
    freq: u16,
    position: u8,
    freq_timer: Timer,
}

impl WaveChannel {
    pub(crate) fn new() -> Self {
        Self {
            wav: [0; 16],
            enabled: false,
            length_counter: LengthCounter::new(256),
            output_level: OutputLevel::Mute,
            freq: 0,
            position: 0,
            freq_timer: Timer::new(4096),
        }
    }

    pub(crate) fn tick(&mut self) {
        if self.freq_timer.tick() {
            self.position += 1;
            if self.position == 32 {
                self.position = 0;
            }
        }
    }

    pub fn tick_frame(&mut self, frame_sequencer: &FrameSequencer) {
        // we only care about the length here
        if frame_sequencer.length_triggered() && self.length_counter.tick() {
            self.enabled = false;
        }
    }

    pub(crate) fn nr30(&self) -> u8 {
        let mut res = 0xFF_u8;
        let bits = res.view_bits_mut::<Lsb0>();
        bits.set(7, self.enabled);

        res
    }

    pub(crate) fn set_nr30(&mut self, b: u8) {
        if b.view_bits::<Lsb0>()[7] {
            self.enabled = true;
            self.position = 0;
        } else {
            self.enabled = false;
        }
    }

    pub(crate) fn nr31(&self) -> u8 {
        // NR31 is write-only
        0xFF
    }

    pub(crate) fn set_nr31(&mut self, b: u8) {
        self.length_counter.load(256 - b as u16);
    }

    pub(crate) fn nr32(&self) -> u8 {
        let mut res = 0xFF_u8;
        let bits = res.view_bits_mut::<Lsb0>();
        bits[5..=6].store(self.output_level as u8);

        res
    }

    pub(crate) fn set_nr32(&mut self, b: u8) {
        self.output_level = match b.view_bits::<Lsb0>()[5..=6].load::<u8>() {
            0 => OutputLevel::Mute,
            1 => OutputLevel::Full,
            2 => OutputLevel::Half,
            3 => OutputLevel::Quarter,
            _ => unreachable!(),
        };
    }

    pub(crate) fn nr33(&self) -> u8 {
        0xFF
    }

    pub(crate) fn set_nr33(&mut self, b: u8) {
        self.freq.view_bits_mut::<Lsb0>()[0..=7].store(b);
    }

    pub(crate) fn nr34(&self) -> u8 {
        let mut res = 0xff;
        let bits = res.view_bits_mut::<Lsb0>();
        bits.set(6, self.length_counter.length_enabled);

        res
    }

    pub(crate) fn set_nr34(&mut self, b: u8) {
        let bits = b.view_bits::<Lsb0>();
        self.freq.view_bits_mut::<Lsb0>()[8..=10].store::<u8>(bits[0..=2].load::<u8>());

        if bits[6] {
            self.length_counter.enable();
        } else {
            self.length_counter.reset();
        }

        if bits[7] {
            // trigger
            self.position = 0;
            self.freq_timer.period = (2048 - self.freq) * 2;
            self.length_counter.trigger();
        }
    }

    pub(crate) fn read_wav(&self, idx: usize) -> u8 {
        self.wav[idx]
    }

    pub(crate) fn write_wav(&mut self, idx: usize, b: u8) {
        self.wav[idx] = b;
    }

    pub(crate) fn output(&self) -> i16 {
        if !self.enabled {
            return 0;
        }

        let byte = self.wav[self.position as usize / 2];
        let value = if self.position % 2 == 0 {
            // lower nibble
            byte & 0x0F
        } else {
            // upper nibble
            byte >> 4
        };
        let adjusted_value = self.output_level.apply(value);

        adjusted_value as i16
        // dac(adjusted_value)
    }

    pub(crate) fn reset(&mut self) {
        self.enabled = false;
        self.length_counter.reset();
        self.position = 0;
        self.output_level = OutputLevel::Mute;
    }

    pub(crate) fn enabled(&self) -> bool {
        self.enabled
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
enum OutputLevel {
    Mute = 0,
    Full = 1,
    Half = 2,
    Quarter = 3,
}

impl OutputLevel {
    fn apply(&self, value: u8) -> u8 {
        match self {
            OutputLevel::Mute => 0,
            OutputLevel::Full => value,
            OutputLevel::Half => value >> 1,
            OutputLevel::Quarter => value >> 2,
        }
    }
}
