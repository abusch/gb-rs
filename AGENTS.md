# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build & Run

- The DMG boot ROM is optional and loaded at runtime (`--boot-rom <PATH>` in the frontend, `dmg_boot.bin` in the system directory for the libretro core). Without it, `GameBoy::new` starts at `0x0100` in the post-boot state. `assets/dmg_boot.bin` is only used by the `skip_boot_matches_boot_rom` test, which is skipped if the file is absent.
- Run: `cargo run --release -- path/to/rom.gb`
- Useful CLI flags (see `src/main.rs`):
  - `-q / --quiet`: disable audio output (still drains the sample ring buffer in a background thread to prevent stalls).
  - `-b <HEX>`: set initial breakpoint (address is parsed as hex, no `0x` prefix).
  - `--enable-soft-break`: treat `LD B,B` as a breakpoint trigger (useful for some test ROMs).
  - `--boot-rom <PATH>`: run the given DMG boot ROM before the game.
- Logging is configured in `main.rs` via `env_logger` with hardcoded filters `gbrs=debug,gbrs::apu=info` (the frontend binary is also called `gbrs`, so this covers both crates). The `release_max_level_info` feature on the `log` crate caps release-build logs at `info` regardless of filter.
- Benchmark/profile the core with `cargo run --release --example headless -p gbrs -- <ROM> [FRAMES]`: it runs without a frontend and prints the speed plus hashes of the video and audio output, so optimisations can be checked for behaviour changes.
- Test ROMs: `just test_roms` downloads the suites into `test_roms/`, and `just rom-tests` runs the DMG-relevant ones (blargg, mooneye, dmg-acid2, mealybug, age) and prints a pass count per suite, in a second or two (`gbrs/examples/test_roms.rs`). Save a baseline with `--save FILE` before changing emulation, then `--compare FILE` lists exactly which tests changed; `-v` lists every result and a trailing argument filters tests by path. Accuracy work should show progress there without regressions.
- Release profile has `debug = true` and `incremental = true` — debugging a release build is intentionally supported (emulation needs release-level perf).

## Architecture

### Layering

The crate is split into a library (`src/lib.rs`) and a binary (`src/main.rs` + `src/emulator.rs` + `src/debugger.rs`). The library is the emulation core and is deliberately I/O-agnostic: it communicates with the outside world through two traits defined in `lib.rs`:

- `FrameSink::push_frame(&[Rgb555])` — called by the PPU when a complete 160×144 frame is ready. Pixels are 15-bit colours in the CGB's native `xBBBBBGGGGGRRRRR` layout (chosen so CGB support can reuse it); DMG shades are mapped through a configurable palette (`GameBoy::set_dmg_palette`, default `gfx::DEFAULT_DMG_PALETTE`). Frontends convert with `Rgb555::to_rgb888`.
- `AudioSink::push_sample` / `push_samples` — called by the APU to emit stereo f32 samples.

The binary provides concrete implementations: `MostRecentFrameSink` (just keeps the latest frame for the window renderer) and `CpalAudioSink` (pushes samples into a `ringbuf::HeapRb<f32>` that cpal drains on its audio thread). Keep this separation when touching rendering or audio: the library should never depend on `winit`, `pixels`, or `cpal`.

### Execution model

`Emulator::update()` (in `src/emulator.rs`) is driven by `winit`'s `RedrawRequested` event. It does **wall-clock-paced emulation**:

- `CPU_CYCLE_TIME_NS = 238` (i.e. 1 / 4.194304 MHz).
- Each update computes `target_cycles = elapsed_ns / 238` and steps the Game Boy until `emulated_cycles >= target_cycles` or the CPU is paused.
- When resuming from the debugger, `start_time_ns` is rebased so that `elapsed - emulated_cycles*238` is preserved — otherwise a long pause would cause a catch-up burst.

`GameBoy::step()` runs one CPU instruction, then the interrupt dispatch if one is due. The CPU accesses memory through `CpuBus` (in `bus.rs`), whose `read_byte`/`write_byte` run the rest of the hardware (PPU, APU, timer) for one M-cycle after each access, so every access lands on the M-cycle it does on hardware (blargg `mem_timing` and mooneye's access-timing tests rely on this). Internal M-cycles that come *before* an access must be run explicitly with `CpuBus::tick` (e.g. `push_word`, conditional `RET`, interrupt dispatch); the ones at the end of an instruction are run by `GameBoy::step`, which pads up to the cycle count the instruction returns. Interrupts are only checked between instructions.

### Bus & memory map

`src/bus.rs` is the hub. It owns: `Apu`, `Gfx`, `Cartridge`, `Joypad`, `Timer`, WRAM, HRAM, interrupt registers, and the serial byte. The memory map and IO-register ranges are declared as top-of-file `RangeInclusive<u16>` constants; `read_byte`/`write_byte` dispatch against them. When adding a new IO register, add a new range constant and extend `read_io`/`write_io`. The joypad interrupt is requested when one of P1's input lines goes low (`Joypad::set_button` and `Joypad::write` return whether one did): pressing a button in a selected group, or selecting a group while one of its buttons is held, but not releasing a button. Some games (e.g. Lawnmower Man) alternate the selected group every frame and rely on the latter to notice presses.

Boot-ROM handling: if a `BootRom` was given, addresses `0x0000..=0x00FF` return its bytes until a non-zero write to `0xFF50` drops it (`boot_rom = None`). After that, cart ROM is visible in that range. Without a boot ROM, `GameBoy::new` calls the `skip_boot` methods (`Cpu`, `Bus`, and through it `Gfx`/`Apu`/`Timer`) to reproduce the state the boot ROM leaves behind (registers, IO, logo in VRAM; see Pan Docs "Power Up Sequence"). If you change what a peripheral's power-on state looks like, check that `skip_boot` still matches.

ROMs and cart saves: the core does no file I/O. `Cartridge::load_bytes` and `BootRom::load_bytes` take raw bytes, and frontends read the files, including unpacking `.zip` ROMs (`read_rom` in `gbrs_frontend/src/emulator.rs`). Battery-backed RAM is exposed as `GameBoy::save_ram`/`save_ram_mut`. The desktop frontend restores it from a `.sav` sibling of the ROM in `Emulator::new` and writes it back in `Emulator::finish()`. The libretro core hands it to RetroArch as `SaveRam`. If you add new MBC support, make its RAM available through `save_ram`.

### Cartridge & MBCs (`src/cartridge/`)

`Cartridge` holds the ROM, a RAM buffer sized from the header, and an `Mbc` enum with one variant per memory bank controller, each in its own file (`mbc1.rs`, `mbc2.rs`, `mbc3.rs`, `mbc5.rs`). `Cartridge` dispatches `read_rom`/`write_rom` (the bus sends every write to 0000-7FFF there, as that's where the MBC registers live) and `read_ram`/`write_ram` to it. ROM-only carts (`Mbc::None`), MBC1, MBC2, MBC3 and MBC5 are implemented; the remaining rare types (MMM01, MBC6/7, HuC1/3, Pocket Camera, TAMA5) fall back to `Mbc1` with a warning, so adding an MBC means adding a variant and a `load_bytes` match arm. MBC2's RAM is built into the chip, so the header doesn't declare it: `load_bytes` sizes it specially, as 512 bytes holding one half-byte each, which is also its save file format. Mappers wrap bank numbers to the ROM/RAM size and return 0xFF for unmapped reads rather than indexing out of bounds. MBC1M multicarts don't say so in their header, so `Cartridge::is_multicart` detects them by the second game's Nintendo logo at bank 0x10; `Mbc1` then wires BANK2 one bit lower. All the mooneye `emulator-only/mbc1`, `mbc2` and `mbc5` ROMs pass.

The MBC3 real-time clock (`rtc.rs`) runs on emulated time: `Bus::cycle` calls `Cartridge::step`, and the seconds counter ticks every 4194304 cycles. Its edge cases (out-of-range values wrap without carrying, writing seconds resets the sub-second counter, halting freezes it) are checked by `test_roms/rtc3test`. Time passed while switched off is applied when loading a save: `GameBoy::save_rtc`/`load_rtc` use BGB/VBA-M's 48-byte format stamped with a Unix time from the frontend, and the desktop frontend appends it to the `.sav` file after the RAM. The libretro core exposes the same data as `MemoryRegion::Rtc`, which RetroArch keeps in a `.rtc` file: since the frontend reads and writes that buffer whenever it likes, `RtcRegion::sync` refreshes it every frame, and treats contents it didn't write itself as a restored save to load.

### CPU

`src/cpu/mod.rs` — SM83 interpreter. Key state: `regs` (see `cpu/register.rs` for the `Reg`/`RegPair` abstraction), `sp`, `pc`, `halted`, `ime` (Interrupt Master Enable), plus three debug-only fields (`breakpoint`, `paused`, `enable_soft_break`) and a `halt_bug` flag for emulating the HALT bug. Interrupt vectors live at the top of the file as `ITR_VBLANK`/`ITR_STAT`/`ITR_TIMER`/`ITR_SERIAL`/`ITR_JOYP`. Interrupt delivery happens in `handle_interrupt`, called between instructions by `GameBoy::step`; dispatch takes 5 M-cycles and picks the interrupt only after pushing PC's high byte (which can overwrite IE, see mooneye `ie_push`). EI only sets IME at the end of the following instruction (`ime_delay`), so `EI; HALT` with an interrupt pending triggers the halt bug, and the handler then returns to the HALT.

### PPU (`src/gfx.rs`)

Cycle-accurate-ish PPU driven by `dots(cycles, frame_sink)`. It tracks `line_dot` (dots since the start of the line), `ly` and `running_mode` (OAM scan / drawing / HBlank / VBlank). The mode only changes at fixed dots (`MODE3_START_DOT`, `MODE0_START_DOT`, end of line), and a whole scanline is rendered by `draw_scan_line` when mode 3 starts. `dots` runs the first dot of each call in full and then skips ahead to the next mode change, since nothing else can happen in between: keep that invariant if you add per-dot behaviour. STAT interrupt sources and conditions are `STAT_*` bitmasks; the STAT interrupt fires on a rising edge of `stat_sources & stat_conditions`. LCDC is decomposed into individual boolean fields rather than kept as a bitmask — when you touch `0xFF40`, update both the raw-register write path and the boolean fields. Turning the LCD off (LCDC bit 7) stops the PPU: `dots` does nothing, LY reads 0 and STAT mode 0, and turning it back on starts a new frame from line 0, dot 0, so the first VBlank comes a full 144 lines later. Games time their loading around that: when the PPU kept counting lines while off, Super Mario Land 2 got VBlank too early after a screen change and crashed. The debugger doesn't turn the LCD off to read VRAM/OAM; it uses `Gfx::set_debugger_access` instead, so pausing doesn't change the emulated state.

### APU (`src/apu/`)

`mod.rs` owns the four channels (`ToneChannel` ×2, `WaveChannel`, `NoiseChannel` — all in `channels.rs`) and the 512 Hz `FrameSequencer` (`frame_sequencer.rs`) that clocks length counters, envelopes, and sweep. A `HighPassFilter` is applied before emitting to the `AudioSink`. The APU runs at CPU clock (4.194304 MHz) and downsamples to the sample rate given to `GameBoy::new` (exact integer resampling, see `sample_counter`); audio register addresses `NR10..NR52` are defined as constants at the top of `mod.rs`.

The APU runs lazily: `Apu::step` only accumulates `pending_cycles`, which are run when a frame sequencer step or a sample is due, or before a register write (`catch_up`). In between, the channels' `Timer`s are advanced arithmetically (`Timer::advance`), not cycle by cycle. So if you add anything that observes channel state from outside at other times (e.g. wave RAM reads while the channel is playing), call `catch_up` first.

### Save states

`GameBoy::save_state`/`load_state` serialise the whole machine with serde and `postcard`: every component derives `Serialize`/`Deserialize`, so **new state fields are saved automatically, but any change to the shape of a component's state must bump `SAVE_STATE_VERSION`** in `gameboy.rs` (old states are then rejected rather than misread). A header also records the ROM's checksums, so a state can't be loaded into another game. Fields that are frontend settings or debugger state rather than machine state (the ROM, the DMG palette, the sample rate, breakpoints) are `#[serde(skip)]`, and each component's `restore_unsaved` carries them over from the machine being replaced: add to it when you add such a field. `Rgb555` is encoded as a fixed 2 bytes so that the framebuffer's size doesn't depend on what's on screen; keep large buffers fixed-size like that, because the libretro core has to announce a `serialize_size` that can't grow (it adds `SAVE_STATE_SLACK` for the few varint fields). The libretro core prefixes its states with `cycle_carry`, so that rewind and run-ahead replay the exact same cycles. The desktop frontend has a single quick-save slot: F5/F7 write and read a `.state` file next to the ROM (`Emulator::save_state`/`load_state`), and failures are only logged.

### Input (`gbrs_frontend/src/input.rs`)

The keyboard and gamepads (through `gilrs`) each keep their own `Buttons` set, and `Emulator::set_buttons` only tells the Game Boy when their union changes, so releasing a key doesn't release a button still held on a gamepad. Gamepads are polled once per `Emulator::update`, from the current state of every connected pad rather than from individual events: a pad that disconnects releases its buttons, and stick jitter can't release a D-pad direction. A and B are mapped by position (right and bottom face buttons), as on the Game Boy.

### Debugger (`src/debugger.rs`)

An in-process CLI debugger using `rustyline`. Pressing a designated key (see `README.md`) pauses the CPU and hands control to a prompt (`gb-rs> `). Commands: `next [N]`, `continue`, `cpu`, `mem <hex>`, `dis <hex>`, `br <hex>`, `oam`, `palettes`, `sprite <id>`, `quit`. The debugger is purely a `main` binary concern and is not in the library.

## Conventions

- Hex addresses in CLI/debugger input are **always** parsed without `0x` prefix (see `parse_addr` in `main.rs` and `u16::from_str_radix(_, 16)` in `debugger.rs`). Preserve this when extending the debugger.
- Unknown/unimplemented IO reads return `0xFF` and writes are logged at `trace` rather than panicking — some commercial ROMs touch undocumented registers.
- The `INVALID_AREA` range (`0xFEA0..=0xFEFF`) silently absorbs writes — several games reset it to 0. Don't "fix" this by panicking.
