use log::{debug, trace};
use serde::{Deserialize, Serialize};

use crate::apu::{Timer, frame_sequencer::FrameSequencer};

use super::{LengthCounter, VolumeEnvelope, dac};

/// Channels 1 and 2: square waves, with a frequency sweep for channel 1.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct ToneChannel {
    dac_enabled: bool,
    enabled: bool,
    length_counter: LengthCounter,

    volume_envelope: VolumeEnvelope,

    freq_hi: u8,
    freq_lo: u8,

    freq_timer: Timer,
    frequency_sweep: Option<FrequencySweep>,
    wave_generator: SquareWaveGenerator,
}

impl ToneChannel {
    pub(crate) fn new(with_frequency_sweep: bool) -> Self {
        Self {
            dac_enabled: false,
            enabled: false,
            length_counter: LengthCounter::new(64),
            volume_envelope: VolumeEnvelope::new(),
            freq_hi: 0,
            freq_lo: 0,
            freq_timer: Timer::new(8192),
            frequency_sweep: with_frequency_sweep.then(FrequencySweep::new),
            wave_generator: SquareWaveGenerator::default(),
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

    /// NR10, which only channel 1 has: the sweep period, direction and shift.
    pub(crate) fn nrx0(&self) -> u8 {
        match &self.frequency_sweep {
            Some(sweep) => {
                0x80 | sweep.period << 4 | (sweep.should_negate as u8) << 3 | sweep.shift
            }
            None => 0xFF,
        }
    }

    pub(crate) fn set_nrx0(&mut self, b: u8) {
        if let Some(ref mut sweep) = self.frequency_sweep
            && sweep.load((b >> 4) & 0x07, b & 0x08 != 0, b & 0x07)
        {
            self.enabled = false;
        }
    }

    /// NRx1: the duty cycle can be read back, but not the length.
    pub(crate) fn nrx1(&self) -> u8 {
        0x3F | self.wave_generator.duty << 6
    }

    pub(crate) fn set_nrx1(&mut self, b: u8) {
        trace!("setting NRx1 to {:08b}", b);
        self.wave_generator.duty = b >> 6;
        self.set_length(b);
    }

    /// Write the length part of NRx1, which is the only one that works while the APU is off.
    pub(crate) fn set_length(&mut self, b: u8) {
        self.length_counter.load(64 - (b & 0x3F) as u16);
    }

    pub(crate) fn nrx2(&self) -> u8 {
        self.volume_envelope.nrx2()
    }

    pub(crate) fn set_nrx2(&mut self, b: u8) {
        trace!("setting NRx2 to {:08b}", b);
        self.dac_enabled = self.volume_envelope.write_nrx2(b);
        if !self.dac_enabled {
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
        self.length_counter.nrx4()
    }

    pub(crate) fn set_nrx4(&mut self, b: u8, frame_sequencer: &FrameSequencer) {
        trace!("setting NRx4 to {:08b}", b);
        self.freq_hi = b & 0x07;
        self.update_period();

        if self.length_counter.write_nrx4(b, frame_sequencer) {
            self.enabled = false;
        }
        if b & 0x80 != 0 {
            debug!("Tone channel triggered");
            // Trigger. The rest of it still happens if the DAC is off, but the channel stays off.
            self.enabled = self.is_dac_on();
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

    /// Clear the registers, like powering the APU off does.
    pub(crate) fn power_off(&mut self) {
        trace!("Resetting square channel");
        self.dac_enabled = false;
        self.enabled = false;
        self.volume_envelope.reset();
        self.length_counter.power_off();
        self.freq_hi = 0;
        self.freq_lo = 0;
        self.freq_timer.reset();
        self.wave_generator = SquareWaveGenerator::default();
        if let Some(ref mut sweep) = self.frequency_sweep {
            sweep.reset()
        }
    }

    pub(crate) fn enabled(&self) -> bool {
        self.enabled
    }
}

/// The waveform of each duty cycle, over the 8 steps of a period: bit `n` is the output at step
/// `n`.
const DUTY_WAVEFORMS: [u8; 4] = [0b1000_0000, 0b1000_0001, 0b1110_0001, 0b0111_1110];

#[derive(Debug, Default, Serialize, Deserialize)]
struct SquareWaveGenerator {
    /// Duty cycle, from NRx1: an index into `DUTY_WAVEFORMS`.
    duty: u8,
    step: u8,
}

impl SquareWaveGenerator {
    fn advance(&mut self, steps: u16) {
        self.step = ((self.step as u16 + steps) % 8) as u8;
    }

    fn output(&self) -> bool {
        DUTY_WAVEFORMS[self.duty as usize] >> self.step & 1 != 0
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

#[derive(Debug, Serialize, Deserialize)]
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
        let mut channel = ToneChannel::new(false);
        channel.set_nrx2(0xF0); // DAC on
        channel.set_nrx3(0x00);
        channel.set_nrx4(0x87, &FrameSequencer::default()); // trigger with frequency 0x700
        assert_eq!(channel.freq_timer.period, (2048 - 0x700) * 4);

        channel.set_nrx3(0x80);
        assert_eq!(channel.freq_timer.period, (2048 - 0x780) * 4);
        channel.set_nrx4(0x06, &FrameSequencer::default());
        assert_eq!(channel.freq_timer.period, (2048 - 0x680) * 4);
        assert!(channel.enabled());
    }

    #[test]
    fn nrx4_is_written_while_dac_is_off() {
        let mut channel = ToneChannel::new(false);
        channel.set_nrx2(0x00); // DAC off
        channel.set_nrx4(0xC7, &FrameSequencer::default()); // trigger, with length enabled and frequency 0x700
        assert!(!channel.enabled());
        assert!(channel.length_counter.length_enabled);
        assert_eq!(channel.freq_timer.period, (2048 - 0x700) * 4);

        channel.set_nrx2(0xF0); // DAC on
        channel.set_nrx4(0x80, &FrameSequencer::default());
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
