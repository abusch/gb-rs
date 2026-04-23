# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build & Run

- Build requires `assets/dmg_boot.bin` (DMG boot ROM) at compile time — it is `include_bytes!`'d into the binary from `src/bus.rs`. Without this file, compilation fails.
- Run: `cargo run --release -- path/to/rom.gb`
- Useful CLI flags (see `src/main.rs`):
  - `-q / --quiet`: disable audio output (still drains the sample ring buffer in a background thread to prevent stalls).
  - `-b <HEX>`: set initial breakpoint (address is parsed as hex, no `0x` prefix).
  - `--enable-soft-break`: treat `LD B,B` as a breakpoint trigger (useful for some test ROMs).
- Logging is configured in `main.rs` via `env_logger` with hardcoded filters `gb_rs=debug,gb_rs::apu=info`. The `release_max_level_info` feature on the `log` crate caps release-build logs at `info` regardless of filter.
- Release profile has `debug = true` and `incremental = true` — debugging a release build is intentionally supported (emulation needs release-level perf).

## Architecture

### Layering

The crate is split into a library (`src/lib.rs`) and a binary (`src/main.rs` + `src/emulator.rs` + `src/debugger.rs`). The library is the emulation core and is deliberately I/O-agnostic: it communicates with the outside world through two traits defined in `lib.rs`:

- `FrameSink::push_frame(&[(u8, u8, u8)])` — called by the PPU when a complete 160×144 RGB frame is ready.
- `AudioSink::push_sample` / `push_samples` — called by the APU to emit stereo f32 samples.

The binary provides concrete implementations: `MostRecentFrameSink` (just keeps the latest frame for the window renderer) and `CpalAudioSink` (pushes samples into a `ringbuf::HeapRb<f32>` that cpal drains on its audio thread). Keep this separation when touching rendering or audio: the library should never depend on `winit`, `pixels`, or `cpal`.

### Execution model

`Emulator::update()` (in `src/emulator.rs`) is driven by `winit`'s `RedrawRequested` event. It does **wall-clock-paced emulation**:

- `CPU_CYCLE_TIME_NS = 238` (i.e. 1 / 4.194304 MHz).
- Each update computes `target_cycles = elapsed_ns / 238` and steps the Game Boy until `emulated_cycles >= target_cycles` or the CPU is paused.
- When resuming from the debugger, `start_time_ns` is rebased so that `elapsed - emulated_cycles*238` is preserved — otherwise a long pause would cause a catch-up burst.

`GameBoy::step()` runs one CPU instruction, then for each of the resulting M-cycles ticks the `Bus` once and lets the CPU handle interrupts. So peripherals (GPU/APU/Timer) advance in lockstep with the CPU at cycle granularity — not per-instruction.

### Bus & memory map

`src/bus.rs` is the hub. It owns: `Apu`, `Gfx`, `Cartridge`, `Joypad`, `Timer`, WRAM, HRAM, interrupt registers, and the serial byte. The memory map and IO-register ranges are declared as top-of-file `RangeInclusive<u16>` constants; `read_byte`/`write_byte` dispatch against them. When adding a new IO register, add a new range constant and extend `read_io`/`write_io`.

Boot-ROM handling: addresses `0x0000..=0x00FF` return boot ROM bytes until a non-zero write to `0xFF50` flips `has_booted = true`. After that, cart ROM is visible in that range. The boot ROM is baked into the binary via `include_bytes!("../assets/dmg_boot.bin")`.

Cart saves: `Cartridge::load` looks for a `.sav` sibling of the ROM and loads it into external RAM if present; `GameBoy::save()` (called on exit via `Emulator::finish()`) writes it back. If you add new MBC support, wire the save/load paths through the same mechanism.

### CPU

`src/cpu/mod.rs` — SM83 interpreter. Key state: `regs` (see `cpu/register.rs` for the `Reg`/`RegPair` abstraction), `sp`, `pc`, `halted`, `ime` (Interrupt Master Enable), plus three debug-only fields (`breakpoint`, `paused`, `enable_soft_break`) and a `halt_bug` flag for emulating the HALT bug. Interrupt vectors live at the top of the file as `ITR_VBLANK`/`ITR_STAT`/`ITR_TIMER`/`ITR_SERIAL`/`ITR_JOYP`. Interrupt delivery happens in `handle_interrupt`, called after every M-cycle batch from `GameBoy::step`.

### PPU (`src/gfx.rs`)

Cycle-accurate-ish PPU driven by `dots(cycles, frame_sink)`. It tracks `dots` (cycles since frame start), a `running_mode` state machine (OAM scan / drawing / HBlank / VBlank), and per-scanline `LineDrawingState`. LCDC is decomposed into individual boolean fields rather than kept as a bitmask — when you touch `0xFF40`, update both the raw-register write path and the boolean fields.

### APU (`src/apu/`)

`mod.rs` owns the four channels (`ToneChannel` ×2, `WaveChannel`, `NoiseChannel` — all in `channels.rs`) and the 512 Hz `FrameSequencer` (`frame_sequencer.rs`) that clocks length counters, envelopes, and sweep. A `HighPassFilter` is applied before emitting to the `AudioSink`. The APU runs at CPU clock (4.194304 MHz) and downsamples to 44.1 kHz; audio register addresses `NR10..NR52` are defined as constants at the top of `mod.rs`.

### Debugger (`src/debugger.rs`)

An in-process CLI debugger using `rustyline`. Pressing a designated key (see `README.md`) pauses the CPU and hands control to a prompt (`gb-rs> `). Commands: `next [N]`, `continue`, `cpu`, `mem <hex>`, `dis <hex>`, `br <hex>`, `oam`, `palettes`, `sprite <id>`, `quit`. The debugger is purely a `main` binary concern and is not in the library.

## Conventions

- Hex addresses in CLI/debugger input are **always** parsed without `0x` prefix (see `parse_addr` in `main.rs` and `u16::from_str_radix(_, 16)` in `debugger.rs`). Preserve this when extending the debugger.
- Unknown/unimplemented IO reads return `0xFF` and writes are logged at `trace` rather than panicking — some commercial ROMs touch undocumented registers.
- The `INVALID_AREA` range (`0xFEA0..=0xFEFF`) silently absorbs writes — several games reset it to 0. Don't "fix" this by panicking.
