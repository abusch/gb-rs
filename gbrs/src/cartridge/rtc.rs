//! The real-time clock of MBC3 cartridges.
//!
//! See <https://gbdev.io/pandocs/MBC3.html>. The behaviour with out-of-range values and of the
//! sub-second counter follows what rtc3test checks.

use anyhow::{Result, bail};

/// Size of the RTC state appended to save files, in the format used by BGB and VBA-M.
pub const RTC_SAVE_SIZE: usize = 48;

// Indices of the registers, as mapped at 0x08-0x0C.
const SECONDS: usize = 0;
const MINUTES: usize = 1;
const HOURS: usize = 2;
const DAYS_LOW: usize = 3;
const DAYS_HIGH: usize = 4;

// Bits of the DH register
const DH_DAY_MSB: u8 = 0x01;
const DH_HALT: u8 = 0x40;
const DH_CARRY: u8 = 0x80;

/// The bits that exist in each register. The others always read as 0.
const MASKS: [u8; 5] = [0x3F, 0x3F, 0x1F, 0xFF, DH_CARRY | DH_HALT | DH_DAY_MSB];

/// The clock ticks every 32768 cycles of its own 32.768kHz oscillator, i.e. once a second.
const CYCLES_PER_SECOND: u32 = 4_194_304;

const SECONDS_PER_DAY: u64 = 24 * 60 * 60;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(super) struct Rtc {
    /// The counters, in register order.
    live: [u8; 5],
    /// Copy of the counters taken on the last latch, which is what reads return.
    latched: [u8; 5],
    /// CPU cycles since the seconds counter last ticked.
    sub_second_cycles: u32,
}

impl Rtc {
    /// Read register `reg` (0-4, for RAM banks 0x08-0x0C).
    pub(super) fn read(&self, reg: u8) -> u8 {
        self.latched[reg as usize]
    }

    /// Write register `reg` (0-4, for RAM banks 0x08-0x0C).
    pub(super) fn write(&mut self, reg: u8, value: u8) {
        let reg = reg as usize;
        let value = value & MASKS[reg];
        self.live[reg] = value;
        self.latched[reg] = value;
        if reg == SECONDS {
            self.sub_second_cycles = 0;
        }
    }

    pub(super) fn latch(&mut self) {
        self.latched = self.live;
    }

    /// Run the clock for the given number of CPU cycles.
    pub(super) fn step(&mut self, cycles: u8) {
        if self.halted() {
            return;
        }
        self.sub_second_cycles += cycles as u32;
        if self.sub_second_cycles >= CYCLES_PER_SECOND {
            self.sub_second_cycles -= CYCLES_PER_SECOND;
            self.tick();
        }
    }

    fn halted(&self) -> bool {
        self.live[DAYS_HIGH] & DH_HALT != 0
    }

    fn days(&self) -> u16 {
        (u16::from(self.live[DAYS_HIGH] & DH_DAY_MSB) << 8) | u16::from(self.live[DAYS_LOW])
    }

    /// Set the 9-bit day counter, setting the carry flag if it overflows.
    fn set_days(&mut self, days: u64) {
        if days >= 512 {
            self.live[DAYS_HIGH] |= DH_CARRY;
        }
        let days = days % 512;
        self.live[DAYS_LOW] = days as u8;
        self.live[DAYS_HIGH] = (self.live[DAYS_HIGH] & !DH_DAY_MSB) | (days >> 8) as u8;
    }

    /// Advance the clock by one second.
    fn tick(&mut self) {
        // A counter carries into the next one when it reaches its normal limit. One that was set
        // beyond it wraps to 0 when it runs out of bits instead, without carrying.
        let live = &mut self.live;
        for (reg, limit) in [(SECONDS, 60), (MINUTES, 60), (HOURS, 24)] {
            live[reg] = (live[reg] + 1) & MASKS[reg];
            if live[reg] != limit {
                return;
            }
            live[reg] = 0;
        }
        self.set_days(u64::from(self.days()) + 1);
    }

    /// Advance the clock by a number of seconds, e.g. the time spent switched off.
    fn advance(&mut self, mut seconds: u64) {
        if self.halted() {
            return;
        }
        // Out-of-range counters don't carry, so tick one second at a time until they've wrapped.
        // That takes at most a few hours' worth of ticks.
        let in_range =
            |live: &[u8; 5]| live[SECONDS] < 60 && live[MINUTES] < 60 && live[HOURS] < 24;
        while seconds > 0 && !in_range(&self.live) {
            self.tick();
            seconds -= 1;
        }
        if seconds == 0 {
            return;
        }

        let time_of_day = u64::from(self.live[HOURS]) * 3600
            + u64::from(self.live[MINUTES]) * 60
            + u64::from(self.live[SECONDS]);
        let total = u64::from(self.days()) * SECONDS_PER_DAY + time_of_day + seconds;
        let time_of_day = total % SECONDS_PER_DAY;
        self.live[SECONDS] = (time_of_day % 60) as u8;
        self.live[MINUTES] = (time_of_day / 60 % 60) as u8;
        self.live[HOURS] = (time_of_day / 3600) as u8;
        self.set_days(total / SECONDS_PER_DAY);
    }

    /// Serialise the clock, stamped with the given time (in seconds since the Unix epoch).
    ///
    /// The format is that of BGB and VBA-M: the 5 live registers then the 5 latched ones, each as
    /// a little-endian `u32`, followed by the timestamp as a little-endian `u64`.
    pub(super) fn save(&self, unix_time: u64) -> [u8; RTC_SAVE_SIZE] {
        let mut data = [0; RTC_SAVE_SIZE];
        let regs = self.live.iter().chain(&self.latched);
        for (chunk, reg) in data.as_chunks_mut::<4>().0.iter_mut().zip(regs) {
            *chunk = u32::from(*reg).to_le_bytes();
        }
        data[40..].copy_from_slice(&unix_time.to_le_bytes());
        data
    }

    /// Restore the clock from [`Rtc::save`]'s format, then advance it by the time elapsed between
    /// the save's timestamp and `unix_time`.
    pub(super) fn load(&mut self, data: &[u8], unix_time: u64) -> Result<()> {
        let timestamp = match data.len() {
            RTC_SAVE_SIZE => u64::from_le_bytes(data[40..48].try_into().unwrap()),
            // Some emulators store a 32-bit timestamp
            44 => u64::from(u32::from_le_bytes(data[40..44].try_into().unwrap())),
            len => bail!("RTC data should be {RTC_SAVE_SIZE} or 44 bytes, but got {len} bytes"),
        };
        for reg in 0..5 {
            self.live[reg] = data[reg * 4] & MASKS[reg];
            self.latched[reg] = data[(reg + 5) * 4] & MASKS[reg];
        }
        self.sub_second_cycles = 0;
        self.advance(unix_time.saturating_sub(timestamp));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rtc(seconds: u8, minutes: u8, hours: u8, days: u16) -> Rtc {
        let mut rtc = Rtc::default();
        rtc.write(SECONDS as u8, seconds);
        rtc.write(MINUTES as u8, minutes);
        rtc.write(HOURS as u8, hours);
        rtc.write(DAYS_LOW as u8, days as u8);
        rtc.write(DAYS_HIGH as u8, (days >> 8) as u8);
        rtc
    }

    #[test]
    fn test_ticks_once_per_second() {
        let mut rtc = Rtc::default();
        for _ in 0..CYCLES_PER_SECOND / 4 - 1 {
            rtc.step(4);
        }
        assert_eq!(rtc.live[SECONDS], 0);
        rtc.step(4);
        assert_eq!(rtc.live[SECONDS], 1);
    }

    #[test]
    fn test_halt_stops_the_clock() {
        let mut rtc = Rtc::default();
        rtc.write(DAYS_HIGH as u8, DH_HALT);
        for _ in 0..CYCLES_PER_SECOND / 4 {
            rtc.step(4);
        }
        assert_eq!(rtc.live[SECONDS], 0);
    }

    #[test]
    fn test_rollovers() {
        let mut clock = rtc(59, 59, 23, 255);
        clock.tick();
        assert_eq!(clock.live, rtc(0, 0, 0, 256).live);

        let mut clock = rtc(59, 59, 23, 511);
        clock.tick();
        assert_eq!(clock.live, [0, 0, 0, 0, DH_CARRY]);
        // The carry flag is sticky
        clock.set_days(511);
        clock.write(HOURS as u8, 23);
        clock.write(MINUTES as u8, 59);
        clock.write(SECONDS as u8, 59);
        clock.tick();
        assert_eq!(clock.live, [0, 0, 0, 0, DH_CARRY]);
    }

    #[test]
    fn test_out_of_range_values_wrap_without_carrying() {
        let mut clock = rtc(60, 63, 28, 3);
        clock.tick();
        assert_eq!(clock.live, rtc(61, 63, 28, 3).live);

        for (from, to) in [
            (rtc(63, 10, 10, 3), rtc(0, 10, 10, 3)),
            (rtc(59, 63, 10, 3), rtc(0, 0, 10, 3)),
            (rtc(59, 59, 31, 3), rtc(0, 0, 0, 3)),
            (rtc(59, 61, 10, 3), rtc(0, 62, 10, 3)),
            (rtc(59, 59, 25, 3), rtc(0, 0, 26, 3)),
        ] {
            let mut clock = from.clone();
            clock.tick();
            assert_eq!(clock.live, to.live, "from {:?}", from.live);
        }
    }

    #[test]
    fn test_advance_matches_ticking() {
        for (start, seconds) in [
            (rtc(0, 0, 0, 0), 1),
            (rtc(12, 34, 5, 67), 1_000_000),
            (rtc(59, 59, 23, 511), 1),
            (rtc(30, 30, 12, 500), 20 * SECONDS_PER_DAY),
            (rtc(62, 63, 30, 10), 100_000),
        ] {
            let mut ticked = start.clone();
            for _ in 0..seconds {
                ticked.tick();
            }
            let mut advanced = start.clone();
            advanced.advance(seconds);
            assert_eq!(advanced.live, ticked.live, "from {:?}", start.live);
        }
    }

    #[test]
    fn test_save_and_load() {
        let mut saved = rtc(1, 2, 3, 300);
        saved.latch();
        saved.live[SECONDS] = 10;

        let mut loaded = Rtc::default();
        loaded.load(&saved.save(1000), 1000).unwrap();
        assert_eq!(loaded, saved);

        // Time keeps passing while switched off
        loaded.load(&saved.save(1000), 1000 + 3600).unwrap();
        assert_eq!(loaded.live, rtc(10, 2, 4, 300).live);
        assert_eq!(loaded.latched, saved.latched);

        // Except if the clock is halted
        saved.write(DAYS_HIGH as u8, DH_HALT);
        loaded.load(&saved.save(1000), 1000 + 3600).unwrap();
        assert_eq!(loaded.live, saved.live);
    }
}
