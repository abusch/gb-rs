use bitflags::bitflags;

bitflags! {
    pub struct InterruptFlag: u8 {
        const VBLANK   = 0b00000001;
        const STAT = 0b00000010;
        const TIMER    = 0b00000100;
        const SERIAL   = 0b00001000;
        const JOYPAD   = 0b00010000;
    }
}
