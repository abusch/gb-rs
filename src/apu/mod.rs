use bitvec::{field::BitField, order::Lsb0, view::BitView};

// Channel 1
const REG_NR10: u16 = 0xFF10;
const REG_NR11: u16 = 0xFF11;
const REG_NR12: u16 = 0xFF12;
const REG_NR13: u16 = 0xFF13;
const REG_NR14: u16 = 0xFF14;
// Channel 2
const REG_NR21: u16 = 0xFF16;
const REG_NR22: u16 = 0xFF17;
const REG_NR23: u16 = 0xFF18;
const REG_NR24: u16 = 0xFF19;
// Channel 3
const REG_NR30: u16 = 0xFF1A;
const REG_NR31: u16 = 0xFF1B;
const REG_NR32: u16 = 0xFF1C;
const REG_NR33: u16 = 0xFF1D;
const REG_NR34: u16 = 0xFF1E;
// Channel 4
const REG_NR41: u16 = 0xFF20;
const REG_NR42: u16 = 0xFF21;
const REG_NR43: u16 = 0xFF22;
const REG_NR44: u16 = 0xFF23;
// sound control
const REG_NR50: u16 = 0xFF24;
const REG_NR51: u16 = 0xFF25;
const REG_NR52: u16 = 0xFF26;

// const CPU_CYCLES_PER_SECOND: u32 = 4194304;
// Period for the main 512Hz timer
const TIMER_PERIOD: u16 = 8192;

#[derive(Debug)]
pub struct Apu {
    sample_rate: u32,
    timer: Timer,
    frame_sequencer: FrameSequencer,

    channel1: Channel,
    channel2: Channel,
}

impl Apu {
    pub fn new() -> Self {
        Self {
            sample_rate: 44100,
            timer: Timer::new(TIMER_PERIOD),
            frame_sequencer: FrameSequencer::default(),
            channel1: Channel::new(),
            channel2: Channel::new(),
        }
    }

    pub fn step(&mut self, cycles: u8) {
        for _ in 0..cycles {
            self.channel1.tick();
            if self.timer.tick() {
                self.frame_sequencer.tick();
                self.channel1.tick_frame(&self.frame_sequencer);
            }
            // TODO
        }
    }

    pub fn read_io(&self, addr: u16) -> u8 {
        match addr {
            // Channel 1
            REG_NR10 => 0,
            REG_NR11 => 0,
            REG_NR12 => 0,
            REG_NR13 => 0,
            REG_NR14 => 0,
            // Channel 2
            REG_NR21 => 0,
            REG_NR22 => 0,
            REG_NR23 => 0,
            REG_NR24 => 0,
            // Channel 3
            REG_NR30 => 0,
            REG_NR31 => 0,
            REG_NR32 => 0,
            REG_NR33 => 0,
            REG_NR34 => 0,
            // Channel 4
            REG_NR41 => 0,
            REG_NR42 => 0,
            REG_NR43 => 0,
            REG_NR44 => 0,
            // sound control
            REG_NR50 => 0,
            REG_NR51 => 0,
            REG_NR52 => 0,
            _ => panic!("Invalid sound register {:04x}", addr),
        }
    }

    pub fn write_io(&mut self, addr: u16, b: u8) {
        match addr {
            // Channel 1
            REG_NR10 => self.channel1.set_nrx0(b),
            REG_NR11 => self.channel1.set_nrx1(b),
            REG_NR12 => self.channel1.set_nrx2(b),
            REG_NR13 => self.channel1.set_nrx3(b),
            REG_NR14 => self.channel1.set_nrx4(b),
            // Channel 2()
            REG_NR21 => self.channel2.set_nrx1(b),
            REG_NR22 => self.channel2.set_nrx2(b),
            REG_NR23 => self.channel2.set_nrx3(b),
            REG_NR24 => self.channel2.set_nrx4(b),
            // Channel 3()
            REG_NR30 => (),
            REG_NR31 => (),
            REG_NR32 => (),
            REG_NR33 => (),
            REG_NR34 => (),
            // Channel 4()
            REG_NR41 => (),
            REG_NR42 => (),
            REG_NR43 => (),
            REG_NR44 => (),
            // sound control
            REG_NR50 => (),
            REG_NR51 => (),
            REG_NR52 => (),
            _ => panic!("Invalid sound register {:04x}", addr),
        };
    }
}

#[derive(Default, Debug, Clone, Copy, PartialEq, Eq)]
struct FrameSequencer(u8);

impl FrameSequencer {
    pub fn tick(&mut self) {
        self.0 = (self.0 + 1) % 8;
    }

    pub fn length_triggered(&self) -> bool {
        self.0 == 0 || self.0 == 2 || self.0 == 4 || self.0 == 6
    }

    pub fn vol_envelope_trigged(&self) -> bool {
        self.0 == 7
    }

    pub fn sweep_triggered(&self) -> bool {
        self.0 == 2 || self.0 == 6
    }
}

#[derive(Debug)]
struct Channel {
    enabled: bool,
    length_enabled: bool,
    length_counter: u8,

    start_volume: u8,

    freq_hi: u8,
    freq_lo: u8,

    freq_timer: Timer,
    wave_generator: WaveGenerator,
}

impl Channel {
    pub fn new() -> Self {
        Self {
            enabled: false,
            length_enabled: false,
            length_counter: 0,
            start_volume: 0,
            freq_hi: 0,
            freq_lo: 0,
            freq_timer: Timer::new(8192),
            wave_generator: WaveGenerator::new(),
        }
    }
    pub fn tick(&mut self) {
        if self.freq_timer.tick() {
            self.wave_generator.tick();
        }
    }

    pub fn tick_frame(&mut self, frame_sequencer: &FrameSequencer) {
        if frame_sequencer.length_triggered() {
            self.length_tick();
        }
        if frame_sequencer.vol_envelope_trigged() {
            // TODO
        }
        if frame_sequencer.sweep_triggered() {
            // TODO
        }
    }

    pub fn set_nrx0(&mut self, b: u8) {
        // let bits = b.view_bits::<Lsb0>();
        // TODO
    }

    pub fn set_nrx1(&mut self, b: u8) {
        let bits = b.view_bits::<Lsb0>();

        let duty = bits[6..=7].load::<u8>().into();
        self.wave_generator.set_duty(duty);

        let length = bits[0..=5].load::<u8>();
        self.length_counter = 64 - length;
    }

    pub fn set_nrx2(&mut self, b: u8) {
        let bits = b.view_bits::<Lsb0>();
        self.start_volume = bits[4..=7].load::<u8>();
        // TODO enveloppe add mode, period
    }

    pub fn set_nrx3(&mut self, b: u8) {
        self.freq_lo = b;
    }

    pub fn set_nrx4(&mut self, b: u8) {
        let bits = b.view_bits::<Lsb0>();

        self.length_enabled = bits[6];
        self.freq_hi = bits[0..=2].load::<u8>();

        if bits[7] {
            // Trigger
            self.enabled = true;
            if self.length_counter == 0 {
                self.length_counter = 64;
            }
            // TODO freq, sweep, etc..
        }
    }

    fn length_tick(&mut self) {
        if self.enabled && self.length_enabled {
            // Only decrement the length counter if the channel is active
            self.length_counter -= 1;
            if self.length_counter == 0 {
                // If we reach 0, disable the channel
                self.enabled = false;
            }
        }
    }
}

#[derive(Debug)]
struct Timer {
    period: u16,
    counter: u16,
}

impl Timer {
    pub fn new(period: u16) -> Self {
        Self {
            period,
            counter: period,
        }
    }

    pub fn tick(&mut self) -> bool {
        self.counter -= 1;
        if self.counter == 0 {
            self.counter = self.period;
            true
        } else {
            false
        }
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

#[derive(Debug)]
struct WaveGenerator {
    duty: Duty,
    step: u8,
}

impl WaveGenerator {
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
}
