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
}

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
        }
    }

    /// Run the timer for one M-cycle. Return whether to request the timer interrupt.
    pub fn cycle(&mut self) -> bool {
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
        // The system counter bit for each speed: TIMA ticks every 1024, 16, 64 or 256 cycles.
        const CLOCK_BITS: [u8; 4] = [9, 3, 5, 7];
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
        self.div_timer = value;
    }

    pub fn reset_div_timer(&mut self) {
        self.update_div(0);
    }

    /// Get the timer's tima.
    pub fn tima(&self) -> u8 {
        self.tima
    }

    /// Write TIMA. Writing it just after it overflowed cancels the reload from TMA, but writing it
    /// while it's being reloaded has no effect.
    pub fn set_tima(&mut self, tima: u8) {
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
        trace!("Writing {:02x} to TMA", tma);
        self.tma = tma;
        if self.reload == Reload::Done {
            self.tima = tma;
        }
    }
}
