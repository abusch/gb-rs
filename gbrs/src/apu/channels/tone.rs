use bitvec::{field::BitField, order::Lsb0, view::BitView};
use log::{debug, trace};

use crate::apu::{Timer, frame_sequencer::FrameSequencer};

use super::{LengthCounter, VolumeEnvelope, dac};
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

    pub(crate) fn advance(&mut self, cycles: u16) {
        let steps = self.freq_timer.advance(cycles);
        self.wave_generator.advance(steps);
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
        if frame_sequencer.sweep_triggered()
            && let Some(ref mut sweep) = self.frequency_sweep
        {
            match sweep.tick() {
                FrequencySweepResult::NewFreq(f) => {
                    // The new frequency is written back to NRx3/NRx4
                    self.freq_lo = f as u8;
                    self.freq_hi = (f >> 8) as u8;
                    self.update_period();
                }
                FrequencySweepResult::Disable => self.enabled = false,
                FrequencySweepResult::Nop => (),
            }
        }
    }

    pub(crate) fn nrx0(&self) -> u8 {
        let mut res = 0xFF;
        let bits = res.view_bits_mut::<Lsb0>();
        if let Some(ref sweep) = self.frequency_sweep {
            // bit 7 is always set
            bits.set(7, true);
            bits[4..=6].store(sweep.period);
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
            if sweep.load(sweep_time, negate, shift) {
                self.enabled = false;
            }
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
        self.update_period();
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
        trace!("Channel {N}: setting NRx4 to {:08b}", b);
        let bits = b.view_bits::<Lsb0>();

        if bits[6] {
            self.length_counter.enable();
        } else {
            self.length_counter.disable();
        }
        self.freq_hi = bits[0..=2].load::<u8>();
        self.update_period();

        if bits[7] {
            debug!("Channel {N}: Tone channel triggered");
            // Trigger. The rest of it still happens if the DAC is off, but the channel stays off.
            self.enabled = self.is_dac_on();
            self.length_counter.trigger();
            let freq = self.frequency();
            self.freq_timer.reset();
            // Reset volume envelope
            self.volume_envelope.trigger();
            if let Some(ref mut sweep) = self.frequency_sweep
                && sweep.trigger(freq)
            {
                self.enabled = false;
            }
        }
    }

    /// The 11-bit frequency from NRx3/NRx4.
    fn frequency(&self) -> u16 {
        ((self.freq_hi as u16) << 8) | self.freq_lo as u16
    }

    /// Set the frequency timer's period from NRx3/NRx4. Like on hardware, it only takes effect
    /// the next time the timer reloads, so changing the frequency doesn't restart the waveform.
    fn update_period(&mut self) {
        self.freq_timer.period = (2048 - self.frequency()) * 4;
    }

    /// Drop the current volume to 0, as if the envelope had fully decayed.
    pub(crate) fn silence(&mut self) {
        self.volume_envelope.volume = 0;
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

    pub fn advance(&mut self, steps: u16) {
        self.step = ((self.step as u16 + steps) % 8) as u8;
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

/// Highest value of the 11-bit frequency.
const MAX_FREQUENCY: u16 = 2047;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FrequencySweepResult {
    NewFreq(u16),
    Disable,
    Nop,
}

#[derive(Debug)]
struct FrequencySweep {
    enabled: bool,
    shadow_register: u16,
    /// Sweep period from NR10, in sweep steps of the frame sequencer. 0 disables the sweep
    /// calculations, but not the timer.
    period: u8,
    should_negate: bool,
    /// Whether a frequency was calculated in negate mode since the last trigger.
    negate_used: bool,
    timer: Timer,
    shift: u8,
}

impl FrequencySweep {
    fn new() -> Self {
        Self {
            enabled: false,
            shadow_register: 0,
            period: 0,
            should_negate: false,
            negate_used: false,
            timer: Timer::new(Self::timer_period(0)),
            shift: 0,
        }
    }

    /// The sweep timer treats a period of 0 as 8.
    fn timer_period(period: u8) -> u16 {
        if period == 0 { 8 } else { period as u16 }
    }

    fn tick(&mut self) -> FrequencySweepResult {
        if !self.timer.tick() || !self.enabled || self.period == 0 {
            return FrequencySweepResult::Nop;
        }

        // The overflow check happens even if the shift is 0 and the frequency doesn't change.
        let new_freq = self.next_frequency();
        if new_freq > MAX_FREQUENCY {
            return FrequencySweepResult::Disable;
        }
        if self.shift == 0 {
            return FrequencySweepResult::Nop;
        }
        self.shadow_register = new_freq;
        // The hardware immediately runs the calculation again with the new frequency, and
        // disables the channel if that would overflow, but doesn't write it back.
        if self.next_frequency() > MAX_FREQUENCY {
            return FrequencySweepResult::Disable;
        }
        FrequencySweepResult::NewFreq(new_freq)
    }

    /// The frequency the next sweep step would switch to, possibly past `MAX_FREQUENCY`.
    fn next_frequency(&mut self) -> u16 {
        let delta = self.shadow_register >> self.shift;
        if self.should_negate {
            self.negate_used = true;
            self.shadow_register - delta
        } else {
            self.shadow_register + delta
        }
    }

    /// Handle a write to NR10. Return whether the channel should be disabled.
    fn load(&mut self, period: u8, negate: bool, shift: u8) -> bool {
        // The new period is used from the next time the timer is reloaded.
        self.period = period;
        self.timer.period = Self::timer_period(period);
        self.shift = shift;
        // Leaving negate mode after a calculation used it since the last trigger disables the
        // channel.
        let leaving_negate = self.should_negate && !negate && self.negate_used;
        self.should_negate = negate;
        leaving_negate
    }

    /// Handle the channel being triggered. Return whether the channel should be disabled.
    fn trigger(&mut self, current_frequency: u16) -> bool {
        self.shadow_register = current_frequency;
        self.timer.reset();
        self.negate_used = false;
        self.enabled = self.period != 0 || self.shift != 0;
        // With a non-zero shift, the overflow check is run straight away.
        self.shift != 0 && self.next_frequency() > MAX_FREQUENCY
    }

    fn reset(&mut self) {
        self.enabled = false;
        self.shadow_register = 0;
        self.period = 0;
        self.timer.period = Self::timer_period(0);
        self.shift = 0;
        self.should_negate = false;
        self.negate_used = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A sweep that steps on every tick with the given settings, triggered with `freq`.
    fn sweep(freq: u16, period: u8, negate: bool, shift: u8) -> FrequencySweep {
        let mut sweep = FrequencySweep::new();
        sweep.load(period, negate, shift);
        assert!(!sweep.trigger(freq), "overflow on trigger");
        sweep
    }

    #[test]
    fn frequency_change_applies_without_trigger() {
        let mut channel = ToneChannel::<2>::new(false);
        channel.set_nrx2(0xF0); // DAC on
        channel.set_nrx3(0x00);
        channel.set_nrx4(0x87); // trigger with frequency 0x700
        assert_eq!(channel.freq_timer.period, (2048 - 0x700) * 4);

        channel.set_nrx3(0x80);
        assert_eq!(channel.freq_timer.period, (2048 - 0x780) * 4);
        channel.set_nrx4(0x06);
        assert_eq!(channel.freq_timer.period, (2048 - 0x680) * 4);
        assert!(channel.enabled());
    }

    #[test]
    fn nrx4_is_written_while_dac_is_off() {
        let mut channel = ToneChannel::<2>::new(false);
        channel.set_nrx2(0x00); // DAC off
        channel.set_nrx4(0xC7); // trigger, with length enabled and frequency 0x700
        assert!(!channel.enabled());
        assert!(channel.length_counter.length_enabled);
        assert_eq!(channel.freq_timer.period, (2048 - 0x700) * 4);

        channel.set_nrx2(0xF0); // DAC on
        channel.set_nrx4(0x80);
        assert!(channel.enabled());
    }

    #[test]
    fn sweep_updates_frequency() {
        let mut up = sweep(600, 1, false, 1);
        assert_eq!(up.tick(), FrequencySweepResult::NewFreq(900));
        let mut down = sweep(1000, 1, true, 1);
        assert_eq!(down.tick(), FrequencySweepResult::NewFreq(500));
    }

    #[test]
    fn sweep_disables_on_overflow() {
        // 1200 + 600 = 1800 fits, but the second calculation (1800 + 900) overflows.
        assert_eq!(
            sweep(1200, 1, false, 1).tick(),
            FrequencySweepResult::Disable
        );
        // Going down never overflows.
        assert_eq!(
            sweep(2047, 1, true, 1).tick(),
            FrequencySweepResult::NewFreq(1024)
        );
    }

    #[test]
    fn sweep_checks_overflow_on_trigger_if_shift_is_not_zero() {
        let mut sweep = FrequencySweep::new();
        sweep.load(1, false, 1);
        assert!(sweep.trigger(1500));
        sweep.load(1, false, 0);
        assert!(!sweep.trigger(1500));
    }

    #[test]
    fn sweep_with_shift_0_checks_overflow_without_updating() {
        let mut fits = sweep(1000, 1, false, 0);
        assert_eq!(fits.tick(), FrequencySweepResult::Nop);
        assert_eq!(fits.shadow_register, 1000);
        assert_eq!(
            sweep(1500, 1, false, 0).tick(),
            FrequencySweepResult::Disable
        );
    }

    #[test]
    fn sweep_with_period_0_does_nothing() {
        let mut sweep = sweep(1000, 0, false, 1);
        assert!(sweep.enabled);
        for _ in 0..16 {
            assert_eq!(sweep.tick(), FrequencySweepResult::Nop);
        }
        assert_eq!(sweep.shadow_register, 1000);
    }

    #[test]
    fn sweep_timer_treats_period_0_as_8() {
        let mut sweep = sweep(100, 0, false, 1);
        for _ in 0..6 {
            sweep.tick();
        }
        // Setting a period only takes effect when the timer reloads, so it has 2 steps to go.
        sweep.load(1, false, 1);
        assert_eq!(sweep.tick(), FrequencySweepResult::Nop);
        assert_eq!(sweep.tick(), FrequencySweepResult::NewFreq(150));
    }

    #[test]
    fn leaving_negate_mode_after_using_it_disables_channel() {
        let mut used = sweep(1000, 1, true, 1);
        used.tick();
        assert!(used.load(1, false, 1));

        let mut unused = sweep(1000, 1, true, 0);
        assert!(!unused.load(1, false, 0));
    }
}
