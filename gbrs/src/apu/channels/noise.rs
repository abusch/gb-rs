use log::debug;
use serde::{Deserialize, Serialize};

use crate::apu::{Timer, frame_sequencer::FrameSequencer};

use super::{LengthCounter, VolumeEnvelope, dac};

/// Linear Feedback Shift Register
#[derive(Debug, Serialize, Deserialize)]
struct Lfsr {
    reg: u16,
    width_mode: bool,
}

impl Lfsr {
    fn new() -> Self {
        Self {
            reg: 0x7FFF,
            width_mode: false,
        }
    }

    /// Reset the register, like a trigger does. With the feedback computed as a XOR and the output
    /// inverted, this is all 1s: all 0s would stay stuck at 0.
    fn restart(&mut self) {
        self.reg = 0x7FFF;
    }

    fn tick(&mut self) {
        let b = (self.reg ^ (self.reg >> 1)) & 1;
        self.reg = (self.reg >> 1) & !(1 << 14) | (b << 14);
        if self.width_mode {
            self.reg = self.reg & !(1 << 6) | (b << 6);
        }
    }

    fn output(&self) -> bool {
        // output is bit 0 *inverted*
        self.reg & 0x0001 == 0
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct NoiseChannel {
    dac_enabled: bool,
    enabled: bool,
    lfsr: Lfsr,
    timer: Timer,
    length_counter: LengthCounter,
    volume_envelope: VolumeEnvelope,
    // These 2 are used to derive the channel frequency but we need to keep them around so they can
    // be read again (via NR43)
    base_divisor: u8,
    shift: u8,
}

impl NoiseChannel {
    pub(crate) fn new() -> Self {
        Self {
            dac_enabled: false,
            enabled: false,
            lfsr: Lfsr::new(),
            timer: Timer::new(4096),
            length_counter: LengthCounter::new(64),
            volume_envelope: VolumeEnvelope::new(),
            base_divisor: 0,
            shift: 0,
        }
    }

    pub(crate) fn advance(&mut self, cycles: u16) {
        for _ in 0..self.timer.advance(cycles) {
            self.lfsr.tick();
        }
    }

    pub(crate) fn tick_frame(&mut self, frame_sequencer: &FrameSequencer) {
        if frame_sequencer.length_triggered() && self.length_counter.tick() {
            debug!("Length counter expired: disabling noise channel");
            self.enabled = false;
        }
        if frame_sequencer.vol_envelope_trigged() {
            self.volume_envelope.tick();
        }
    }

    pub(crate) fn nr41(&self) -> u8 {
        // NR41 is write-only
        0xFF
    }

    /// Only the length can be written, so this works while the APU is off too.
    pub(crate) fn set_nr41(&mut self, b: u8) {
        self.length_counter.load(64 - (b & 0x3F) as u16);
    }

    pub(crate) fn nr42(&self) -> u8 {
        self.volume_envelope.nrx2()
    }

    pub(crate) fn set_nr42(&mut self, b: u8) {
        self.dac_enabled = self.volume_envelope.write_nrx2(b);
        if !self.dac_enabled {
            self.enabled = false;
        }
    }

    pub(crate) fn nr43(&self) -> u8 {
        self.shift << 4 | (self.lfsr.width_mode as u8) << 3 | self.base_divisor
    }

    pub(crate) fn set_nr43(&mut self, b: u8) {
        self.base_divisor = b & 0x07;
        self.shift = b >> 4;
        self.lfsr.width_mode = b & 0x08 != 0;
        // A divisor code of 0 means 8, the others are multiples of 16.
        let divisor = if self.base_divisor == 0 {
            8
        } else {
            u16::from(self.base_divisor) * 16
        };
        // Like the other channels' frequency, it takes effect the next time the timer reloads.
        self.timer.period = divisor << self.shift;
    }

    pub(crate) fn nr44(&self) -> u8 {
        self.length_counter.nrx4()
    }

    pub(crate) fn set_nr44(&mut self, b: u8, frame_sequencer: &FrameSequencer) {
        if self.length_counter.write_nrx4(b, frame_sequencer) {
            self.enabled = false;
        }
        if b & 0x80 != 0 {
            debug!("Noise channel triggered");
            // Like the other channels, it stays off if its DAC is.
            self.enabled = self.is_dac_on();
            self.lfsr.restart();
            self.timer.reset();
            self.volume_envelope.trigger();
        }
    }

    /// Clear the registers, like powering the APU off does.
    pub(crate) fn power_off(&mut self) {
        self.dac_enabled = false;
        self.enabled = false;
        self.length_counter.power_off();
        self.volume_envelope.reset();
        self.lfsr.width_mode = false;
        self.set_nr43(0);
    }

    pub(crate) fn digital_output(&self) -> u8 {
        if self.enabled && self.lfsr.output() {
            self.volume_envelope.volume()
        } else {
            0
        }
    }

    pub(crate) fn output(&self) -> f32 {
        if self.is_dac_on() {
            dac(self.digital_output())
        } else {
            0.0
        }
    }

    pub(crate) fn is_dac_on(&self) -> bool {
        self.dac_enabled
    }

    pub(crate) fn enabled(&self) -> bool {
        self.enabled
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_noise_after_trigger() {
        let mut channel = NoiseChannel::new();
        channel.set_nr42(0xF0); // DAC on, full volume
        channel.set_nr43(0x00); // fastest clock, 15-bit LFSR
        channel.set_nr44(0x80, &FrameSequencer::default()); // trigger
        let mut outputs = std::collections::HashSet::new();
        for _ in 0..1000 {
            channel.advance(8);
            outputs.insert(channel.digital_output());
        }
        // Both 0 and the full volume show up, so the LFSR isn't stuck.
        assert_eq!(outputs.len(), 2, "outputs: {outputs:?}");
    }
}
