use serde::{Deserialize, Serialize};

use crate::apu::{Timer, frame_sequencer::FrameSequencer};

use super::{LengthCounter, dac};

// The channel runs at 2MHz: these are in CPU cycles, a multiple of its ticks. They come from
// SameBoy, and blargg's dmg_sound tests 09, 10 and 12 check them.
/// Extra delay before the first byte is fetched after a trigger.
const TRIGGER_DELAY: u16 = 6;
/// How long after fetching a byte the CPU can access it while the channel plays (on the DMG).
const ACCESS_WINDOW: u16 = 2;
/// How close to fetching a byte a retrigger has to be to corrupt wave RAM (on the DMG).
const CORRUPTION_WINDOW: u16 = 2;

/// How much each NR32 volume code shifts the samples right by: muted, 100%, 50% and 25%.
const VOLUME_SHIFTS: [u8; 4] = [4, 0, 1, 2];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct WaveChannel {
    // Wave table containing 32 4-bit samples
    wav: [u8; 16],
    dac_enabled: bool,
    enabled: bool,
    length_counter: LengthCounter,
    /// Volume code from NR32: an index into `VOLUME_SHIFTS`.
    output_level: u8,
    freq: u16,
    position: u8,
    freq_timer: Timer,
    /// Cycles since the channel last fetched a byte from wave RAM (saturating).
    since_fetch: u16,
}

impl WaveChannel {
    pub(crate) fn new() -> Self {
        Self {
            wav: [0; 16],
            dac_enabled: false,
            enabled: false,
            length_counter: LengthCounter::new(256),
            output_level: 0,
            freq: 0,
            position: 0,
            freq_timer: Timer::new(4096),
            since_fetch: u16::MAX,
        }
    }

    pub(crate) fn advance(&mut self, cycles: u16) {
        let steps = self.freq_timer.advance(cycles);
        self.since_fetch = if steps > 0 {
            // The timer reloads when it fires, so this is how long ago it last did.
            self.freq_timer.period - self.freq_timer.counter
        } else {
            self.since_fetch.saturating_add(cycles)
        };
        self.position = ((self.position as u16 + steps) % 32) as u8;
    }

    pub fn tick_frame(&mut self, frame_sequencer: &FrameSequencer) {
        // we only care about the length here
        if frame_sequencer.length_triggered() && self.length_counter.tick() {
            self.enabled = false;
        }
    }

    pub(crate) fn nr30(&self) -> u8 {
        0x7F | (self.dac_enabled as u8) << 7
    }

    pub(crate) fn set_nr30(&mut self, b: u8) {
        self.dac_enabled = b & 0x80 != 0;
        if !self.dac_enabled {
            self.enabled = false;
        }
    }

    pub(crate) fn nr31(&self) -> u8 {
        // NR31 is write-only
        0xFF
    }

    pub(crate) fn set_nr31(&mut self, b: u8) {
        self.length_counter.load(256 - b as u16);
    }

    pub(crate) fn nr32(&self) -> u8 {
        0x9F | self.output_level << 5
    }

    pub(crate) fn set_nr32(&mut self, b: u8) {
        self.output_level = (b >> 5) & 0x03;
    }

    pub(crate) fn nr33(&self) -> u8 {
        0xFF
    }

    pub(crate) fn set_nr33(&mut self, b: u8) {
        self.freq = (self.freq & 0x700) | u16::from(b);
        self.update_period();
    }

    pub(crate) fn nr34(&self) -> u8 {
        self.length_counter.nrx4()
    }

    pub(crate) fn set_nr34(&mut self, b: u8, frame_sequencer: &FrameSequencer) {
        self.freq = (self.freq & 0xFF) | (u16::from(b & 0x07) << 8);
        self.update_period();

        if self.length_counter.write_nrx4(b, frame_sequencer) {
            self.enabled = false;
        }
        let trigger = b & 0x80 != 0;
        if trigger {
            // On the DMG, retriggering just as the channel fetches a byte corrupts wave RAM.
            if self.enabled && self.freq_timer.counter <= CORRUPTION_WINDOW {
                // The byte holding the next sample, i.e. sample `position + 1`
                let offset = (self.position as usize).div_ceil(2) % 16;
                if offset < 4 {
                    self.wav[0] = self.wav[offset];
                } else {
                    let start = offset & !3;
                    self.wav.copy_within(start..start + 4, 0);
                }
            }
            if self.is_dac_on() {
                self.enabled = true;
            }
            self.position = 0;
            // The first byte is fetched a little later than a whole period after triggering.
            self.freq_timer.reset();
            self.freq_timer.counter += TRIGGER_DELAY;
            self.since_fetch = u16::MAX;
        }
    }

    /// Set the frequency timer's period from NR33/NR34. Like on hardware, it only takes effect
    /// the next time the timer reloads.
    fn update_period(&mut self) {
        self.freq_timer.period = (2048 - self.freq) * 2;
    }

    /// Read wave RAM. While the channel plays, reads get the byte it's on instead, and on the DMG
    /// only right after it fetched it: otherwise, they return 0xFF.
    pub(crate) fn read_wav(&self, idx: usize) -> u8 {
        if !self.enabled {
            self.wav[idx]
        } else if self.since_fetch < ACCESS_WINDOW {
            self.wav[self.position as usize / 2]
        } else {
            0xFF
        }
    }

    /// Write wave RAM, with the same restrictions as `read_wav` while the channel plays.
    pub(crate) fn write_wav(&mut self, idx: usize, b: u8) {
        if !self.enabled {
            self.wav[idx] = b;
        } else if self.since_fetch < ACCESS_WINDOW {
            self.wav[self.position as usize / 2] = b;
        }
    }

    pub(crate) fn digital_output(&self) -> u8 {
        if !self.enabled {
            return 0;
        }

        let byte = self.wav[self.position as usize / 2];
        let value = if self.position.is_multiple_of(2) {
            // lower nibble
            byte & 0x0F
        } else {
            // upper nibble
            byte >> 4
        };

        value >> VOLUME_SHIFTS[self.output_level as usize]
    }

    pub(crate) fn output(&self) -> f32 {
        if self.is_dac_on() {
            dac(self.digital_output())
        } else {
            0.0
        }
    }

    /// Clear the registers, like powering the APU off does. Wave RAM is left alone.
    pub(crate) fn power_off(&mut self) {
        self.dac_enabled = false;
        self.enabled = false;
        self.length_counter.power_off();
        self.position = 0;
        self.output_level = 0;
        self.freq = 0;
        self.update_period();
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
    fn frequency_change_applies_without_trigger() {
        let mut channel = WaveChannel::new();
        channel.set_nr30(0x80); // DAC on
        channel.set_nr33(0x00);
        channel.set_nr34(0x87, &FrameSequencer::default()); // trigger with frequency 0x700
        assert_eq!(channel.freq_timer.period, (2048 - 0x700) * 2);

        channel.set_nr33(0x80);
        assert_eq!(channel.freq_timer.period, (2048 - 0x780) * 2);
        channel.set_nr34(0x06, &FrameSequencer::default());
        assert_eq!(channel.freq_timer.period, (2048 - 0x680) * 2);
        assert!(channel.enabled());
    }
}
