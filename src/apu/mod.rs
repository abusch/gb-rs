use std::{time::Duration, collections::VecDeque, ops::ShrAssign};

use bitvec::{field::BitField, order::Lsb0, view::BitView};
use log::{debug, trace};

use crate::AudioSink;

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

const CPU_CYCLES_PER_SECOND: u32 = 4194304;
// Period for the main 512Hz timer
const TIMER_PERIOD: u16 = 8192;
const TARGET_SAMPLE_RATE: u32 = 44100;
// const SAMPLE_CYCLES: f32 = CPU_CYCLES_PER_SECOND as f32 / 44100.0;

#[derive(Debug)]
pub struct Apu {
    /// Main on/off switch for the whole APU. Comes from NR52 (bit 7).
    apu_enabled: bool,
    /// NR51
    sound_output_selection: u8,
    /// Left volume. Comes from NR50.
    left_volume: u8,
    /// Right volume. Comes from NR50.
    right_volume: u8,

    sample_rate: u32,
    sample_period: f32,
    sample_counter: f32,
    timer: Timer,
    frame_sequencer: FrameSequencer,

    channel1: Channel,
    channel2: Channel,
    channel3: WaveChannel,
    channel4: NoiseChannel,

    buf: VecDeque<i16>,
}

impl Apu {
    pub fn new() -> Self {
        Self {
            apu_enabled: true,
            sound_output_selection: 0,
            left_volume: 0,
            right_volume: 0,
            sample_rate: TARGET_SAMPLE_RATE,
            sample_period: CPU_CYCLES_PER_SECOND as f32 / TARGET_SAMPLE_RATE as f32,
            sample_counter: 0.0,
            timer: Timer::new(TIMER_PERIOD),
            frame_sequencer: FrameSequencer::default(),
            channel1: Channel::new(),
            channel2: Channel::new(),
            channel3: WaveChannel::new(),
            channel4: NoiseChannel::new(),
            buf: VecDeque::new(),
        }
    }

    pub fn step(&mut self, cycles: u8, sink: &mut dyn AudioSink) {
        for _ in 0..cycles {
            self.channel1.tick();
            self.channel2.tick();
            self.channel3.tick();
            self.channel4.tick();

            if self.timer.tick() {
                self.frame_sequencer.tick();
                self.channel1.tick_frame(&self.frame_sequencer);
                self.channel2.tick_frame(&self.frame_sequencer);
                self.channel3.tick_frame(&self.frame_sequencer);
                self.channel4.tick_frame(&self.frame_sequencer);
            }
            // TODO

            self.sample_counter += 1.0;
            if self.sample_counter >= self.sample_period {
                self.sample_counter -= self.sample_period;
                let (left, right) = self.output();
                if self.buf.len() < 400 {
                    self.buf.push_back(left);
                    self.buf.push_back(right);
                }

                if self.buf.len() > 200 {
                    sink.push_samples(&mut self.buf);
                }
                // if sink.push_sample(self.output()) {
                //     // Buffer is full, wait a little bit
                //     // TODO this is a pretty ugly hack....
                //     std::thread::sleep(Duration::from_millis(40));
                // }
            }
        }
    }

    // Outputs a pair of left/right samples
    fn output(&self) -> (i16, i16) {
        let mut left = 0;
        let mut right = 0;

        if !self.apu_enabled {
            return (left, right);
        }

        let nr51 = self.sound_output_selection.view_bits::<Lsb0>();

        if nr51[7] {
            left += self.channel4.output();
        }
        if nr51[6] {
            left += self.channel3.output();
        }
        if nr51[5] {
            left += self.channel2.output();
        }
        if nr51[4] {
            left += self.channel1.output();
        }
        if nr51[3] {
            right += self.channel4.output();
        }
        if nr51[2] {
            right += self.channel3.output();
        }
        if nr51[1] {
            right += self.channel2.output();
        }
        if nr51[0] {
            right += self.channel1.output();
        }

        left *= self.left_volume as i16 + 1;
        right *= self.right_volume as i16 + 1;

        (left, right)
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
            REG_NR52 => {
                let mut byte = 0u8;
                let bits = byte.view_bits_mut::<Lsb0>();
                bits.set(7, self.apu_enabled);
                // TODO
                bits.set(3, self.channel4.enabled);
                bits.set(2, self.channel3.enabled);
                bits.set(1, self.channel2.enabled);
                bits.set(0, self.channel1.enabled);
                byte
            }
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
            REG_NR30 => self.channel3.set_nr30(b),
            REG_NR31 => self.channel3.set_nr31(b),
            REG_NR32 => self.channel3.set_nr32(b),
            REG_NR33 => self.channel3.set_nr33(b),
            REG_NR34 => self.channel3.set_nr34(b),
            // Channel 4()
            REG_NR41 => self.channel4.set_nr41(b),
            REG_NR42 => self.channel4.set_nr42(b),
            REG_NR43 => self.channel4.set_nr43(b),
            REG_NR44 => self.channel4.set_nr44(b),
            // sound control
            REG_NR50 => {
                let bits = b.view_bits::<Lsb0>();
                self.left_volume = bits[4..=6].load::<u8>();
                self.right_volume = bits[0..=2].load::<u8>();
                trace!(
                    "Setting volumes left={}, right={}",
                    self.left_volume, self.right_volume
                );
            }
            REG_NR51 => {
                self.sound_output_selection = b;
                trace!(
                    "Setting output selection {:08b}",
                    self.sound_output_selection
                );
            }
            REG_NR52 => {
                self.apu_enabled = b.view_bits::<Lsb0>()[7];
                if self.apu_enabled {
                    debug!("Turning APU ON!");
                    self.channel1.reset();
                } else {
                    debug!("Turning APU OFF!");
                    self.buf.clear();
                    self.channel1.reset();
                    self.channel2.reset();
                    self.channel3.reset();
                    self.channel4.reset();
                }
                // TODO reset all registers if we disable sound
            }
            _ => panic!("Invalid sound register {:04x}", addr),
        };
    }

    pub fn write_wav(&mut self, addr: u16, value: u8) {
        let index = addr - 0xFF30;
        assert!(index <= 0x0F);
        self.channel3.wav[index as usize] = value;
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
    volume: u8,
    volume_increase: bool,
    // envelope_period: u8,
    // envelope_timer: u8,
    envelope_timer: Timer,

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
            volume: 0,
            volume_increase: false,
            // envelope_period: 0,
            // envelope_timer: 0,
            envelope_timer: Timer::new(0),
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
            self.volume_tick();
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
        trace!("setting NRx1 to {:08b}", b);
        let bits = b.view_bits::<Lsb0>();

        let duty = bits[6..=7].load::<u8>().into();
        self.wave_generator.set_duty(duty);
        trace!("duty = {:?}", duty);

        let length = bits[0..=5].load::<u8>();
        trace!("length = {}", length);
        self.length_counter = 64 - length;
    }

    pub fn set_nrx2(&mut self, b: u8) {
        trace!("setting NRx2 to {:08b}", b);
        let bits = b.view_bits::<Lsb0>();
        self.start_volume = bits[4..=7].load::<u8>();
        self.volume_increase = bits[3];
        self.envelope_timer.period = bits[0..=2].load::<u8>() as u16;
        self.envelope_timer.reset();
        trace!(
            "start_volume={}, volume_increase={}, envelope_period={}",
            self.start_volume, self.volume_increase, self.envelope_timer.period
        );
        // Not sure why the docs said to do this? This is wrong...
        // if self.envelope_timer.period == 0 {
        //     self.envelope_timer.period = 8;
        // }
        if !self.is_dac_on() {
            trace!("DAC is off, disabling channel");
            self.enabled = false;
        }
    }

    pub fn set_nrx3(&mut self, b: u8) {
        trace!("setting NRx3 to {:08b}", b);
        self.freq_lo = b;
    }

    pub fn set_nrx4(&mut self, b: u8) {
        trace!("setting NRx4 to {:08b}", b);
        let bits = b.view_bits::<Lsb0>();

        self.length_enabled = bits[6];
        self.freq_hi = bits[0..=2].load::<u8>();

        if bits[7] {
            // Trigger
            self.enabled = true;
            if self.length_counter == 0 {
                self.length_counter = 64;
            }
            let freq = ((self.freq_hi as u16) << 8) + self.freq_lo as u16;
            if freq == 0 {
                // should we do this?
                self.enabled = false;
            }
            self.freq_timer.period = (2048 - freq) * 4;
            self.freq_timer.reset();
            // Reset volume envelope
            self.volume = self.start_volume;
            // self.envelope_timer.period = self.envelope_period;
            self.envelope_timer.reset();
            trace!(
                "Triggering channel with freq={}, period={}, length={}, length_enabled={}",
                freq, self.freq_timer.period, self.length_counter, self.length_enabled
            );
            if !self.is_dac_on() {
                // If DAC is off, disable the channel
                trace!("DAC is off, disabling channel");
                self.enabled = false;
            }
            // TODO  sweep, etc..
        }
    }

    pub fn output(&self) -> i16 {
        if self.enabled && self.is_dac_on() && self.wave_generator.output() {
            self.volume as i16
        } else {
            0
        }
    }

    fn length_tick(&mut self) {
        if self.enabled && self.length_enabled {
            // Only decrement the length counter if the channel is active
            self.length_counter -= 1;
            if self.length_counter == 0 {
                // If we reach 0, disable the channel
                trace!("length expired, disabling channel");
                self.enabled = false;
            }
        }
    }

    fn volume_tick(&mut self) {
        if self.envelope_timer.tick()  {
            if self.volume_increase && self.volume < 15 {
                self.volume += 1;
                trace!("increasing volume {}", self.volume);
            } else if !self.volume_increase && self.volume > 0 {
                self.volume -= 1;
                trace!("decreasing volume {}", self.volume);
            }
            // self.envelope_timer -= 1;
        }
    }

    fn is_dac_on(&self) -> bool {
        self.start_volume != 0 || self.volume_increase
    }

    pub(crate) fn reset(&mut self) {
        trace!("Resetting square channel");
        self.enabled = false;
        self.start_volume = 0;
        self.volume = 0;
        self.length_counter = 0;
        self.length_enabled = false;
        // self.envelope_period = 0;
        // self.envelope_timer = 0;
        self.envelope_timer.period = 0;
        self.envelope_timer.reset();
        self.volume_increase = false;
        self.freq_hi = 0;
        self.freq_lo = 0;
        self.freq_timer.reset();
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
            self.reset();
            true
        } else {
            false
        }
    }

    pub fn reset(&mut self) {
        self.counter = self.period;
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

#[derive(Debug)]
struct WaveChannel {
    // Wave table containing 32 4-bit samples
    wav: [u8; 16],
    enabled: bool,
    length_enabled: bool,
    length_counter: u16,
    output_level: OutputLevel,
    freq: u16,
    position: u8,
    freq_timer: Timer,
}

impl WaveChannel {
    fn new() -> Self {
        Self {
            wav: [0; 16],
            enabled: false,
            length_enabled: false,
            length_counter: 0,
            output_level: OutputLevel::Mute,
            freq: 0,
            position: 0,
            freq_timer: Timer::new(4096),
        }
    }

    fn tick(&mut self) {
        if self.freq_timer.tick() {
            self.position += 1;
            if self.position == 32 {
                self.position = 0;
            }
        }
    }

    pub fn tick_frame(&mut self, frame_sequencer: &FrameSequencer) {
        // we only care about the length here
        if frame_sequencer.length_triggered() {
            self.length_tick();
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

    fn set_nr30(&mut self, b: u8) {
        if b.view_bits::<Lsb0>()[7] {
            self.enabled = true;
            self.position = 0;
        } else {
            self.enabled = false;
        }
    }

    fn set_nr31(&mut self, b: u8) {
        self.length_counter = 256 - b as u16;
    }

    fn set_nr32(&mut self, b: u8) {
        self.output_level = match b.view_bits::<Lsb0>()[5..=6].load::<u8>() {
            0 => OutputLevel::Mute,
            1 => OutputLevel::Full,
            2 => OutputLevel::Half,
            3 => OutputLevel::Quarter,
            _ => unreachable!(),
        };
    }

    fn set_nr33(&mut self, b: u8) {
        self.freq.view_bits_mut::<Lsb0>()[0..=7].store(b);
    }

    fn set_nr34(&mut self, b: u8) {
        let bits = b.view_bits::<Lsb0>();
        self.freq.view_bits_mut::<Lsb0>()[8..=10].store::<u8>(bits[0..=2].load::<u8>());

        if bits[7] {
            // trigger
            self.position = 0;
            self.freq_timer.period = (2048 - self.freq) * 2;
            self.length_counter = 256;
        }
    }

    fn output(&self) -> i16 {
        if !self.enabled {
            return 0;
        }

        let byte = self.wav[self.position as usize / 2];
        let value = if self.position % 2 == 0 {
            // lower nibble
            byte & 0x0F
        } else {
            // upper nibble
            byte >> 4
        };
        let adjusted_value = self.output_level.apply(value);

        adjusted_value as i16
        // dac(adjusted_value)
    }

    fn reset(&mut self) {
        self.enabled = false;
        self.length_enabled = false;
        self.length_counter = 0;
        self.position = 0;
        self.output_level = OutputLevel::Mute;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OutputLevel {
    Mute,
    Full,
    Half,
    Quarter,
}

impl OutputLevel {
    fn apply(&self, value: u8) -> u8 {
        match self {
            OutputLevel::Mute => 0,
            OutputLevel::Full => value,
            OutputLevel::Half => value >> 1,
            OutputLevel::Quarter => value >> 2,
        }
    }
}

/// Linear Feedback Shift Register
#[derive(Debug)]
struct LSFR {
    reg: u16,
    width_mode: bool,
}

impl LSFR {
    fn new() -> Self {
        Self {
            reg: 0xffff,
            width_mode: false,
        }
    }

    fn reset(&mut self) {
        self.reg = 0xffff;
        self.width_mode = false;
    }

    fn tick(&mut self) {
        let bits = self.reg.view_bits::<Lsb0>();
        let b = bits[0] ^ bits[1];
        self.reg.shr_assign(1);

        let bits = self.reg.view_bits_mut::<Lsb0>(); 
        bits.set(14, b);
        if self.width_mode {
            bits.set(6, b);
        }
    }

    fn output(&self) -> bool {
        // output is bit 0 *inverted*
        self.reg & 0x0001 == 0
    }
}

#[derive(Debug)]
struct NoiseChannel {
    enabled: bool,
    lsfr: LSFR,
    timer: Timer,
    length_enabled: bool,
    length_counter: u8,
    start_volume: u8,
    volume: u8,
    volume_increase: bool,
    envelope_timer: Timer,
}

impl NoiseChannel {
    fn new() -> Self {
        Self {
            enabled: false,
            lsfr: LSFR::new(),
            timer: Timer::new(4096),
            length_enabled: false,
            length_counter: 0,
            start_volume: 0,
            volume: 0,
            volume_increase: false,
            envelope_timer: Timer::new(0),
        }
    }

    fn tick(&mut self) {
        if self.timer.tick() {
            self.lsfr.tick();
        }
    }

    fn tick_frame(&mut self, frame_sequencer: &FrameSequencer) {
        if frame_sequencer.length_triggered() {
            self.length_tick();
        }
        if frame_sequencer.vol_envelope_trigged() {
            self.volume_tick();
        }
    }

    fn length_tick(&mut self) {
        if self.length_enabled && self.length_counter > 0 {
            self.length_counter -= 1;
            if self.length_counter == 0 {
                trace!("Length expired: disabling noise channel");
                self.enabled = false;
            }
        }
    }

    fn volume_tick(&mut self) {
        if self.envelope_timer.tick()  {
            if self.volume_increase && self.volume < 15 {
                self.volume += 1;
                trace!("increasing volume {}", self.volume);
            } else if !self.volume_increase && self.volume > 0 {
                self.volume -= 1;
                trace!("decreasing volume {}", self.volume);
            }
            // self.envelope_timer -= 1;
        }
        if !self.is_dac_on() {
            self.enabled = false;
        }
    }

    fn set_nr41(&mut self, b: u8) {
        self.length_counter = 64 - (b & 0x3F);
    }

    fn set_nr42(&mut self, b: u8) {
        let bits = b.view_bits::<Lsb0>();
        self.start_volume = bits[4..=7].load::<u8>();
        self.volume_increase = bits[3];
        // todo envelope sweep
        self.envelope_timer.period = bits[0..=2].load::<u8>() as u16;
        self.envelope_timer.reset();
        trace!(
            "start_volume={}, volume_increase={}, envelope_period={}",
            self.start_volume, self.volume_increase, self.envelope_timer.period
        );
        // if self.envelope_timer.period == 0 {
        //     self.envelope_timer.period = 8;
        // }
        if !self.is_dac_on() {
            self.enabled = false;
        }
    }

    fn set_nr43(&mut self, b: u8) {
        let bits = b.view_bits::<Lsb0>();
        let base_divisor = match bits[0..=2].load::<u8>() {
            0 => 8,
            n@1..=7 => n * 16,
            _ => unreachable!(),
        };
        let shift = bits[4..=7].load::<u8>();
        let width = bits[3];
        let period = (base_divisor as u16) << (shift as u16);
        self.timer.period = period;
        self.timer.reset();
        self.lsfr.width_mode = width;
    }

    fn set_nr44(&mut self, b: u8) {
        let bits = b.view_bits::<Lsb0>();
        self.length_enabled = bits[6];

        if bits[7] {
            // trigger
            trace!("Noise channel triggered");
            self.enabled = true;
            self.lsfr.reset();
            self.volume = self.start_volume;
            self.envelope_timer.reset();
            if self.length_counter == 0 {
                self.length_counter = 64;
            }
            if !self.is_dac_on() {
                self.enabled = false;
            }
        }
    }

    fn reset(&mut self) {
        self.enabled = false;
        self.length_counter = 0;
        self.length_enabled = false;
        self.volume = self.start_volume;
        self.volume_increase = false;
    }

    fn output(&self) -> i16 {
        if self.enabled && self.is_dac_on() && self.lsfr.output() {
            self.volume as i16
        } else {
            0
        }
    }

    fn is_dac_on(&self) -> bool {
        self.start_volume != 0 || self.volume_increase
    }

}
