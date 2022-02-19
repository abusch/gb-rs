use bitvec::{view::BitView, order::Lsb0, field::BitField};

pub struct Timer {
    /// FF04 - DIV - Divider Register
    /// This register is incremented at a rate of 16384Hz (~16779Hz on SGB). In other words, it is
    /// incremented every 256 cycles.
    div_timer: u8,
    div_timer_ticker: u16,
    /// FF05 - TIMA - Time counter
    tima: u8,
    /// FF06 - TMA - Time Modulo
    tma: u8,
    /// FF07 - TAC - Timer Control
    tac_timer_enable: bool,
    tac_input_clock_select: ClockSpeed,

    cycle_counter: u16,
}

impl Timer {
    pub fn new() -> Self {
        Self {
            div_timer: 0,
            div_timer_ticker: 0,
            tima: 0,
            tma: 0,
            tac_timer_enable: false,
            tac_input_clock_select: ClockSpeed::Speed0,
            cycle_counter: 0,
        }
    }

    pub fn cycle(&mut self, cycles: u8) -> bool {
        let mut request_interrupt = false;
        for _ in 0..cycles {
            self.cycle_counter = self.cycle_counter.wrapping_add(1);
            // Update DIV
            self.div_timer_ticker += 1;
            if self.div_timer_ticker > 255 {
                self.div_timer = self.div_timer.wrapping_add(1);
                self.div_timer_ticker = 0;
                request_interrupt = true;
            }
        }
        request_interrupt
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
        let mut tac = 0;
        let bits = tac.view_bits_mut::<Lsb0>();
        bits.set(2, self.tac_timer_enable);
        bits[0..2].store(self.tac_input_clock_select as u8);

        tac
    }

    pub fn div_timer(&self) -> u8 {
        self.div_timer
    }

    pub fn reset_div_timer(&mut self) {
        self.div_timer = 0;
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
