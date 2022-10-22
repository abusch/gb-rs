use std::ops::ShrAssign;

use bitvec::{field::BitField, order::Lsb0, view::BitView};
use log::trace;

use crate::apu::{frame_sequencer::FrameSequencer, Timer};

use super::{LengthCounter, VolumeEnvelope};

/// Linear Feedback Shift Register
#[derive(Debug)]
struct Lsfr {
    reg: u16,
    width_mode: bool,
}

impl Lsfr {
    fn new() -> Self {
        Self {
            reg: 0xffff,
            width_mode: false,
        }
    }

    fn reset(&mut self) {
        self.reg = 0x0000;
        self.width_mode = false;
    }

    fn tick(&mut self) {
        let bits = self.reg.view_bits::<Lsb0>();
        let b = bits[0] ^ bits[1];
        self.reg.shr_assign(1);

        let bits = self.reg.view_bits_mut::<Lsb0>();
        bits.set(14, b);
        if self.width_mode {
            bits.set(6, b);
        }
    }

    fn output(&self) -> bool {
        // output is bit 0 *inverted*
        self.reg & 0x0001 == 0
    }
}

#[derive(Debug)]
pub(crate) struct NoiseChannel {
    enabled: bool,
    lsfr: Lsfr,
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
            enabled: false,
            lsfr: Lsfr::new(),
            timer: Timer::new(4096),
            length_counter: LengthCounter::new(64),
            volume_envelope: VolumeEnvelope::new(),
            base_divisor: 0,
            shift: 0,
        }
    }

    pub(crate) fn tick(&mut self) {
        if self.timer.tick() {
            self.lsfr.tick();
        }
    }

    pub(crate) fn tick_frame(&mut self, frame_sequencer: &FrameSequencer) {
        if frame_sequencer.length_triggered() && self.length_counter.tick() {
            self.enabled = false;
        }
        if frame_sequencer.vol_envelope_trigged() {
            self.volume_envelope.tick();
            if !self.volume_envelope.is_dac_on() {
                self.enabled = false;
            }
        }
    }

    pub(crate) fn nr41(&self) -> u8 {
        // NR41 is write-only
        0xFF
    }

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
        if !self.is_dac_on() {
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
        let period = (base_divisor as u16) << (self.shift as u16);
        self.timer.period = period;
        self.timer.reset();
        self.lsfr.width_mode = width;
    }

    pub(crate) fn nr44(&self) -> u8 {
        let mut res = 0xff;
        let bits = res.view_bits_mut::<Lsb0>();

        bits.set(6, self.length_counter.length_enabled);

        res
    }

    pub(crate) fn set_nr44(&mut self, b: u8) {
        let bits = b.view_bits::<Lsb0>();
        if bits[6] {
            self.length_counter.enable();
        } else {
            self.length_counter.reset();
        }

        if bits[7] {
            // trigger
            trace!("Noise channel triggered");
            self.enabled = true;
            self.lsfr.reset();
            self.volume_envelope.trigger();
            self.length_counter.trigger();
            if !self.is_dac_on() {
                self.enabled = false;
            }
        }
    }

    pub(crate) fn reset(&mut self) {
        self.enabled = false;
        self.length_counter.reset();
        self.volume_envelope.reset();
        self.lsfr.reset();
        self.base_divisor = 0;
        self.shift = 0;
        self.timer.reset();
    }

    pub(crate) fn output(&self) -> i16 {
        if self.enabled && self.is_dac_on() && self.lsfr.output() {
            self.volume_envelope.volume() as i16
        } else {
            0
        }
    }

    fn is_dac_on(&self) -> bool {
        self.volume_envelope.is_dac_on()
    }

    pub(crate) fn enabled(&self) -> bool {
        self.enabled
    }
}
