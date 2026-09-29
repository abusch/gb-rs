tmpdir := "/tmp"
test_roms_version := "v5.1"
test_roms_file := tmpdir / "test_roms_" + test_roms_version + ".zip"

# Run emulator with given ROM
run romfile:
  cargo run -q --release -- {{romfile}}

# Download test ROMs
test_roms:
  curl -sSL https://github.com/c-sp/gameboy-test-roms/releases/download/v5.1/game-boy-test-roms-{{test_roms_version}}.zip --output {{test_roms_file}}
  unzip {{test_roms_file}} -d test_roms
  rm {{test_roms_file}}

# Run the test ROMs and report which pass (options: -v, --save FILE, --compare FILE, FILTER)
rom-tests *args:
  cargo run -q --release --example test_roms -p gbrs -- {{args}}

# Cross-build the libretro core (and the headless benchmark) for the Miyoo Mini (Plus) into target/miyoo/ (needs zig and cargo-zigbuild)
miyoo:
  RUSTFLAGS="-C target-cpu=cortex-a7" cargo zigbuild --profile handheld --target armv7-unknown-linux-gnueabihf.2.17 -p gbrs_libretro
  RUSTFLAGS="-C target-cpu=cortex-a7" cargo zigbuild --profile handheld --target armv7-unknown-linux-gnueabihf.2.17 -p gbrs --example headless
  mkdir -p target/miyoo
  cp target/armv7-unknown-linux-gnueabihf/handheld/libgbrs_libretro.so target/miyoo/gbrs_libretro.so
  cp target/armv7-unknown-linux-gnueabihf/handheld/examples/headless gbrs_libretro/gbrs_libretro.info target/miyoo/

# Check that the working copy's emulation output matches revision REV's on every ROM (set GBRS_ROMSET to a directory of zipped games to include them)
compare-output rev="@-" frames="1200":
  scripts/compare-output.sh {{rev}} {{frames}}
