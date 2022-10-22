use std::collections::VecDeque;

use bitvec::{field::BitField, order::Lsb0, view::BitView};
use log::debug;

use crate::AudioSink;

mod channels;
mod frame_sequencer;

use frame_sequencer::FrameSequencer;

use self::channels::{NoiseChannel, ToneChannel, WaveChannel};

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

const WAV_RAM_START: u16 = 0xFF30;

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
    /// Enable Vin into left output (comes from NR50)
    left_vin_enabled: bool,
    /// Enable Vin into right output (comes from NR50)
    right_vin_enabled: bool,

    #[allow(dead_code)]
    sample_rate: u32,
    sample_period: f32,
    sample_counter: f32,
    timer: Timer,
    frame_sequencer: FrameSequencer,

    channel1: ToneChannel,
    channel2: ToneChannel,
    channel3: WaveChannel,
    channel4: NoiseChannel,

    buf: VecDeque<i16>,
}

impl Apu {
    pub fn new() -> Self {
        Self {
            apu_enabled: true,
            sound_output_selection: 0,
            left_vin_enabled: false,
            left_volume: 0,
            right_vin_enabled: false,
            right_volume: 0,
            sample_rate: TARGET_SAMPLE_RATE,
            sample_period: CPU_CYCLES_PER_SECOND as f32 / TARGET_SAMPLE_RATE as f32,
            sample_counter: 0.0,
            timer: Timer::new(TIMER_PERIOD),
            frame_sequencer: FrameSequencer::default(),
            channel1: ToneChannel::new(true),
            channel2: ToneChannel::new(false),
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
            REG_NR10 => self.channel1.nrx0(),
            REG_NR11 => self.channel1.nrx1(),
            REG_NR12 => self.channel1.nrx2(),
            REG_NR13 => self.channel1.nrx3(),
            REG_NR14 => self.channel1.nrx4(),
            0xFF15 => 0xFF, // NR15/NR20 doesn't really exist
            // Channel 2
            REG_NR21 => self.channel2.nrx1(),
            REG_NR22 => self.channel2.nrx2(),
            REG_NR23 => self.channel2.nrx3(),
            REG_NR24 => self.channel2.nrx4(),
            // Channel 3
            REG_NR30 => self.channel3.nr30(),
            REG_NR31 => self.channel3.nr31(),
            REG_NR32 => self.channel3.nr32(),
            REG_NR33 => self.channel3.nr33(),
            REG_NR34 => self.channel3.nr34(),
            0xFF1F => 0xFF,
            // Channel 4
            REG_NR41 => self.channel4.nr41(),
            REG_NR42 => self.channel4.nr42(),
            REG_NR43 => self.channel4.nr43(),
            REG_NR44 => self.channel4.nr44(),
            // sound control
            REG_NR50 => {
                let mut res = 0xFF;
                let bits = res.view_bits_mut::<Lsb0>();
                bits.set(7, self.left_vin_enabled);
                bits[4..=6].store::<u8>(self.left_volume);
                bits.set(3, self.right_vin_enabled);
                bits[0..=2].store::<u8>(self.right_volume);
                res
            }
            REG_NR51 => self.sound_output_selection,
            REG_NR52 => {
                let mut byte = 0xff;
                let bits = byte.view_bits_mut::<Lsb0>();
                bits.set(7, self.apu_enabled);
                bits.set(3, self.channel4.enabled());
                bits.set(2, self.channel3.enabled());
                bits.set(1, self.channel2.enabled());
                bits.set(0, self.channel1.enabled());
                byte
            }
            _ => panic!("Invalid sound register {:04x}", addr),
        }
    }

    pub fn write_io(&mut self, addr: u16, b: u8) {
        // If the APU is disabled, all writes are ignored, except for NR52
        if addr != REG_NR52 && !self.apu_enabled {
            return
        }

        match addr {
            // Channel 1
            REG_NR10 => self.channel1.set_nrx0(b),
            REG_NR11 => self.channel1.set_nrx1(b),
            REG_NR12 => self.channel1.set_nrx2(b),
            REG_NR13 => self.channel1.set_nrx3(b),
            REG_NR14 => self.channel1.set_nrx4(b),
            0xFF15 => (), // nop
            // Channel 2
            REG_NR21 => self.channel2.set_nrx1(b),
            REG_NR22 => self.channel2.set_nrx2(b),
            REG_NR23 => self.channel2.set_nrx3(b),
            REG_NR24 => self.channel2.set_nrx4(b),
            // Channel 3
            REG_NR30 => self.channel3.set_nr30(b),
            REG_NR31 => self.channel3.set_nr31(b),
            REG_NR32 => self.channel3.set_nr32(b),
            REG_NR33 => self.channel3.set_nr33(b),
            REG_NR34 => self.channel3.set_nr34(b),
            0xFF1F => (), // nop
            // Channel 4
            REG_NR41 => self.channel4.set_nr41(b),
            REG_NR42 => self.channel4.set_nr42(b),
            REG_NR43 => self.channel4.set_nr43(b),
            REG_NR44 => self.channel4.set_nr44(b),
            // sound control
            REG_NR50 => {
                let bits = b.view_bits::<Lsb0>();
                self.left_vin_enabled = bits[7];
                self.left_volume = bits[4..=6].load::<u8>();
                self.right_vin_enabled = bits[3];
                self.right_volume = bits[0..=2].load::<u8>();
            }
            REG_NR51 => self.sound_output_selection = b,
            REG_NR52 => {
                self.apu_enabled = b.view_bits::<Lsb0>()[7];
                if self.apu_enabled {
                    debug!("Turning APU ON!");
                    self.channel1.reset();
                } else {
                    debug!("Turning APU OFF!");
                    self.buf.clear();
                    self.left_vin_enabled = false;
                    self.right_vin_enabled = false;
                    self.timer.reset();
                    self.left_volume = 0;
                    self.right_volume = 0;
                    self.sound_output_selection = 0;
                    self.channel1.reset();
                    self.channel2.reset();
                    self.channel3.reset();
                    self.channel4.reset();
                }
            }
            _ => panic!("Invalid sound register {:04x}", addr),
        };
    }

    pub fn read_wav(&self, addr: u16) -> u8 {
        let index = addr - WAV_RAM_START;
        assert!(index <= 0x0F);
        self.channel3.read_wav(index as usize)
    }

    pub fn write_wav(&mut self, addr: u16, value: u8) {
        let index = addr - WAV_RAM_START;
        assert!(index <= 0x0F);
        self.channel3.write_wav(index as usize, value);
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
        if self.counter > 0 {
            self.counter -= 1;
        }
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
