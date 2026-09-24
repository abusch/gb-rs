mod noise;
mod tone;
mod wave;

use log::debug;
pub(crate) use noise::NoiseChannel;
pub(crate) use tone::ToneChannel;
pub(crate) use wave::WaveChannel;

use super::Timer;

#[derive(Debug, Default)]
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
}

/// Turn a digital value between $0 and $F into an analog value between -1 and 1.
pub fn dac(digital: u8) -> f32 {
    // need to map the range [0, 15] to [-1, 1]
    -(((digital << 1) as f32) / 15.0 - 1.0)
}

#[derive(Debug)]
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

    fn enable(&mut self) {
        self.length_enabled = true;
    }

    fn disable(&mut self) {
        self.length_enabled = false;
    }

    fn reset(&mut self) {
        self.length_enabled = false;
        self.length_counter = 0;
    }

    fn trigger(&mut self) {
        if self.length_counter == 0 {
            self.length_counter = self.default_length;
        }
    }
}

#[derive(Debug)]
struct VolumeEnvelope<const N: u8> {
    start_volume: u8,
    volume: u8,
    volume_increase: bool,
    timer: Timer,
}

impl<const N: u8> VolumeEnvelope<N> {
    fn new() -> Self {
        Self {
            start_volume: 0,
            volume: 0,
            volume_increase: false,
            timer: Timer::new(0),
        }
    }

    fn reload(&mut self, start_volume: u8, volume_increase: bool, envelope_period: u16) {
        self.start_volume = start_volume;
        self.volume_increase = volume_increase;
        self.timer.period = envelope_period;
        self.timer.reset();
    }

    fn tick(&mut self) {
        if self.timer.tick() {
            if self.volume_increase && self.volume < 15 {
                self.volume += 1;
                debug!("Channel {N}: increasing volume {}", self.volume);
            } else if !self.volume_increase && self.volume > 0 {
                self.volume -= 1;
                debug!("Channel {N} decreasing volume {}", self.volume);
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
