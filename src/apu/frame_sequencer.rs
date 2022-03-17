#[derive(Default, Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FrameSequencer(u8);

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


