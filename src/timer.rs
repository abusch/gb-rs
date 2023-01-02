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
        for _ in 0..cycles {
            if self.tima_has_overflowed {
                // When TIMA overflows, there is a 1-cycle delay before it is reloaded with TMA and
                // an interrupt is triggered
                self.tima_has_overflowed = false;
                self.tima = self.tma;
                request_interrupt = true;
            }
            self.update_div(self.div_timer.wrapping_add(1));
        }
        request_interrupt
    }

    fn update_div(&mut self, new_value: u16) {
        let old_div_timer = self.div_timer;
        // Update DIV
        self.div_timer = new_value;

        // Update TIMA
        if self.tac_timer_enable {
            // Bit number of the system clock counter to check for a falling edge
            let bit_num = match self.tac_input_clock_select {
                ClockSpeed::Speed0 => 9,
                ClockSpeed::Speed1 => 3,
                ClockSpeed::Speed2 => 5,
                ClockSpeed::Speed3 => 7,
            };
            let old_bit = old_div_timer.view_bits::<Lsb0>()[bit_num];
            let new_bit = self.div_timer.view_bits::<Lsb0>()[bit_num];

            if old_bit && !new_bit {
                // Falling edge detected: update TIMA
                let (new_tima, overflow) = self.tima.overflowing_add(1);
                if overflow {
                    self.tima_has_overflowed = true;
                    self.tima = 0;
                } else {
                    self.tima = new_tima;
                }
            }
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
