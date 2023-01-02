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
