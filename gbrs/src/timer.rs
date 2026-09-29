use log::trace;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub struct Timer {
    /// FF04 - DIV - Divider Register
    /// This register is incremented at a rate of 16384Hz (~16779Hz on SGB). In other words, it is
    /// incremented every 256 cycles.
    /// This is implemented as the high byte of a 16-bit counter.
    div_timer: u16,
    /// FF05 - TIMA - Time counter
    tima: u8,
    /// FF06 - TMA - Time Modulo
    tma: u8,
    /// FF07 - TAC - Timer Control: bit 2 enables the timer, bits 0-1 select its speed
    tac: u8,

    reload: Reload,

    /// Clock cycles that have gone by since the timer last ran. `step` lets them pile up until
    /// something can happen, and `sync` catches up before anything reads or writes the registers.
    lag: u32,
    /// How far the timer can lag behind before something happens: TIMA overflowing, a step of its
    /// reload, or picking up a register write.
    idle: u32,
}

/// The system counter bit for each speed: TIMA ticks every 1024, 16, 64 or 256 cycles.
const CLOCK_BITS: [u8; 4] = [9, 3, 5, 7];
/// The longest the timer lags behind while it's stopped, so that `lag` can't overflow.
const MAX_IDLE: u32 = 1 << 16;

/// Where TIMA is in reloading from TMA after overflowing. Each state lasts one M-cycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Reload {
    None,
    /// TIMA just overflowed, and reads 0. Writing it cancels the reload and the interrupt.
    Pending,
    /// TIMA was just reloaded with TMA and the interrupt requested. Writes to TIMA are ignored,
    /// and writes to TMA go to TIMA too.
    Done,
}

impl Timer {
    pub fn new() -> Self {
        Self {
            div_timer: 0,
            tima: 0,
            tma: 0,
            tac: 0,
            reload: Reload::None,
            lag: 0,
            idle: 0,
        }
    }

    /// Let `cycles` clock cycles go by. Return whether to request the timer interrupt. The timer
    /// only runs once something can happen, so most calls just count the cycles.
    #[inline(always)]
    pub fn step(&mut self, cycles: u32) -> bool {
        self.lag += cycles;
        if self.lag < self.idle {
            return false;
        }
        self.catch_up()
    }

    /// Bring the timer up to date before its registers get read or written. Nothing can have
    /// happened in the cycles it lags behind by (or `step` would have run them), so there's no
    /// interrupt to request.
    pub fn sync(&mut self) {
        // Nothing to do, and a register write may have just asked for the next cycle to run.
        if self.lag == 0 {
            return;
        }
        debug_assert!(self.lag < self.idle);
        let interrupt = self.catch_up();
        debug_assert!(!interrupt);
    }

    fn catch_up(&mut self) -> bool {
        let mut interrupt = false;
        while self.lag > 0 {
            if self.reload == Reload::None {
                // Until TIMA overflows, DIV counting and TIMA counting its falling edges is all
                // that happens.
                let cycles = self.lag.min(self.cycles_to_overflow());
                self.advance(cycles);
                self.lag -= cycles;
            } else {
                interrupt |= self.cycle();
                self.lag = self.lag.saturating_sub(4);
            }
        }
        self.idle = if self.reload == Reload::None {
            self.cycles_to_overflow().min(MAX_IDLE)
        } else {
            4
        };
        interrupt
    }

    /// Clock cycles until TIMA overflows, if nothing gets written in the meantime.
    fn cycles_to_overflow(&self) -> u32 {
        let Some(period) = self.tima_period() else {
            return u32::MAX;
        };
        let to_first_increment = period - u32::from(self.div_timer) % period;
        to_first_increment + (255 - u32::from(self.tima)) * period
    }

    /// Clock cycles between TIMA increments, or `None` if the timer is stopped.
    fn tima_period(&self) -> Option<u32> {
        let enabled = self.tac & 0b100 != 0;
        enabled.then(|| 2 << CLOCK_BITS[usize::from(self.tac & 0b11)])
    }

    /// Run the timer for `cycles` clock cycles in one go, which mustn't take TIMA past its overflow.
    fn advance(&mut self, cycles: u32) {
        let old = u32::from(self.div_timer);
        let new = old + cycles;
        // The counter wraps around at a multiple of every period, so this also counts the
        // increments across the wrap.
        self.div_timer = new as u16;
        if let Some(period) = self.tima_period() {
            let tima = u32::from(self.tima) + new / period - old / period;
            debug_assert!(tima <= 0x100);
            if tima == 0x100 {
                // TIMA reads 0 for an M-cycle before being reloaded
                self.tima = 0;
                self.reload = Reload::Pending;
            } else {
                self.tima = tima as u8;
            }
        }
    }

    /// Run the timer for one M-cycle. Return whether to request the timer interrupt.
    fn cycle(&mut self) -> bool {
        let mut request_interrupt = false;
        self.reload = match self.reload {
            Reload::Pending => {
                self.tima = self.tma;
                request_interrupt = true;
                Reload::Done
            }
            Reload::None | Reload::Done => Reload::None,
        };
        // TIMA's clock ticks at least every 16 cycles, so this can't skip a falling edge.
        self.update_div(self.div_timer.wrapping_add(4));
        request_interrupt
    }

    /// The signal whose falling edges increment TIMA: the timer being enabled, and the system
    /// counter bit selected by TAC.
    fn tima_clock(&self) -> bool {
        let enabled = self.tac & 0b100 != 0;
        enabled && self.div_timer & (1 << CLOCK_BITS[usize::from(self.tac & 0b11)]) != 0
    }

    fn increment_tima(&mut self) {
        let (new_tima, overflow) = self.tima.overflowing_add(1);
        self.tima = new_tima;
        if overflow {
            // TIMA reads 0 for an M-cycle before being reloaded
            self.reload = Reload::Pending;
        }
    }

    fn update_div(&mut self, new_value: u16) {
        let old_clock = self.tima_clock();
        self.div_timer = new_value;
        // TIMA is incremented on a falling edge of the selected bit of the system counter
        if old_clock && !self.tima_clock() {
            self.increment_tima();
        }
    }

    pub fn set_tac(&mut self, tac: u8) {
        self.sync();
        self.idle = 0;
        // Changing TAC can make the signal clocking TIMA fall too, which increments it (on DMG)
        let old_clock = self.tima_clock();
        self.tac = tac & 0b111;
        if old_clock && !self.tima_clock() {
            self.increment_tima();
        }
    }

    pub fn tac(&self) -> u8 {
        // Unused bits read as 1
        0xF8 | self.tac
    }

    pub fn div_timer(&self) -> u8 {
        (self.div_timer >> 8) as u8
    }

    /// Set the full 16-bit counter behind DIV, e.g. to where the boot ROM would have left it.
    pub fn set_div_counter(&mut self, value: u16) {
        self.sync();
        self.idle = 0;
        self.div_timer = value;
    }

    pub fn reset_div_timer(&mut self) {
        self.sync();
        self.idle = 0;
        self.update_div(0);
    }

    /// Get the timer's tima.
    pub fn tima(&self) -> u8 {
        self.tima
    }

    /// Write TIMA. Writing it just after it overflowed cancels the reload from TMA, but writing it
    /// while it's being reloaded has no effect.
    pub fn set_tima(&mut self, tima: u8) {
        self.sync();
        self.idle = 0;
        match self.reload {
            Reload::Pending => {
                self.reload = Reload::None;
                self.tima = tima;
            }
            Reload::Done => (),
            Reload::None => self.tima = tima,
        }
    }

    /// Get the timer's tma.
    pub fn tma(&self) -> u8 {
        self.tma
    }

    /// Write TMA. While TIMA is being reloaded from it, the new value goes to TIMA too.
    pub fn set_tma(&mut self, tma: u8) {
        self.sync();
        self.idle = 0;
        trace!("Writing {:02x} to TMA", tma);
        self.tma = tma;
        if self.reload == Reload::Done {
            self.tima = tma;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Running lazily must match running every M-cycle, whatever gets written when.
    #[test]
    fn test_lazy_matches_every_cycle() {
        // xorshift, so the test is repeatable
        let mut seed = 0x2545_f491_u32;
        let mut random = move || {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            seed
        };

        let mut lazy = Timer::new();
        let mut reference = Timer::new();
        let mut interrupts = 0;
        for cycle in 0..2_000_000 {
            // CPU accesses come before the rest of the M-cycle.
            let r = random();
            if r % 64 == 0 {
                // Mostly fast speeds and TIMA close to overflowing, so that the reload comes up often.
                let b = (r >> 8) as u8;
                match (r >> 16) % 5 {
                    0 => [&mut lazy, &mut reference].map(|t| t.reset_div_timer()),
                    1 => [&mut lazy, &mut reference].map(|t| t.set_tima(b | 0xF0)),
                    2 => [&mut lazy, &mut reference].map(|t| t.set_tma(b)),
                    3 => [&mut lazy, &mut reference].map(|t| t.set_tac(0b101 | b & 0b10)),
                    _ => [&mut lazy, &mut reference].map(|t| t.set_tac(b)),
                };
            } else if r % 7 == 0 {
                lazy.sync();
                assert_eq!(
                    (lazy.div_timer, lazy.tima, lazy.reload),
                    (reference.div_timer, reference.tima, reference.reload),
                    "cycle {cycle}"
                );
            }
            let interrupt = reference.cycle();
            assert_eq!(lazy.step(4), interrupt, "cycle {cycle}");
            interrupts += u32::from(interrupt);
        }
        assert!(interrupts > 1_000, "only {interrupts} interrupts");
    }
}
