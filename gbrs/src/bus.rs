use std::ops::RangeInclusive;

use anyhow::Result;
use log::{info, trace};
use serde::{Deserialize, Serialize};

use crate::{
    AudioSink, FrameSink, apu::Apu, cartridge::Cartridge, gfx::Gfx, interrupt::InterruptFlag,
    joypad::Joypad, timer::Timer,
};

pub const BOOT_ROM_SIZE: usize = 0x100;

/// Image of the DMG boot ROM, which is mapped over the start of the cartridge until it disables
/// itself.
// Serialised as a `Vec`, since serde doesn't support arrays that large.
#[derive(Clone, Serialize, Deserialize)]
#[serde(try_from = "Vec<u8>", into = "Vec<u8>")]
pub struct BootRom(Box<[u8; BOOT_ROM_SIZE]>);

impl BootRom {
    pub fn load_bytes(content: Vec<u8>) -> Result<Self> {
        let len = content.len();
        let data = content.into_boxed_slice().try_into().map_err(|_| {
            anyhow::anyhow!("Boot ROM should be {BOOT_ROM_SIZE} bytes, but got {len} bytes")
        })?;
        Ok(Self(data))
    }
}

impl TryFrom<Vec<u8>> for BootRom {
    type Error = anyhow::Error;

    fn try_from(content: Vec<u8>) -> Result<Self> {
        Self::load_bytes(content)
    }
}

impl From<BootRom> for Vec<u8> {
    fn from(boot_rom: BootRom) -> Self {
        boot_rom.0.to_vec()
    }
}

// Memory Map
const BOOT_ROM: RangeInclusive<u16> = 0x0000..=0x00FF;
const CART_BANK_00: RangeInclusive<u16> = 0x0000..=0x3FFF;
const CART_BANK_MAPPED: RangeInclusive<u16> = 0x4000..=0x7FFF;
const VRAM: RangeInclusive<u16> = 0x8000..=0x9FFF;
const EXT_RAM: RangeInclusive<u16> = 0xA000..=0xBFFF;
const WRAM: RangeInclusive<u16> = 0xC000..=0xDFFF;
const ECHO_RAM: RangeInclusive<u16> = 0xE000..=0xFDFF;
const OAM: RangeInclusive<u16> = 0xFE00..=0xFE9F;
const INVALID_AREA: RangeInclusive<u16> = 0xFEA0..=0xFEFF;
const IO_REGISTERS: RangeInclusive<u16> = 0xFF00..=0xFF7F;
const HRAM: RangeInclusive<u16> = 0xFF80..=0xFFFE;

//
// IO registers ranges (TODO CGB registers)
//
/// Joypad controller
const IO_RANGE_JPD: RangeInclusive<u16> = 0xFF00..=0xFF00;
/// Communication
const IO_RANGE_COM: RangeInclusive<u16> = 0xFF01..=0xFF02;
/// Divider and Timer
const IO_RANGE_TIM: RangeInclusive<u16> = 0xFF04..=0xFF07;
/// IF - Interrupt Flag
const IO_RANGE_INT: RangeInclusive<u16> = 0xFF0F..=0xFF0F;
/// Sound (APU)
const IO_RANGE_APU: RangeInclusive<u16> = 0xFF10..=0xFF26;
/// Waveform RAM
const IO_RANGE_WAV: RangeInclusive<u16> = 0xFF30..=0xFF3F;
/// LCD
const IO_RANGE_LCD: RangeInclusive<u16> = 0xFF40..=0xFF4F;
/// Disable Boot ROM
const IO_RANGE_DBR: RangeInclusive<u16> = 0xFF50..=0xFF50;
/// OAM DMA source address & start
const DMA_REG: u16 = 0xFF46;

/// M-cycles between writing to DMA_REG and the first byte being copied.
const DMA_START_DELAY: u8 = 1;
const DMA_LENGTH: u8 = 0xA0;

/// OAM DMA: copies 160 bytes to OAM, one per M-cycle.
///
/// While it runs, the CPU can't access OAM, nor the bus the DMA reads from: the external bus
/// (cartridge and WRAM), or the video bus (VRAM). Reads there return the byte the DMA is copying.
/// The other bus, the IO registers and HRAM can be accessed as usual.
#[derive(Default, Serialize, Deserialize)]
struct OamDma {
    /// Last value written to DMA_REG, which is what it reads back as.
    register: u8,
    /// The transfer in progress: source address, and how many bytes have been copied.
    active: Option<(u16, u8)>,
    /// A transfer that's been requested: source address, and M-cycles left until it starts.
    requested: Option<(u16, u8)>,
}

impl OamDma {
    fn request(&mut self, page: u8) {
        self.register = page;
        // Pages above 0xDF read the echo of WRAM.
        let page = if page >= 0xE0 { page - 0x20 } else { page };
        self.requested = Some((u16::from(page) << 8, DMA_START_DELAY));
    }
}

#[derive(Serialize, Deserialize)]
pub struct Bus {
    ram: Box<[u8]>,
    hram: Box<[u8]>,
    apu: Apu,
    pub(crate) gfx: Gfx,
    pub(crate) cartridge: Cartridge,
    /// P1/JOYP Joypad contoller
    joypad: Joypad,
    /// Set when a joypad input line has gone low since the last `cycle()`.
    joypad_interrupt: bool,

    /// Boot ROM, until the boot sequence completes and it gets unmapped
    boot_rom: Option<BootRom>,

    /// IE - Interrupt Enable register
    interrupt_enable: InterruptFlag,
    /// IF - Interrupt Flag register
    interrupt_flag: InterruptFlag,
    /// Timer-related registers
    timer: Timer,
    /// SB - serial byte
    sb: u8,
    dma: OamDma,
}

impl Bus {
    pub fn new(cartridge: Cartridge, sample_rate: u32, boot_rom: Option<BootRom>) -> Self {
        Self {
            ram: vec![0; 0x2000].into_boxed_slice(),
            hram: vec![0; 0x80].into_boxed_slice(),
            apu: Apu::new(sample_rate),
            gfx: Gfx::new(),
            cartridge,
            joypad: Joypad::default(),
            joypad_interrupt: false,
            boot_rom,
            interrupt_enable: InterruptFlag::empty(),
            interrupt_flag: InterruptFlag::empty(),
            timer: Timer::new(),
            sb: 0,
            dma: OamDma::default(),
        }
    }

    /// Put the peripherals in the state the DMG boot ROM leaves them in when it jumps to the
    /// cartridge. Must only be called on a freshly created `Bus` without a boot ROM.
    ///
    /// See <https://gbdev.io/pandocs/Power_Up_Sequence.html>
    pub(crate) fn skip_boot(&mut self) {
        let logo = std::array::from_fn(|i| self.cartridge.read_rom(0x0104 + i as u16));
        self.gfx.skip_boot(&logo);
        self.apu.skip_boot();
        self.timer.set_div_counter(0xABCC);
        // The boot ROM waits for VBlank with interrupts disabled, so this stays pending
        self.interrupt_flag = InterruptFlag::VBLANK;
    }

    /// Carry over what save states leave out from the `Bus` this one replaces. On error, `previous`
    /// is left untouched.
    pub(crate) fn restore_unsaved(&mut self, previous: &mut Self) -> Result<()> {
        self.cartridge.restore_unsaved(&mut previous.cartridge)?;
        self.gfx.restore_unsaved(&previous.gfx);
        self.apu.restore_unsaved(&previous.apu);
        Ok(())
    }

    /// The header checksum, which the boot ROM verifies (and leaves traces of in the CPU flags).
    pub(crate) fn header_checksum(&self) -> u8 {
        self.cartridge.read_rom(0x014D)
    }

    /// Run the different peripherals for the given number of clock cycles
    pub fn cycle(
        &mut self,
        cycles: u8,
        frame_sink: &mut dyn FrameSink,
        audio_sink: &mut dyn AudioSink,
    ) {
        self.interrupt_flag |= self.gfx.dots(cycles, frame_sink);
        self.apu.step(cycles, audio_sink);
        self.cartridge.step(cycles);
        if self.timer.cycle(cycles) {
            self.interrupt_flag |= InterruptFlag::TIMER;
        }
        if self.joypad_interrupt {
            self.interrupt_flag |= InterruptFlag::JOYPAD;
            self.joypad_interrupt = false;
        }
        self.step_dma();
    }

    /// Run the OAM DMA for one M-cycle.
    fn step_dma(&mut self) {
        if let Some((source, copied)) = self.dma.active {
            let b = self.read_byte(source + u16::from(copied));
            self.gfx.write_oam_dma(copied, b);
            self.dma.active = (copied + 1 < DMA_LENGTH).then_some((source, copied + 1));
        }
        // A new transfer replaces the one in progress once it starts.
        if let Some((source, delay)) = self.dma.requested {
            if delay == 0 {
                self.dma.active = Some((source, 0));
                self.dma.requested = None;
            } else {
                self.dma.requested = Some((source, delay - 1));
            }
        }
    }

    /// If the OAM DMA stops the CPU from accessing `addr`, what a read returns instead.
    fn dma_conflict(&self, addr: u16) -> Option<u8> {
        let (source, copied) = self.dma.active?;
        let on_video_bus = |addr| VRAM.contains(&addr);
        if OAM.contains(&addr) || INVALID_AREA.contains(&addr) {
            Some(0xFF)
        } else if addr < *IO_REGISTERS.start() && on_video_bus(addr) == on_video_bus(source) {
            Some(self.read_byte(source + u16::from(copied)))
        } else {
            None
        }
    }

    pub fn read_byte(&self, addr: u16) -> u8 {
        if let Some(BootRom(boot_rom)) = &self.boot_rom
            && BOOT_ROM.contains(&addr)
        {
            // read from boot rom
            boot_rom[addr as usize]
        } else if CART_BANK_00.contains(&addr) || CART_BANK_MAPPED.contains(&addr) {
            self.cartridge.read_rom(addr)
        } else if VRAM.contains(&addr) {
            self.gfx.read_vram(addr)
        } else if EXT_RAM.contains(&addr) {
            trace!("External RAM 0x{:04x}", addr);
            self.cartridge.read_ram(addr - EXT_RAM.start())
        } else if WRAM.contains(&addr) {
            self.ram[(addr - WRAM.start()) as usize]
        } else if ECHO_RAM.contains(&addr) {
            // ECHO RAM: mirror of C000-DDFF
            trace!("Accessing ECHO RAM!");
            self.read_byte(addr - 0x2000)
        } else if OAM.contains(&addr) {
            // debug!("Reading Sprite attribute table (OAM): 0x{:04x}", addr);
            self.gfx.read_oam(addr)
        } else if INVALID_AREA.contains(&addr) {
            trace!("Invalid access to address 0x{:04x}", addr);
            0x00
        } else if IO_REGISTERS.contains(&addr) {
            self.read_io(addr)
        } else if HRAM.contains(&addr) {
            self.hram[(addr - HRAM.start()) as usize]
        } else if addr == 0xFFFF {
            trace!("Reading IE register: {:?}", self.interrupt_enable);
            self.interrupt_enable.bits()
        } else {
            unreachable!("How did we get here?");
        }
    }

    pub fn write_byte(&mut self, addr: u16, b: u8) {
        // Writes to the boot ROM area go to the cartridge's MBC, even while the boot ROM is mapped.
        if CART_BANK_00.contains(&addr) || CART_BANK_MAPPED.contains(&addr) {
            self.cartridge.write_rom(addr, b);
        } else if VRAM.contains(&addr) {
            self.gfx.write_vram(addr, b);
        } else if EXT_RAM.contains(&addr) {
            self.cartridge.write_ram(addr - EXT_RAM.start(), b);
        } else if WRAM.contains(&addr) {
            self.ram[(addr - WRAM.start()) as usize] = b;
        } else if ECHO_RAM.contains(&addr) {
            // ECHO RAM: mirror of C000-DDFF
            self.write_byte(addr - 0x2000, b);
        } else if OAM.contains(&addr) {
            // debug!("Writing Sprite attribute table (OAM): 0x{:04x}", addr);
            self.gfx.write_oam(addr, b);
        } else if INVALID_AREA.contains(&addr) {
            // Ignore writes to this area as some games reset it to 0 for some reason
            // warn!("Invalid access to address 0x{:04x}", addr);
        } else if IO_REGISTERS.contains(&addr) {
            self.write_io(addr, b);
        } else if HRAM.contains(&addr) {
            self.hram[(addr - HRAM.start()) as usize] = b;
        } else if addr == 0xFFFF {
            trace!("Setting Interrupt Enable Register with 0b{:08b}", b);
            self.interrupt_enable = InterruptFlag::from_bits_retain(b);
        } else {
            unreachable!("How did we get here? addr=0x{:04x}", addr);
        }
    }

    pub fn interrupt_enable(&self) -> InterruptFlag {
        self.interrupt_enable
    }

    pub fn interrupt_flag(&self) -> InterruptFlag {
        self.interrupt_flag
    }

    pub fn ack_interrupt(&mut self, flag: InterruptFlag) {
        self.interrupt_flag.remove(flag);
        trace!(
            "Acknowledging interrupt: {:?}. Pending: {:?}",
            flag, self.interrupt_flag
        );
    }

    pub fn interrupt_pending(&self) -> bool {
        !(self.interrupt_enable & self.interrupt_flag).is_empty()
    }

    /// Read access to IO registers
    fn read_io(&self, addr: u16) -> u8 {
        if IO_RANGE_JPD.contains(&addr) {
            // Joypad controller register
            trace!("Read Joypad controller register 0x{:04x}", addr);
            self.joypad.read()
        } else if IO_RANGE_COM.contains(&addr) {
            // Communication controller
            // FIXME: implement properly
            if addr == 0xFF01 { self.sb } else { 0x7E }
        } else if IO_RANGE_TIM.contains(&addr) {
            match addr {
                0xff04 => self.timer.div_timer(),
                0xff05 => self.timer.tima(),
                0xff06 => self.timer.tma(),
                0xff07 => self.timer.tac(),
                _ => unreachable!(),
            }
        } else if IO_RANGE_INT.contains(&addr) {
            // IF - interrupt flag
            // Note: unused bits are always 1
            self.interrupt_flag.bits() | 0b11100000
        } else if IO_RANGE_APU.contains(&addr) {
            // Sound
            self.apu.read_io(addr)
        } else if (0xFF27..=0xFF2F).contains(&addr) {
            // Technically part of the APU range, but not used
            0xff
        } else if IO_RANGE_WAV.contains(&addr) {
            // Waveform ram
            self.apu.read_wav(addr)
        } else if addr == DMA_REG {
            self.dma.register
        } else if IO_RANGE_LCD.contains(&addr) {
            // LCD
            // debug!("Read LCD controller 0x{:04x}", addr);
            self.gfx.read_reg(addr)
        } else {
            // some games access unknown registers for some reason, so just return FF
            0xFF
        }
    }

    /// Write access to IO registers.
    fn write_io(&mut self, addr: u16, b: u8) {
        if IO_RANGE_JPD.contains(&addr) {
            // Joypad controller register
            self.joypad_interrupt |= self.joypad.write(b);
            trace!(
                "Write Joypad controller register 0x{:04x}<-0x{:02X}. Register is now {:08b}",
                addr,
                b,
                self.joypad.read()
            );
        } else if IO_RANGE_COM.contains(&addr) {
            // Communication controller
            if addr == 0xFF01 {
                self.sb = b;
            }
        } else if IO_RANGE_TIM.contains(&addr) {
            match addr {
                0xff04 => self.timer.reset_div_timer(),
                0xff05 => self.timer.set_tima(b),
                0xff06 => self.timer.set_tma(b),
                0xff07 => self.timer.set_tac(b),
                _ => unreachable!(),
            }
        } else if IO_RANGE_INT.contains(&addr) {
            // IF - interrupt flag
            trace!("Setting IF with {:08b}", b);
            self.interrupt_flag = InterruptFlag::from_bits_truncate(b);
        } else if IO_RANGE_APU.contains(&addr) {
            // Sound
            trace!("Write sound register 0x{:04x}<-0x{:02X}", addr, b);
            self.apu.write_io(addr, b);
        } else if IO_RANGE_WAV.contains(&addr) {
            // Waveform ram
            trace!("Write waveform RAM 0x{:04x}<-0x{:02X}", addr, b);
            self.apu.write_wav(addr, b);
        } else if IO_RANGE_LCD.contains(&addr) {
            // LCD
            // debug!("Write LCD controller 0x{:04x}<-0x{:02X}", addr, b);
            if addr == DMA_REG {
                self.dma.request(b);
            } else {
                self.gfx.write_reg(addr, b);
            }
        } else if IO_RANGE_DBR.contains(&addr) {
            if b != 0 && self.boot_rom.take().is_some() {
                info!("Boot sequence complete. Disabling boot ROM.");
            }
        } else {
            trace!(
                "Write I/O Register 0x{:04x}<-0x{:02X} (NOT IMPLEMENTED)",
                addr, b
            );
        }
    }

    pub(crate) fn set_button_pressed(&mut self, button: crate::joypad::Button, is_pressed: bool) {
        // OR in the result: several buttons may be updated before the next `cycle()`, and a
        // later one that doesn't request the interrupt must not clear an earlier one's request.
        self.joypad_interrupt |= self.joypad.set_button(button, is_pressed);
    }
}

/// The bus as the CPU sees it. Every memory access takes one M-cycle, during which the rest of
/// the hardware runs, so accesses land on the right cycle within an instruction.
pub(crate) struct CpuBus<'a> {
    bus: &'a mut Bus,
    frame_sink: &'a mut dyn FrameSink,
    audio_sink: &'a mut dyn AudioSink,
    /// Clock cycles run so far.
    cycles: u8,
}

impl<'a> CpuBus<'a> {
    pub(crate) fn new(
        bus: &'a mut Bus,
        frame_sink: &'a mut dyn FrameSink,
        audio_sink: &'a mut dyn AudioSink,
    ) -> Self {
        Self {
            bus,
            frame_sink,
            audio_sink,
            cycles: 0,
        }
    }

    /// Read a byte, taking one M-cycle.
    pub(crate) fn read_byte(&mut self, addr: u16) -> u8 {
        let b = self
            .bus
            .dma_conflict(addr)
            .unwrap_or_else(|| self.bus.read_byte(addr));
        self.tick();
        b
    }

    /// Write a byte, taking one M-cycle.
    pub(crate) fn write_byte(&mut self, addr: u16, b: u8) {
        if self.bus.dma_conflict(addr).is_none() {
            self.bus.write_byte(addr, b);
        }
        self.tick();
    }

    /// Run the hardware for an M-cycle in which the CPU doesn't access memory.
    pub(crate) fn tick(&mut self) {
        self.bus.cycle(4, self.frame_sink, self.audio_sink);
        self.cycles += 4;
    }

    /// Clock cycles run so far.
    pub(crate) fn cycles(&self) -> u8 {
        self.cycles
    }

    pub(crate) fn interrupt_enable(&self) -> InterruptFlag {
        self.bus.interrupt_enable()
    }

    pub(crate) fn interrupt_flag(&self) -> InterruptFlag {
        self.bus.interrupt_flag()
    }

    pub(crate) fn interrupt_pending(&self) -> bool {
        self.bus.interrupt_pending()
    }

    pub(crate) fn ack_interrupt(&mut self, flag: InterruptFlag) {
        self.bus.ack_interrupt(flag);
    }
}
