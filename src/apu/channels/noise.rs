use std::ops::ShrAssign;

use bitvec::{view::BitView, order::Lsb0, field::BitField};
use log::trace;

use crate::apu::{Timer, frame_sequencer::FrameSequencer};

use super::LengthCounter;

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
        self.reg = 0xffff;
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
    start_volume: u8,
    volume: u8,
    volume_increase: bool,
    envelope_timer: Timer,
}

impl NoiseChannel {
    pub(crate) fn new() -> Self {
        Self {
            enabled: false,
            lsfr: Lsfr::new(),
            timer: Timer::new(4096),
            length_counter: LengthCounter::new(64),
            start_volume: 0,
            volume: 0,
            volume_increase: false,
            envelope_timer: Timer::new(0),
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
            self.volume_tick();
        }
    }

    fn volume_tick(&mut self) {
        if self.envelope_timer.tick() {
            if self.volume_increase && self.volume < 15 {
                self.volume += 1;
                trace!("increasing volume {}", self.volume);
            } else if !self.volume_increase && self.volume > 0 {
                self.volume -= 1;
                trace!("decreasing volume {}", self.volume);
            }
            // self.envelope_timer -= 1;
        }
        if !self.is_dac_on() {
            self.enabled = false;
        }
    }

    pub(crate) fn set_nr41(&mut self, b: u8) {
        self.length_counter.load(64 - (b & 0x3F) as u16);
    }

    pub(crate) fn set_nr42(&mut self, b: u8) {
        let bits = b.view_bits::<Lsb0>();
        self.start_volume = bits[4..=7].load::<u8>();
        self.volume_increase = bits[3];
        // todo envelope sweep
        self.envelope_timer.period = bits[0..=2].load::<u8>() as u16;
        self.envelope_timer.reset();
        trace!(
            "start_volume={}, volume_increase={}, envelope_period={}",
            self.start_volume,
            self.volume_increase,
            self.envelope_timer.period
        );
        if !self.is_dac_on() {
            self.enabled = false;
        }
    }

    pub(crate) fn set_nr43(&mut self, b: u8) {
        let bits = b.view_bits::<Lsb0>();
        let base_divisor = match bits[0..=2].load::<u8>() {
            0 => 8,
            n @ 1..=7 => n * 16,
            _ => unreachable!(),
        };
        let shift = bits[4..=7].load::<u8>();
        let width = bits[3];
        let period = (base_divisor as u16) << (shift as u16);
        self.timer.period = period;
        self.timer.reset();
        self.lsfr.width_mode = width;
    }

    pub(crate) fn set_nr44(&mut self, b: u8) {
        let bits = b.view_bits::<Lsb0>();
        if bits[6] {
            self.length_counter.enable();
        };

        if bits[7] {
            // trigger
            trace!("Noise channel triggered");
            self.enabled = true;
            self.lsfr.reset();
            self.volume = self.start_volume;
            self.envelope_timer.reset();
            self.length_counter.trigger();
            if !self.is_dac_on() {
                self.enabled = false;
            }
        }
    }

    pub(crate) fn reset(&mut self) {
        self.enabled = false;
        self.length_counter.reset();
        self.volume = self.start_volume;
        self.volume_increase = false;
    }

    pub(crate) fn output(&self) -> i16 {
        if self.enabled && self.is_dac_on() && self.lsfr.output() {
            self.volume as i16
        } else {
            0
        }
    }

    fn is_dac_on(&self) -> bool {
        self.start_volume != 0 || self.volume_increase
    }

    pub(crate) fn enabled(&self) -> bool {
        self.enabled
    }
}

