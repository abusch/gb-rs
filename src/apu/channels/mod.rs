mod tone;
mod wave;
mod noise;

pub(crate) use tone::ToneChannel;
pub(crate) use wave::WaveChannel;
pub(crate) use noise::NoiseChannel;

#[derive(Debug)]
struct LengthCounter {
    length_enabled: bool,
    length_counter: u16,
    default_length: u16,
}

impl LengthCounter {
    pub fn new(default_length: u16) -> Self {
        Self {
            length_enabled: false,
            length_counter: 0,
            default_length,
        }
    }

    fn tick(&mut self) -> bool {
        if self.length_enabled && self.length_counter > 0 {
            self.length_counter -= 1;
        }

        // Return true if the length counter is enabled and the counter has reached 0
        self.length_enabled && self.length_counter == 0
    }

    fn load(&mut self, length: u16) {
        self.length_counter = length;
    }

    fn enable(&mut self) {
        self.length_enabled = true;
    }

    fn reset(&mut self) {
        self.length_enabled = false;
        self.length_counter = 0;
    }

    fn trigger(&mut self) {
        if self.length_counter == 0 {
            self.length_counter = self.default_length;
        }
    }
}
