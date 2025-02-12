use bitvec::{field::BitField, order::Lsb0, view::BitView};
use log::{debug, trace};

use crate::apu::{frame_sequencer::FrameSequencer, Timer};

use super::{dac, LengthCounter, VolumeEnvelope};
#[derive(Debug)]
pub(crate) struct ToneChannel<const N: u8> {
    dac_enabled: bool,
    enabled: bool,
    length_counter: LengthCounter,

    volume_envelope: VolumeEnvelope<N>,

    freq_hi: u8,
    freq_lo: u8,

    freq_timer: Timer,
    frequency_sweep: Option<FrequencySweep>,
    wave_generator: SquareWaveGenerator,
}

impl<const N: u8> ToneChannel<N> {
    pub(crate) fn new(with_frequency_sweep: bool) -> Self {
        Self {
            dac_enabled: false,
            enabled: false,
            length_counter: LengthCounter::new(64),
            volume_envelope: VolumeEnvelope::new(),
            freq_hi: 0,
            freq_lo: 0,
            freq_timer: Timer::new(8192),
            frequency_sweep: if with_frequency_sweep {
                Some(FrequencySweep::new())
            } else {
                None
            },
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
            debug!("Length counter expired: disabling tone channel");
            self.enabled = false;
        }
        if frame_sequencer.vol_envelope_trigged() {
            self.volume_envelope.tick();
        }
        if frame_sequencer.sweep_triggered() {
            if let Some(ref mut sweep) = self.frequency_sweep {
                match sweep.tick() {
                    FrequencySweepResult::NewFreq(f) => self.freq_timer.period = (2048 - f) * 4,
                    FrequencySweepResult::Disable => self.enabled = false,
                    FrequencySweepResult::Nop => (),
                }
            }
        }
    }

    pub(crate) fn nrx0(&self) -> u8 {
        let mut res = 0xFF;
        let bits = res.view_bits_mut::<Lsb0>();
        if let Some(ref sweep) = self.frequency_sweep {
            // bit 7 is always set
            bits.set(7, true);
            bits[4..=6].store(sweep.timer.period as u8);
            bits.set(3, sweep.should_negate);
            bits[0..=2].store(sweep.shift);
        }

        res
    }

    pub(crate) fn set_nrx0(&mut self, b: u8) {
        if let Some(ref mut sweep) = self.frequency_sweep {
            let bits = b.view_bits::<Lsb0>();
            let sweep_time = bits[4..=6].load::<u8>();
            let negate = bits[3];
            let shift = bits[0..=2].load::<u8>();
            sweep.load(sweep_time as u16, negate, shift);
        }
    }

    pub(crate) fn nrx1(&self) -> u8 {
        let mut res = 0xFF;
        let bits = res.view_bits_mut::<Lsb0>();
        bits[6..=7].store(self.wave_generator.duty as u8);

        res
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

    pub(crate) fn nrx2(&self) -> u8 {
        let mut res = 0xFF;
        let bits = res.view_bits_mut::<Lsb0>();

        bits[4..=7].store(self.volume_envelope.start_volume);
        bits.set(3, self.volume_envelope.volume_increase);
        bits[0..=2].store(self.volume_envelope.timer.period);

        res
    }

    pub(crate) fn set_nrx2(&mut self, b: u8) {
        trace!("setting NRx2 to {:08b}", b);
        let bits = b.view_bits::<Lsb0>();
        let start_volume = bits[4..=7].load::<u8>();
        let volume_increase = bits[3];
        let envelope_period = bits[0..=2].load::<u8>() as u16;
        // Not sure why the docs said to do this? This is wrong...
        // if envelope_period == 0 {
        //     envelope_period = 8;
        // }
        self.volume_envelope
            .reload(start_volume, volume_increase, envelope_period);
        debug!(
            "Channel {N}: volume envelope = {start_volume}, {volume_increase}, {envelope_period}"
        );
        if start_volume != 0 || volume_increase {
            if !self.dac_enabled {
                debug!("Channel {N}: DAC turned on");
                self.dac_enabled = true;
            }
        } else {
            debug!("Channel {N}: DAC turned off, disabling channel");
            self.dac_enabled = false;
            self.enabled = false;
        }
    }

    pub(crate) fn nrx3(&self) -> u8 {
        // NRx3 is write-only
        0xFF
    }

    pub(crate) fn set_nrx3(&mut self, b: u8) {
        trace!("setting NRx3 to {:08b}", b);
        self.freq_lo = b;
    }

    pub(crate) fn nrx4(&self) -> u8 {
        let mut res = 0xFF;
        let bits = res.view_bits_mut::<Lsb0>();
        // only bit 6 can be read back
        bits.set(6, self.length_counter.length_enabled);
        trace!("Returning {:08b} for NRx4", res);
        res
    }

    pub(crate) fn set_nrx4(&mut self, b: u8) {
        if !self.is_dac_on() {
            debug!("Channel {N}: DAC is off, ignoring write to NRx4");
            return;
        }
        trace!("Channel {N}: setting NRx4 to {:08b}", b);
        let bits = b.view_bits::<Lsb0>();

        if bits[6] {
            self.length_counter.enable();
        } else {
            self.length_counter.disable();
        }
        self.freq_hi = bits[0..=2].load::<u8>();

        if bits[7] {
            debug!("Channel {N}: Tone channel triggered");
            // Trigger
            self.enabled = true;
            self.length_counter.trigger();
            let freq = ((self.freq_hi as u16) << 8) + self.freq_lo as u16;
            // if freq == 0 {
            //     // should we do this?
            //     self.enabled = false;
            // }
            self.freq_timer.period = (2048 - freq) * 4;
            self.freq_timer.reset();
            // Reset volume envelope
            self.volume_envelope.trigger();
            if let Some(ref mut sweep) = self.frequency_sweep {
                sweep.trigger(freq);
            }
            // if !self.is_dac_on() {
            //     // If DAC is off, disable the channel
            //     debug!("Channel {N}: Tone channel DAC is off, disabling channel");
            //     self.enabled = false;
            // }
            // TODO  sweep, etc..
        }
    }

    pub(crate) fn digital_output(&self) -> u8 {
        if self.enabled && self.wave_generator.output() {
            self.volume_envelope.volume()
        } else {
            // If the channel is disabled, return *digital* 0
            0
        }
    }

    pub(crate) fn output(&self) -> f32 {
        if self.is_dac_on() {
            dac(self.digital_output())
        } else {
            // If the DAC is off, *always* output analog 0.0
            0.0
        }
    }

    pub(crate) fn is_dac_on(&self) -> bool {
        self.dac_enabled
    }

    pub(crate) fn reset(&mut self) {
        trace!("Resetting square channel");
        self.dac_enabled = false;
        self.enabled = false;
        self.volume_envelope.reset();
        self.length_counter.reset();
        self.freq_hi = 0;
        self.freq_lo = 0;
        self.freq_timer.reset();
        self.wave_generator.reset();
        if let Some(ref mut sweep) = self.frequency_sweep {
            sweep.reset()
        }
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

    pub fn reset(&mut self) {
        self.duty = Duty::Duty0;
        self.step = 0;
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

#[derive(Debug, Clone, Copy)]
enum FrequencySweepResult {
    NewFreq(u16),
    Disable,
    Nop,
}

#[derive(Debug)]
struct FrequencySweep {
    enabled: bool,
    shadow_register: u16,
    should_negate: bool,
    timer: Timer,
    shift: u8,
}

impl FrequencySweep {
    fn new() -> Self {
        Self {
            enabled: false,
            shadow_register: 0,
            should_negate: false,
            timer: Timer::new(0),
            shift: 0,
        }
    }

    fn tick(&mut self) -> FrequencySweepResult {
        if self.timer.tick() && self.enabled && self.shift != 0 {
            let delta = self.shadow_register >> self.shift as u16;
            let new_freq = if self.should_negate {
                self.shadow_register.wrapping_sub(delta)
            } else {
                self.shadow_register.wrapping_add(delta)
            };
            let overflow = self.shadow_register > 2047;
            if overflow {
                return FrequencySweepResult::Disable;
            } else {
                self.shadow_register = new_freq;
                return FrequencySweepResult::NewFreq(new_freq);
            }
        }

        FrequencySweepResult::Nop
    }

    fn load(&mut self, sweep_time: u16, negate: bool, shift: u8) {
        self.timer.period = sweep_time;
        self.should_negate = negate;
        self.shift = shift;
    }

    fn trigger(&mut self, current_frequency: u16) {
        self.shadow_register = current_frequency;
        self.timer.reset();
        if self.timer.period != 0 || self.shift != 0 {
            self.enabled = true;
        }
    }

    fn reset(&mut self) {
        self.enabled = false;
        self.shadow_register = 0;
        self.shift = 0;
        self.should_negate = false;
        self.timer.period = 0;
    }
}
