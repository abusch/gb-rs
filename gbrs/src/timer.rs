use bitvec::{field::BitField, order::Lsb0, view::BitView};
use log::trace;

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
    /// FF07 - TAC - Timer Control
    tac_timer_enable: bool,
    tac_input_clock_select: ClockSpeed,

    tima_has_overflowed: bool,
}

impl Timer {
    pub fn new() -> Self {
        Self {
            div_timer: 0,
            tima: 0,
            tma: 0,
            tac_timer_enable: false,
            tac_input_clock_select: ClockSpeed::Speed0,
            tima_has_overflowed: false,
        }
    }

    pub fn cycle(&mut self, cycles: u8) -> bool {
        let mut request_interrupt = false;
        let mut remaining = cycles as u16;
        while remaining > 0 {
            if self.tima_has_overflowed {
                // When TIMA overflows, there is a 1-cycle delay before it is reloaded with TMA and
                // an interrupt is triggered
                self.tima_has_overflowed = false;
                self.tima = self.tma;
                request_interrupt = true;
            }
            // Nothing else happens until the next falling edge of the selected bit, so jump to it
            // (or as far as we can go).
            let period = self.tima_period();
            let to_edge = period - (self.div_timer & (period - 1));
            let n = to_edge.min(remaining);
            self.div_timer = self.div_timer.wrapping_add(n);
            remaining -= n;
            if n == to_edge && self.tac_timer_enable {
                self.increment_tima();
            }
        }
        request_interrupt
    }

    /// Number of cycles between two increments of TIMA, i.e. between two falling edges of the
    /// system counter bit selected by TAC.
    fn tima_period(&self) -> u16 {
        match self.tac_input_clock_select {
            ClockSpeed::Speed0 => 1 << 10,
            ClockSpeed::Speed1 => 1 << 4,
            ClockSpeed::Speed2 => 1 << 6,
            ClockSpeed::Speed3 => 1 << 8,
        }
    }

    fn increment_tima(&mut self) {
        let (new_tima, overflow) = self.tima.overflowing_add(1);
        self.tima = new_tima;
        if overflow {
            self.tima_has_overflowed = true;
        }
    }

    fn update_div(&mut self, new_value: u16) {
        let old_div_timer = self.div_timer;
        // Update DIV
        self.div_timer = new_value;

        // TIMA is incremented on a falling edge of the selected bit of the system counter
        let bit = self.tima_period() >> 1;
        if self.tac_timer_enable && old_div_timer & bit != 0 && new_value & bit == 0 {
            self.increment_tima();
        }
    }

    pub fn set_tac(&mut self, tac: u8) {
        let bits = tac.view_bits::<Lsb0>();
        self.tac_timer_enable = bits[2];
        self.tac_input_clock_select = match bits[0..2].load::<u8>() {
            0 => ClockSpeed::Speed0,
            1 => ClockSpeed::Speed1,
            2 => ClockSpeed::Speed2,
            3 => ClockSpeed::Speed3,
            _ => unreachable!(),
        };
    }

    pub fn tac(&self) -> u8 {
        let mut tac = 0xFF; // unused are set to 1
        let bits = tac.view_bits_mut::<Lsb0>();
        bits.set(2, self.tac_timer_enable);
        bits[0..2].store(self.tac_input_clock_select as u8);

        tac
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

    /// Set the timer's tima.
    pub fn set_tima(&mut self, tima: u8) {
        self.tima = tima;
    }

    /// Get the timer's tma.
    pub fn tma(&self) -> u8 {
        self.tma
    }

    /// Set the timer's tma.
    pub fn set_tma(&mut self, tma: u8) {
        trace!("Writing {:02x} to TMA", tma);
        self.tma = tma;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
enum ClockSpeed {
    Speed0 = 0,
    Speed1 = 1,
    Speed2 = 2,
    Speed3 = 3,
}
