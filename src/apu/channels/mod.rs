mod tone;
mod wave;
mod noise;

use log::trace;
pub(crate) use tone::ToneChannel;
pub(crate) use wave::WaveChannel;
pub(crate) use noise::NoiseChannel;

use super::Timer;

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
                trace!("increasing volume {}", self.volume);
            } else if !self.volume_increase && self.volume > 0 {
                self.volume -= 1;
                trace!("decreasing volume {}", self.volume);
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

    fn is_dac_on(&self) -> bool {
        self.start_volume != 0 || self.volume_increase
    }

    fn reset(&mut self) {
        self.start_volume = 0;
        self.volume = 0;
        self.volume_increase = false;
        self.timer.period = 0;
        self.timer.reset();
    }
}

