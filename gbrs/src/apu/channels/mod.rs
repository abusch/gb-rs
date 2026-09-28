mod noise;
mod tone;
mod wave;

pub(crate) use noise::NoiseChannel;
use serde::{Deserialize, Serialize};
pub(crate) use tone::ToneChannel;
pub(crate) use wave::WaveChannel;

use super::{Timer, frame_sequencer::FrameSequencer};

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct HighPassFilter {
    capacitor: f32,
}

impl HighPassFilter {
    /// Charge factor of the capacitor for a target sample rage of 44.1kHz.
    ///
    /// See https://gbdev.io/pandocs/Audio_details.html#obscure-behavior
    const CHARGE_FACTOR: f32 = 0.996;

    /// Convert the given digital input (from 0 to 15) to an analog value between -1.0 and 1.0, and
    /// apply a high-pass filter.
    pub fn apply(&mut self, input: f32, dacs_enabled: bool) -> f32 {
        if dacs_enabled {
            // Apply HPF
            let out = input - self.capacitor;
            self.capacitor = input - out * Self::CHARGE_FACTOR;
            out
        } else {
            // if *all* DACs are off, output 0.0.
            0.0
        }
    }

    /// Charge the capacitor as if `input` had been applied for a long time, so that it doesn't
    /// produce a transient.
    pub fn settle(&mut self, input: f32) {
        self.capacitor = input;
    }
}

/// Turn a digital value between $0 and $F into an analog value between -1 and 1.
pub fn dac(digital: u8) -> f32 {
    // need to map the range [0, 15] to [-1, 1]
    -(((digital << 1) as f32) / 15.0 - 1.0)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct LengthCounter {
    length_enabled: bool,
    length_counter: u16,
    default_length: u16,
}

impl LengthCounter {
    pub fn new(default_length: u16) -> Self {
        Self {
            length_enabled: false,
            length_counter: 0,
            default_length,
        }
    }

    fn tick(&mut self) -> bool {
        if self.length_enabled && self.length_counter > 0 {
            self.length_counter -= 1;
        }

        // Return true if the length counter is enabled and the counter has reached 0
        self.length_enabled && self.length_counter == 0
    }

    fn load(&mut self, length: u16) {
        self.length_counter = length;
    }

    /// NRx4, where only the length enable bit (6) can be read back.
    fn nrx4(&self) -> u8 {
        0xBF | (self.length_enabled as u8) << 6
    }

    /// Handle a write to NRx4, which enables or disables length counting (bit 6) and can trigger
    /// the channel (bit 7). Return whether that disables the channel.
    ///
    /// Enabling length counting when the frame sequencer's next step doesn't clock it clocks it
    /// once straight away, and a trigger loads a counter that reached 0 with the maximum, minus
    /// that extra clock. See <https://gbdev.io/pandocs/Audio_details.html#obscure-behavior>.
    fn write_nrx4(&mut self, b: u8, frame_sequencer: &FrameSequencer) -> bool {
        let enable = b & 0x40 != 0;
        let trigger = b & 0x80 != 0;
        let extra_clock = enable && !frame_sequencer.next_step_clocks_length();
        let mut disable = false;
        if extra_clock && !self.length_enabled && self.length_counter > 0 {
            self.length_counter -= 1;
            disable = self.length_counter == 0 && !trigger;
        }
        self.length_enabled = enable;
        if trigger && self.length_counter == 0 {
            self.length_counter = self.default_length;
            if extra_clock {
                self.length_counter -= 1;
            }
        }
        disable
    }

    /// Powering the APU off disables length counting, but on the DMG it keeps the counters.
    fn power_off(&mut self) {
        self.length_enabled = false;
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct VolumeEnvelope {
    start_volume: u8,
    volume: u8,
    volume_increase: bool,
    timer: Timer,
}

impl VolumeEnvelope {
    fn new() -> Self {
        Self {
            start_volume: 0,
            volume: 0,
            volume_increase: false,
            timer: Timer::new(0),
        }
    }

    /// NRx2: the start volume, direction and period.
    fn nrx2(&self) -> u8 {
        self.start_volume << 4 | (self.volume_increase as u8) << 3 | self.timer.period as u8
    }

    /// Write NRx2. Return whether the channel's DAC is on, which it is unless both the start
    /// volume and the direction are 0.
    fn write_nrx2(&mut self, b: u8) -> bool {
        self.start_volume = b >> 4;
        self.volume_increase = b & 0x08 != 0;
        self.timer.period = u16::from(b & 0x07);
        self.timer.reset();
        b & 0xF8 != 0
    }

    fn tick(&mut self) {
        if self.timer.tick() {
            if self.volume_increase && self.volume < 15 {
                self.volume += 1;
            } else if !self.volume_increase && self.volume > 0 {
                self.volume -= 1;
            }
        }
    }

    fn volume(&self) -> u8 {
        self.volume
    }

    fn trigger(&mut self) {
        self.volume = self.start_volume;
        self.timer.reset();
    }

    fn reset(&mut self) {
        self.start_volume = 0;
        self.volume = 0;
        self.volume_increase = false;
        self.timer.period = 0;
        self.timer.reset();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dac() {
        assert_eq!(dac(0x0), 1.0);
        assert_eq!(dac(0xF), -1.0);
    }
}
