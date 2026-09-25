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
- Logging is configured in `main.rs` via `env_logger` with hardcoded filters `gb_rs=debug,gb_rs::apu=info`. The `release_max_level_info` feature on the `log` crate caps release-build logs at `info` regardless of filter.
- Benchmark/profile the core with `cargo run --release --example headless -p gbrs -- <ROM> [FRAMES]`: it runs without a frontend and prints the speed plus hashes of the video and audio output, so optimisations can be checked for behaviour changes.
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

`GameBoy::step()` runs one CPU instruction, then for each of the resulting M-cycles ticks the `Bus` once (with 4 T-cycles) and lets the CPU handle interrupts. So peripherals (GPU/APU/Timer) advance in lockstep with the CPU at M-cycle granularity — not per-instruction.

### Bus & memory map

`src/bus.rs` is the hub. It owns: `Apu`, `Gfx`, `Cartridge`, `Joypad`, `Timer`, WRAM, HRAM, interrupt registers, and the serial byte. The memory map and IO-register ranges are declared as top-of-file `RangeInclusive<u16>` constants; `read_byte`/`write_byte` dispatch against them. When adding a new IO register, add a new range constant and extend `read_io`/`write_io`.

Boot-ROM handling: if a `BootRom` was given, addresses `0x0000..=0x00FF` return its bytes until a non-zero write to `0xFF50` drops it (`boot_rom = None`). After that, cart ROM is visible in that range. Without a boot ROM, `GameBoy::new` calls the `skip_boot` methods (`Cpu`, `Bus`, and through it `Gfx`/`Apu`/`Timer`) to reproduce the state the boot ROM leaves behind (registers, IO, logo in VRAM; see Pan Docs "Power Up Sequence"). If you change what a peripheral's power-on state looks like, check that `skip_boot` still matches.

ROMs and cart saves: the core does no file I/O. `Cartridge::load_bytes` and `BootRom::load_bytes` take raw bytes, and frontends read the files, including unpacking `.zip` ROMs (`read_rom` in `gbrs_frontend/src/emulator.rs`). Battery-backed RAM is exposed as `GameBoy::save_ram`/`save_ram_mut`. The desktop frontend restores it from a `.sav` sibling of the ROM in `Emulator::new` and writes it back in `Emulator::finish()`. The libretro core hands it to RetroArch as `SaveRam`. If you add new MBC support, make its RAM available through `save_ram`.

### Cartridge & MBCs (`src/cartridge/`)

`Cartridge` holds the ROM, a RAM buffer sized from the header, and an `Mbc` enum with one variant per memory bank controller, each in its own file (`mbc1.rs`, `mbc2.rs`, `mbc3.rs`, `mbc5.rs`). `Cartridge` dispatches `read_rom`/`write_rom` (the bus sends every write to 0000-7FFF there, as that's where the MBC registers live) and `read_ram`/`write_ram` to it. MBC1, MBC2, MBC3 and MBC5 are implemented; every other cartridge type falls back to `Mbc1`, which is what the emulator did before MBCs were split out, so adding an MBC means adding a variant and a `load_bytes` match arm. Keep the MBC1 fallback's `bank_0_selects_1` quirk until the types relying on it (ROM-only carts and the rare unsupported types) get their own mapper. MBC2's RAM is built into the chip, so the header doesn't declare it: `load_bytes` sizes it specially, as 512 bytes holding one half-byte each, which is also its save file format. Mappers wrap bank numbers to the ROM/RAM size and return 0xFF for unmapped reads rather than indexing out of bounds. MBC1M multicarts don't say so in their header, so `Cartridge::is_multicart` detects them by the second game's Nintendo logo at bank 0x10; `Mbc1` then wires BANK2 one bit lower. All the mooneye `emulator-only/mbc1`, `mbc2` and `mbc5` ROMs pass.

The MBC3 real-time clock (`rtc.rs`) runs on emulated time: `Bus::cycle` calls `Cartridge::step`, and the seconds counter ticks every 4194304 cycles. Its edge cases (out-of-range values wrap without carrying, writing seconds resets the sub-second counter, halting freezes it) are checked by `test_roms/rtc3test`. Time passed while switched off is applied when loading a save: `GameBoy::save_rtc`/`load_rtc` use BGB/VBA-M's 48-byte format stamped with a Unix time from the frontend, and the desktop frontend appends it to the `.sav` file after the RAM. The libretro core doesn't persist the RTC yet.

### CPU

`src/cpu/mod.rs` — SM83 interpreter. Key state: `regs` (see `cpu/register.rs` for the `Reg`/`RegPair` abstraction), `sp`, `pc`, `halted`, `ime` (Interrupt Master Enable), plus three debug-only fields (`breakpoint`, `paused`, `enable_soft_break`) and a `halt_bug` flag for emulating the HALT bug. Interrupt vectors live at the top of the file as `ITR_VBLANK`/`ITR_STAT`/`ITR_TIMER`/`ITR_SERIAL`/`ITR_JOYP`. Interrupt delivery happens in `handle_interrupt`, called after every M-cycle batch from `GameBoy::step`.

### PPU (`src/gfx.rs`)

Cycle-accurate-ish PPU driven by `dots(cycles, frame_sink)`. It tracks `line_dot` (dots since the start of the line), `ly` and `running_mode` (OAM scan / drawing / HBlank / VBlank). The mode only changes at fixed dots (`MODE3_START_DOT`, `MODE0_START_DOT`, end of line), and a whole scanline is rendered by `draw_scan_line` when mode 3 starts. `dots` runs the first dot of each call in full and then skips ahead to the next mode change, since nothing else can happen in between: keep that invariant if you add per-dot behaviour. STAT interrupt sources and conditions are `STAT_*` bitmasks; the STAT interrupt fires on a rising edge of `stat_sources & stat_conditions`. LCDC is decomposed into individual boolean fields rather than kept as a bitmask — when you touch `0xFF40`, update both the raw-register write path and the boolean fields.

### APU (`src/apu/`)

`mod.rs` owns the four channels (`ToneChannel` ×2, `WaveChannel`, `NoiseChannel` — all in `channels.rs`) and the 512 Hz `FrameSequencer` (`frame_sequencer.rs`) that clocks length counters, envelopes, and sweep. A `HighPassFilter` is applied before emitting to the `AudioSink`. The APU runs at CPU clock (4.194304 MHz) and downsamples to the sample rate given to `GameBoy::new` (exact integer resampling, see `sample_counter`); audio register addresses `NR10..NR52` are defined as constants at the top of `mod.rs`.

The APU runs lazily: `Apu::step` only accumulates `pending_cycles`, which are run when a frame sequencer step or a sample is due, or before a register write (`catch_up`). In between, the channels' `Timer`s are advanced arithmetically (`Timer::advance`), not cycle by cycle. So if you add anything that observes channel state from outside at other times (e.g. wave RAM reads while the channel is playing), call `catch_up` first.

### Debugger (`src/debugger.rs`)

An in-process CLI debugger using `rustyline`. Pressing a designated key (see `README.md`) pauses the CPU and hands control to a prompt (`gb-rs> `). Commands: `next [N]`, `continue`, `cpu`, `mem <hex>`, `dis <hex>`, `br <hex>`, `oam`, `palettes`, `sprite <id>`, `quit`. The debugger is purely a `main` binary concern and is not in the library.

## Conventions

- Hex addresses in CLI/debugger input are **always** parsed without `0x` prefix (see `parse_addr` in `main.rs` and `u16::from_str_radix(_, 16)` in `debugger.rs`). Preserve this when extending the debugger.
- Unknown/unimplemented IO reads return `0xFF` and writes are logged at `trace` rather than panicking — some commercial ROMs touch undocumented registers.
- The `INVALID_AREA` range (`0xFEA0..=0xFEFF`) silently absorbs writes — several games reset it to 0. Don't "fix" this by panicking.
