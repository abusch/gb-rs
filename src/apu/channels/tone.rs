use bitvec::{view::BitView, order::Lsb0, field::BitField};
use log::trace;

use crate::apu::{Timer, frame_sequencer::FrameSequencer};

use super::LengthCounter;

#[derive(Debug)]
pub(crate) struct ToneChannel {
    enabled: bool,
    length_counter: LengthCounter,

    start_volume: u8,
    volume: u8,
    volume_increase: bool,
    // envelope_period: u8,
    // envelope_timer: u8,
    envelope_timer: Timer,

    freq_hi: u8,
    freq_lo: u8,

    freq_timer: Timer,
    wave_generator: SquareWaveGenerator,
}

impl ToneChannel {
    pub(crate) fn new() -> Self {
        Self {
            enabled: false,
            length_counter: LengthCounter::new(64),
            start_volume: 0,
            volume: 0,
            volume_increase: false,
            // envelope_period: 0,
            // envelope_timer: 0,
            envelope_timer: Timer::new(0),
            freq_hi: 0,
            freq_lo: 0,
            freq_timer: Timer::new(8192),
            wave_generator: SquareWaveGenerator::new(),
        }
    }

    pub(crate) fn tick(&mut self) {
        if self.freq_timer.tick() {
            self.wave_generator.tick();
        }
    }

    pub(crate) fn tick_frame(&mut self, frame_sequencer: &FrameSequencer) {
        if frame_sequencer.length_triggered() && self.length_counter.tick() {
            // The length counter expired: disable the channel
            self.enabled = false;
        }
        if frame_sequencer.vol_envelope_trigged() {
            self.volume_tick();
        }
        if frame_sequencer.sweep_triggered() {
            // TODO
        }
    }

    pub(crate) fn set_nrx0(&mut self, b: u8) {
        // let bits = b.view_bits::<Lsb0>();
        // TODO
    }

    pub(crate) fn set_nrx1(&mut self, b: u8) {
        trace!("setting NRx1 to {:08b}", b);
        let bits = b.view_bits::<Lsb0>();

        let duty = bits[6..=7].load::<u8>().into();
        self.wave_generator.set_duty(duty);
        trace!("duty = {:?}", duty);

        let length = bits[0..=5].load::<u8>();
        trace!("length = {}", length);
        self.length_counter.load(64 - length as u16);
    }

    pub(crate) fn set_nrx2(&mut self, b: u8) {
        trace!("setting NRx2 to {:08b}", b);
        let bits = b.view_bits::<Lsb0>();
        self.start_volume = bits[4..=7].load::<u8>();
        self.volume_increase = bits[3];
        self.envelope_timer.period = bits[0..=2].load::<u8>() as u16;
        self.envelope_timer.reset();
        trace!(
            "start_volume={}, volume_increase={}, envelope_period={}",
            self.start_volume,
            self.volume_increase,
            self.envelope_timer.period
        );
        // Not sure why the docs said to do this? This is wrong...
        // if self.envelope_timer.period == 0 {
        //     self.envelope_timer.period = 8;
        // }
        if !self.is_dac_on() {
            trace!("DAC is off, disabling channel");
            self.enabled = false;
        }
    }

    pub(crate) fn set_nrx3(&mut self, b: u8) {
        trace!("setting NRx3 to {:08b}", b);
        self.freq_lo = b;
    }

    pub(crate) fn set_nrx4(&mut self, b: u8) {
        trace!("setting NRx4 to {:08b}", b);
        let bits = b.view_bits::<Lsb0>();

        if bits[6] {
            self.length_counter.enable();
        }
        self.freq_hi = bits[0..=2].load::<u8>();

        if bits[7] {
            // Trigger
            self.enabled = true;
            self.length_counter.trigger();
            let freq = ((self.freq_hi as u16) << 8) + self.freq_lo as u16;
            if freq == 0 {
                // should we do this?
                self.enabled = false;
            }
            self.freq_timer.period = (2048 - freq) * 4;
            self.freq_timer.reset();
            // Reset volume envelope
            self.volume = self.start_volume;
            self.envelope_timer.reset();
            if !self.is_dac_on() {
                // If DAC is off, disable the channel
                trace!("DAC is off, disabling channel");
                self.enabled = false;
            }
            // TODO  sweep, etc..
        }
    }

    pub(crate) fn output(&self) -> i16 {
        if self.enabled && self.is_dac_on() && self.wave_generator.output() {
            self.volume as i16
        } else {
            0
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
    }

    fn is_dac_on(&self) -> bool {
        self.start_volume != 0 || self.volume_increase
    }

    pub(crate) fn reset(&mut self) {
        trace!("Resetting square channel");
        self.enabled = false;
        self.start_volume = 0;
        self.volume = 0;
        self.length_counter.reset();
        // self.envelope_period = 0;
        // self.envelope_timer = 0;
        self.envelope_timer.period = 0;
        self.envelope_timer.reset();
        self.volume_increase = false;
        self.freq_hi = 0;
        self.freq_lo = 0;
        self.freq_timer.reset();
    }

    pub(crate) fn enabled(&self) -> bool {
        self.enabled
    }
}

#[derive(Debug)]
struct SquareWaveGenerator {
    duty: Duty,
    step: u8,
}

impl SquareWaveGenerator {
    fn new() -> Self {
        Self {
            duty: Duty::Duty0,
            step: 0,
        }
    }

    pub fn tick(&mut self) {
        self.step = (self.step + 1) % 8;
    }

    pub fn set_duty(&mut self, duty: Duty) {
        self.duty = duty;
    }

    pub fn output(&self) -> bool {
        match self.duty {
            Duty::Duty0 => self.step == 7,
            Duty::Duty1 => self.step == 0 || self.step == 7,
            Duty::Duty2 => self.step == 0 || self.step == 5 || self.step == 6 || self.step == 7,
            Duty::Duty3 => self.step != 0 && self.step != 7,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
enum Duty {
    Duty0 = 0,
    Duty1 = 1,
    Duty2 = 2,
    Duty3 = 3,
}

impl From<u8> for Duty {
    fn from(d: u8) -> Self {
        match d {
            0 => Duty::Duty0,
            1 => Duty::Duty1,
            2 => Duty::Duty2,
            3 => Duty::Duty3,
            _ => panic!("Unsupported value for Duty enum: {}", d),
        }
    }
}
