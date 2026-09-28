use bitvec::{field::BitField, order::Lsb0, view::BitView};
use log::debug;
use serde::{Deserialize, Serialize};

use crate::apu::{Timer, frame_sequencer::FrameSequencer};

use super::{LengthCounter, VolumeEnvelope, dac};

/// Linear Feedback Shift Register
#[derive(Debug, Serialize, Deserialize)]
struct Lsfr {
    reg: u16,
    width_mode: bool,
}

impl Lsfr {
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
    lsfr: Lsfr,
    timer: Timer,
    length_counter: LengthCounter,
    volume_envelope: VolumeEnvelope<4>,
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
            lsfr: Lsfr::new(),
            timer: Timer::new(4096),
            length_counter: LengthCounter::new(64),
            volume_envelope: VolumeEnvelope::new(),
            base_divisor: 0,
            shift: 0,
        }
    }

    pub(crate) fn advance(&mut self, cycles: u16) {
        for _ in 0..self.timer.advance(cycles) {
            self.lsfr.tick();
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
        let mut res = 0xFF;
        let bits = res.view_bits_mut::<Lsb0>();

        bits[4..=7].store(self.volume_envelope.start_volume);
        bits.set(3, self.volume_envelope.volume_increase);
        bits[0..=2].store(self.volume_envelope.timer.period);

        res
    }

    pub(crate) fn set_nr42(&mut self, b: u8) {
        let bits = b.view_bits::<Lsb0>();
        let start_volume = bits[4..=7].load::<u8>();
        let volume_increase = bits[3];
        // todo envelope sweep
        let envelope_period = bits[0..=2].load::<u8>() as u16;
        self.volume_envelope
            .reload(start_volume, volume_increase, envelope_period);

        if start_volume != 0 || volume_increase {
            if !self.dac_enabled {
                debug!("DAC4 turned on.");
                self.dac_enabled = true;
            }
        } else {
            debug!("DAC4 turned off. Disabling noise channel");
            self.dac_enabled = false;
            self.enabled = false;
        }
    }

    pub(crate) fn nr43(&self) -> u8 {
        let mut res = 0xFF;
        let bits = res.view_bits_mut::<Lsb0>();

        bits[4..=7].store(self.shift);
        bits.set(3, self.lsfr.width_mode);
        bits[0..=2].store(self.base_divisor);

        res
    }
    pub(crate) fn set_nr43(&mut self, b: u8) {
        let bits = b.view_bits::<Lsb0>();
        self.base_divisor = bits[0..=2].load::<u8>();
        let base_divisor = match self.base_divisor {
            0 => 8,
            n @ 1..=7 => n * 16,
            _ => unreachable!(),
        };
        self.shift = bits[4..=7].load::<u8>();
        let width = bits[3];
        // Like the other channels' frequency, it takes effect the next time the timer reloads.
        self.timer.period = (base_divisor as u16) << (self.shift as u16);
        self.lsfr.width_mode = width;
    }

    pub(crate) fn nr44(&self) -> u8 {
        let mut res = 0xff;
        let bits = res.view_bits_mut::<Lsb0>();

        bits.set(6, self.length_counter.length_enabled);

        res
    }

    pub(crate) fn set_nr44(&mut self, b: u8, frame_sequencer: &FrameSequencer) {
        let bits = b.view_bits::<Lsb0>();
        let trigger = bits[7];
        if self.length_counter.write_nrx4(
            bits[6],
            trigger,
            frame_sequencer.next_step_clocks_length(),
        ) {
            self.enabled = false;
        }

        if trigger {
            debug!("Noise channel triggered");
            self.enabled = true;
            self.lsfr.restart();
            self.timer.reset();
            self.volume_envelope.trigger();
            if !self.is_dac_on() {
                debug!("DAC4 is off, disabling noise channel");
                self.enabled = false;
            }
        }
    }

    /// Clear the registers, like powering the APU off does.
    pub(crate) fn power_off(&mut self) {
        self.dac_enabled = false;
        self.enabled = false;
        self.length_counter.power_off();
        self.volume_envelope.reset();
        self.lsfr.width_mode = false;
        self.set_nr43(0);
    }

    pub(crate) fn digital_output(&self) -> u8 {
        if self.enabled && self.lsfr.output() {
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
