use serde::{Deserialize, Serialize};
#[derive(Default, Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct FrameSequencer(u8);

impl FrameSequencer {
    /// Restart so that the next step is step 0, like when the APU is powered on.
    pub fn restart(&mut self) {
        self.0 = 7;
    }

    /// Whether the next step clocks the length counters. Enabling a length counter when it
    /// doesn't clocks it straight away (see `LengthCounter::write_nrx4`).
    pub fn next_step_clocks_length(&self) -> bool {
        (self.0 + 1).is_multiple_of(2)
    }

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
