//! Run the test ROM suites from `test_roms/` (downloaded with `just test_roms`) and report which
//! tests pass. Only tests that apply to the DMG are run.
//!
//! Usage: `cargo run --release --example test_roms -p gbrs -- [OPTIONS] [FILTER]`
//!
//! - `-v`: list every test's result, not just a summary per suite
//! - `--save FILE`: save the results to FILE
//! - `--compare FILE`: list the tests whose result changed since the run saved in FILE
//! - `FILTER`: only run the tests whose path contains FILTER

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

use anyhow::{Context, Result, bail};
use gbrs::{CPU_HZ, CYCLES_PER_FRAME, FrameSink, Rgb555, cartridge::Cartridge, gameboy::GameBoy};

/// The 4 DMG shades used by the reference screenshots.
const SHADES: [u8; 4] = [0xFF, 0xAA, 0x55, 0x00];

/// How a test reports its result.
enum Check {
    /// Run until `LD B,B`, then check for Fibonacci numbers in the registers (mooneye-style).
    Registers,
    /// Wait for a result code at $A000, after the signature DE B0 61 (blargg's newer tests).
    ResultCode,
    /// Compare the screen with a reference screenshot, after running for the whole time or until
    /// `LD B,B`.
    Screenshot { png: PathBuf, until_ld_b_b: bool },
}

struct Test {
    suite: &'static str,
    rom: PathBuf,
    check: Check,
    /// How long to run for at most, in emulated seconds.
    seconds: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Outcome {
    Pass,
    Fail(String),
    Timeout,
    Panic,
}

impl Outcome {
    /// The outcome without its details, as saved in results files.
    fn kind(&self) -> &'static str {
        match self {
            Outcome::Pass => "PASS",
            Outcome::Fail(_) => "FAIL",
            Outcome::Timeout => "TIMEOUT",
            Outcome::Panic => "PANIC",
        }
    }
}

#[derive(Default)]
struct ScreenSink(Vec<Rgb555>);

impl FrameSink for ScreenSink {
    fn push_frame(&mut self, frame: &[Rgb555]) {
        self.0.clear();
        self.0.extend_from_slice(frame);
    }
}

/// Whether a test for the given models (the last part of its file name, in mooneye's naming
/// scheme, e.g. `-GS` or `-dmgABCmgb`) runs on a DMG.
fn runs_on_dmg(rom: &Path) -> bool {
    let stem = rom.file_stem().unwrap().to_string_lossy();
    match stem.rsplit_once('-') {
        None => true,
        Some((_, models)) if models.chars().all(|c| c.is_ascii_uppercase()) => models.contains('G'),
        Some((_, models)) => models.starts_with("dmgABC"),
    }
}

/// The `.gb` files in `dir` and its subdirectories, sorted.
fn roms(dir: &Path) -> Vec<PathBuf> {
    let mut roms = Vec::new();
    let mut dirs = vec![dir.to_path_buf()];
    while let Some(dir) = dirs.pop() {
        for entry in fs::read_dir(&dir).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() {
                dirs.push(path);
            } else if path.extension().is_some_and(|ext| ext == "gb") {
                roms.push(path);
            }
        }
    }
    roms.sort();
    roms
}

fn all_tests(root: &Path) -> Vec<Test> {
    let mut tests = Vec::new();
    let screenshot = |suite, rom: PathBuf, png: PathBuf, seconds, until_ld_b_b| Test {
        suite,
        rom,
        check: Check::Screenshot { png, until_ld_b_b },
        seconds,
    };

    let blargg = root.join("blargg");
    for (dir, rom, png, seconds) in [
        ("cpu_instrs", "cpu_instrs", "cpu_instrs-dmg-cgb", 55),
        ("instr_timing", "instr_timing", "instr_timing-dmg-cgb", 1),
        (".", "halt_bug", "halt_bug-dmg-cgb", 2),
        ("mem_timing", "mem_timing", "mem_timing-dmg-cgb", 3),
        ("mem_timing-2", "mem_timing", "mem_timing-dmg-cgb", 4),
        ("oam_bug", "oam_bug", "oam_bug-dmg", 21),
        ("dmg_sound", "dmg_sound", "dmg_sound-dmg", 36),
    ] {
        let dir = blargg.join(dir);
        tests.push(screenshot(
            "blargg",
            dir.join(format!("{rom}.gb")),
            dir.join(format!("{png}.png")),
            seconds,
            false,
        ));
    }
    for dir in ["mem_timing-2", "oam_bug", "dmg_sound"] {
        for rom in roms(&blargg.join(dir).join("rom_singles")) {
            tests.push(Test {
                suite: "blargg singles",
                rom,
                check: Check::ResultCode,
                seconds: 60,
            });
        }
    }

    let mooneye = root.join("mooneye-test-suite");
    for (suite, dir) in [
        ("mooneye acceptance", "acceptance"),
        ("mooneye emulator-only", "emulator-only"),
    ] {
        for rom in roms(&mooneye.join(dir))
            .into_iter()
            .filter(|rom| runs_on_dmg(rom))
        {
            tests.push(Test {
                suite,
                rom,
                check: Check::Registers,
                seconds: 20,
            });
        }
    }

    // Checked against a screenshot rather than by the registers, unlike mooneye's other tests.
    let manual = mooneye.join("manual-only");
    tests.push(screenshot(
        "mooneye manual-only",
        manual.join("sprite_priority.gb"),
        manual.join("sprite_priority-dmg.png"),
        5,
        true,
    ));

    let acid2 = root.join("dmg-acid2");
    tests.push(screenshot(
        "dmg-acid2",
        acid2.join("dmg-acid2.gb"),
        acid2.join("dmg-acid2-dmg.png"),
        10,
        true,
    ));

    let mealybug = root.join("mealybug-tearoom-tests/ppu");
    for rom in roms(&mealybug) {
        let png = rom.with_file_name(format!(
            "{}_dmg_blob.png",
            rom.file_stem().unwrap().to_string_lossy()
        ));
        if png.exists() {
            tests.push(screenshot("mealybug", rom, png, 10, true));
        }
    }

    // The file names of age's tests say which models they were verified on.
    let age = root.join("age-test-roms");
    for rom in roms(&age) {
        let stem = rom.file_stem().unwrap().to_string_lossy().into_owned();
        let png = rom.with_file_name(format!("{stem}-dmgC.png"));
        if png.exists() {
            tests.push(screenshot("age", rom, png, 10, true));
        } else if stem.contains("-dmgC") {
            tests.push(Test {
                suite: "age",
                rom,
                check: Check::Registers,
                seconds: 20,
            });
        }
    }

    tests
}

/// Load a reference screenshot as DMG shade indices (0 = lightest).
fn load_shades(png: &Path) -> Result<Vec<u8>> {
    let file = fs::File::open(png).with_context(|| format!("Failed to open {}", png.display()))?;
    let mut decoder = png::Decoder::new(std::io::BufReader::new(file));
    decoder.set_transformations(png::Transformations::EXPAND);
    let mut reader = decoder.read_info()?;
    let mut buf = vec![0; reader.output_buffer_size().context("PNG too large")?];
    let info = reader.next_frame(&mut buf)?;
    let samples = info.color_type.samples();
    Ok(buf[..info.buffer_size()]
        .chunks(samples)
        .map(|pixel| shade(pixel[0]))
        .collect())
}

/// The DMG shade closest to a grey level.
fn shade(level: u8) -> u8 {
    (0..4)
        .min_by_key(|&i| SHADES[i as usize].abs_diff(level))
        .unwrap()
}

fn run_test(test: &Test) -> Result<Outcome> {
    let cartridge = Cartridge::load_bytes(fs::read(&test.rom)?)?;
    let expected = match &test.check {
        Check::Screenshot { png, .. } => Some(load_shades(png)?),
        _ => None,
    };
    let stop_on_ld_b_b = matches!(
        test.check,
        Check::Registers
            | Check::Screenshot {
                until_ld_b_b: true,
                ..
            }
    );

    let run = std::panic::catch_unwind(|| {
        let mut gb = GameBoy::new(cartridge, None, 48_000);
        gb.set_soft_break(stop_on_ld_b_b);
        gb.set_dmg_palette(SHADES.map(|level| Rgb555::from_rgb888(level, level, level)));
        let mut screen = ScreenSink::default();
        let mut finished = false;
        'frames: for _ in 0..test.seconds * CPU_HZ / CYCLES_PER_FRAME {
            let mut cycles = 0;
            while cycles < CYCLES_PER_FRAME {
                if gb.is_paused() {
                    finished = true;
                    break 'frames;
                }
                cycles += gb.step(&mut screen, &mut ());
            }
            if matches!(test.check, Check::ResultCode)
                && [0xA001, 0xA002, 0xA003].map(|addr| gb.peek(addr)) == [0xDE, 0xB0, 0x61]
                && gb.peek(0xA000) != 0x80
            {
                finished = true;
                break;
            }
        }
        (gb, screen, finished)
    });
    let Ok((gb, screen, finished)) = run else {
        return Ok(Outcome::Panic);
    };

    Ok(match &test.check {
        Check::Registers if !finished => Outcome::Timeout,
        Check::Registers => {
            let regs = gb.registers();
            if (regs.bc, regs.de, regs.hl) == (0x0305, 0x080D, 0x1522) {
                Outcome::Pass
            } else {
                Outcome::Fail(format!(
                    "BC={:04X} DE={:04X} HL={:04X}",
                    regs.bc, regs.de, regs.hl
                ))
            }
        }
        Check::ResultCode if !finished => Outcome::Timeout,
        Check::ResultCode => match gb.peek(0xA000) {
            0 => Outcome::Pass,
            code => Outcome::Fail(format!("code {code:02X}")),
        },
        Check::Screenshot { until_ld_b_b, .. } if *until_ld_b_b && !finished => Outcome::Timeout,
        Check::Screenshot { .. } => {
            let expected = expected.unwrap();
            if screen.0.len() != expected.len() {
                Outcome::Fail("no frame".into())
            } else {
                let wrong = screen
                    .0
                    .iter()
                    .zip(&expected)
                    .filter(|(pixel, expected)| shade(pixel.to_rgb888().0) != **expected)
                    .count();
                if wrong == 0 {
                    Outcome::Pass
                } else {
                    Outcome::Fail(format!("{wrong} pixels differ"))
                }
            }
        }
    })
}

fn main() -> Result<()> {
    let mut verbose = false;
    let mut save = None;
    let mut compare = None;
    let mut filter = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-v" => verbose = true,
            "--save" => save = Some(args.next().context("--save needs a file")?),
            "--compare" => compare = Some(args.next().context("--compare needs a file")?),
            _ if arg.starts_with('-') => bail!("Unknown option {arg}"),
            _ => filter = Some(arg),
        }
    }

    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../test_roms");
    if !root.exists() {
        bail!(
            "No test ROMs in {}: run `just test_roms` first",
            root.display()
        );
    }
    let name = |test: &Test| {
        test.rom
            .strip_prefix(&root)
            .unwrap()
            .to_string_lossy()
            .into_owned()
    };
    let tests: Vec<Test> = all_tests(&root)
        .into_iter()
        .filter(|test| {
            filter
                .as_ref()
                .is_none_or(|f| name(test).contains(f.as_str()))
        })
        .collect();

    // Keep the panics of crashing tests quiet: they're reported as a result.
    std::panic::set_hook(Box::new(|_| {}));
    let next = AtomicUsize::new(0);
    let outcomes = Mutex::new(vec![None; tests.len()]);
    std::thread::scope(|scope| {
        for _ in 0..std::thread::available_parallelism().map_or(4, |n| n.get()) {
            scope.spawn(|| {
                loop {
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    let Some(test) = tests.get(index) else { break };
                    let outcome =
                        run_test(test).unwrap_or_else(|e| Outcome::Fail(format!("{e:#}")));
                    outcomes.lock().unwrap()[index] = Some(outcome);
                }
            });
        }
    });
    let outcomes: Vec<Outcome> = outcomes
        .into_inner()
        .unwrap()
        .into_iter()
        .flatten()
        .collect();

    let mut suites: Vec<(&str, usize, usize)> = Vec::new();
    for (test, outcome) in tests.iter().zip(&outcomes) {
        if suites.last().is_none_or(|(suite, ..)| *suite != test.suite) {
            suites.push((test.suite, 0, 0));
        }
        let (_, passed, total) = suites.last_mut().unwrap();
        *passed += (*outcome == Outcome::Pass) as usize;
        *total += 1;
        if verbose {
            let details = match outcome {
                Outcome::Fail(details) => format!(" ({details})"),
                _ => String::new(),
            };
            println!("{:7} {}{details}", outcome.kind(), name(test));
        }
    }
    if verbose {
        println!();
    }
    for (suite, passed, total) in &suites {
        println!("{suite:24} {passed:3} / {total:3}");
    }
    let passed = outcomes.iter().filter(|o| **o == Outcome::Pass).count();
    println!("{:24} {passed:3} / {:3}", "total", outcomes.len());

    let results: BTreeMap<String, &str> = tests
        .iter()
        .zip(&outcomes)
        .map(|(test, outcome)| (name(test), outcome.kind()))
        .collect();
    if let Some(file) = compare {
        let previous =
            fs::read_to_string(&file).with_context(|| format!("Failed to read {file}"))?;
        let previous: BTreeMap<&str, &str> = previous
            .lines()
            .filter_map(|line| line.split_once('\t'))
            .map(|(kind, name)| (name, kind))
            .collect();
        println!("\nChanges since {file}:");
        let mut changes = 0;
        for (name, kind) in &results {
            if let Some(before) = previous.get(name.as_str())
                && before != kind
            {
                println!("  {before:7} -> {kind:7} {name}");
                changes += 1;
            }
        }
        if changes == 0 {
            println!("  none");
        }
    }
    if let Some(file) = save {
        let content: String = results
            .iter()
            .map(|(name, kind)| format!("{kind}\t{name}\n"))
            .collect();
        fs::write(&file, content).with_context(|| format!("Failed to write {file}"))?;
    }
    Ok(())
}
