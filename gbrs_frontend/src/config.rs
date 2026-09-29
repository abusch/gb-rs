//! The frontend's settings, read from a TOML file in the user's configuration directory.
//!
//! gb-rs writes to it too, but only to remember the palette last switched to (see
//! [`save_palette`]).
//!
//! Every setting is optional. Directories follow the XDG base directory conventions, on macOS
//! too (like most command-line tools), and the Windows known folders on Windows:
//!
//! ```toml
//! # The palette to start with: a built-in one (green, dmg, pocket, grey) or one from `[palettes]`.
//! # Switching palettes updates it.
//! palette = "pocket"
//! # Battery-backed cartridge RAM (`.sav`). Default: $XDG_DATA_HOME/gbrs/saves
//! save-dir = "~/Games/gb/saves"
//! # Quick save states (`.state`). Default: $XDG_STATE_HOME/gbrs/states
//! state-dir = "~/Games/gb/states"
//! # Screenshots. Default: the current directory
//! screenshot-dir = "~/Pictures"
//!
//! # Extra palettes, from lightest to darkest, which come after the built-in ones when cycling
//! # through them. One named like a built-in palette replaces it.
//! [palettes]
//! blue = ["#e0f0ff", "#80a8d0", "#305078", "#081828"]
//! ```

use std::{
    collections::BTreeMap,
    fs, io,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use etcetera::{BaseStrategy, choose_base_strategy};
use gbrs::{DMG_PALETTES, Rgb555};
use log::info;
use serde::Deserialize;
use toml_edit::{DocumentMut, Item};

const APP_NAME: &str = "gbrs";

/// The contents of the config file, before defaults are applied.
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "kebab-case")]
struct ConfigFile {
    save_dir: Option<PathBuf>,
    state_dir: Option<PathBuf>,
    screenshot_dir: Option<PathBuf>,
    palette: Option<String>,
    palettes: BTreeMap<String, [HexColor; 4]>,
}

/// A colour written as `#rrggbb`.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(try_from = "String")]
struct HexColor(Rgb555);

impl TryFrom<String> for HexColor {
    type Error = String;

    fn try_from(s: String) -> Result<Self, Self::Error> {
        let error = || format!("invalid colour `{s}`, expected `#rrggbb`");
        let hex = s
            .strip_prefix('#')
            .filter(|hex| hex.len() == 6 && hex.bytes().all(|b| b.is_ascii_hexdigit()))
            .ok_or_else(error)?;
        let rgb = u32::from_str_radix(hex, 16).map_err(|_| error())?;
        let [_, r, g, b] = rgb.to_be_bytes();
        Ok(Self(Rgb555::from_rgb888(r, g, b)))
    }
}

/// Colours for the 4 DMG shades, from lightest to darkest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Palette {
    pub name: String,
    pub colors: [Rgb555; 4],
}

#[derive(Debug)]
pub struct Config {
    /// The file this was loaded from, or would have been if it existed.
    pub path: PathBuf,
    /// Where battery-backed cartridge RAM is saved. It's valuable (it's the player's progress)
    /// and compatible with other emulators, so it's data rather than state.
    pub save_dir: PathBuf,
    /// Where quick save states go. Only this version of gb-rs can read them, which makes them
    /// state in XDG terms.
    pub state_dir: PathBuf,
    pub screenshot_dir: PathBuf,
    /// The palettes to cycle through: the built-in ones, then the config file's.
    pub palettes: Vec<Palette>,
    /// The index in `palettes` of the one to start with.
    pub palette: usize,
}

impl Config {
    /// Load the config from `path`, or from the default location if `None`.
    ///
    /// A missing file at the default location just means the defaults are used, but one given
    /// explicitly has to exist.
    pub fn load(path: Option<&Path>) -> Result<Self> {
        let dirs = choose_base_strategy().context("Failed to find the home directory")?;
        let default_path = dirs.config_dir().join(APP_NAME).join("config.toml");
        let path = path.unwrap_or(&default_path);
        let file = match fs::read_to_string(path) {
            Ok(content) => {
                info!("Loading config from {}", path.display());
                toml::from_str(&content)
                    .with_context(|| format!("Invalid config file {}", path.display()))?
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound && path == default_path => {
                ConfigFile::default()
            }
            Err(e) => {
                return Err(e).with_context(|| format!("Failed to read {}", path.display()));
            }
        };
        Self::resolve(file, path, &dirs)
            .with_context(|| format!("Invalid config file {}", path.display()))
    }

    /// Apply the defaults to the settings the file leaves out.
    fn resolve(file: ConfigFile, path: &Path, dirs: &impl BaseStrategy) -> Result<Self> {
        let home = dirs.home_dir();
        let dir = |setting: Option<PathBuf>, default: PathBuf| {
            setting.map_or(default, |path| expand_tilde(&path, home))
        };
        // Windows has no state directory.
        let state_home = dirs.state_dir().unwrap_or_else(|| dirs.data_dir());

        let mut palettes: Vec<Palette> = DMG_PALETTES
            .iter()
            .map(|p| Palette {
                name: p.name.to_owned(),
                colors: p.colors,
            })
            .collect();
        for (name, colors) in file.palettes {
            let colors = colors.map(|HexColor(color)| color);
            match palettes.iter_mut().find(|p| p.name == name) {
                Some(palette) => palette.colors = colors,
                None => palettes.push(Palette { name, colors }),
            }
        }
        let palette = match file.palette {
            None => 0,
            Some(name) => match palettes.iter().position(|p| p.name == name) {
                Some(index) => index,
                None => {
                    let names: Vec<_> = palettes.iter().map(|p| p.name.as_str()).collect();
                    bail!(
                        "Unknown palette `{name}`, expected one of {}",
                        names.join(", ")
                    );
                }
            },
        };

        Ok(Self {
            path: path.to_owned(),
            save_dir: dir(file.save_dir, dirs.data_dir().join(APP_NAME).join("saves")),
            state_dir: dir(file.state_dir, state_home.join(APP_NAME).join("states")),
            screenshot_dir: dir(file.screenshot_dir, PathBuf::from(".")),
            palettes,
            palette,
        })
    }
}

/// Set `palette` to `name` in the config file at `path`, creating it if needed.
///
/// The file is read again first, so that edits made since it was loaded are kept, and only that
/// setting changes: comments and formatting stay as they are.
pub fn save_palette(path: &Path, name: &str) -> Result<()> {
    let content = match fs::read_to_string(path) {
        Ok(content) => content,
        Err(e) if e.kind() == io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e).context("Failed to read the config file"),
    };
    let content = set_palette(&content, name)?;
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).context("Failed to create the config directory")?;
    }
    fs::write(path, content).context("Failed to write the config file")
}

/// Set `palette` to `name` in the TOML document `content`.
fn set_palette(content: &str, name: &str) -> Result<String> {
    let mut doc: DocumentMut = content.parse().context("Failed to parse the config file")?;
    match doc.get_mut("palette").and_then(Item::as_value_mut) {
        // Keep the comments around the old value.
        Some(value) => {
            let decor = value.decor().clone();
            *value = name.into();
            *value.decor_mut() = decor;
        }
        None => {
            doc.insert("palette", toml_edit::value(name));
        }
    }
    Ok(doc.to_string())
}

/// Replace a leading `~` in `path` with the home directory, as a shell would.
fn expand_tilde(path: &Path, home: &Path) -> PathBuf {
    match path.strip_prefix("~") {
        Ok(rest) => home.join(rest),
        Err(_) => path.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use etcetera::base_strategy::Xdg;

    use super::*;

    #[test]
    fn test_expand_tilde() {
        let home = Path::new("/home/me");
        assert_eq!(expand_tilde(Path::new("~"), home), home);
        assert_eq!(
            expand_tilde(Path::new("~/saves"), home),
            Path::new("/home/me/saves")
        );
        assert_eq!(
            expand_tilde(Path::new("~me/saves"), home),
            Path::new("~me/saves")
        );
        assert_eq!(expand_tilde(Path::new("/saves"), home), Path::new("/saves"));
    }

    #[test]
    fn test_resolve() {
        let dirs = Xdg::new().unwrap();
        let file: ConfigFile = toml::from_str(r#"save-dir = "~/saves""#).unwrap();
        let config = Config::resolve(file, Path::new("config.toml"), &dirs).unwrap();
        assert_eq!(config.save_dir, dirs.home_dir().join("saves"));
        assert_eq!(
            config.state_dir,
            dirs.state_dir().unwrap().join("gbrs/states")
        );
        assert_eq!(config.screenshot_dir, Path::new("."));
        assert_eq!(config.palettes.len(), DMG_PALETTES.len());
        assert_eq!(config.palette, 0);
    }

    #[test]
    fn test_palettes() {
        let dirs = Xdg::new().unwrap();
        let file: ConfigFile = toml::from_str(
            r##"
            palette = "blue"
            [palettes]
            blue = ["#ffffff", "#80a8d0", "#305078", "#000000"]
            dmg = ["#ffffff", "#aaaaaa", "#555555", "#000000"]
            "##,
        )
        .unwrap();
        let config = Config::resolve(file, Path::new("config.toml"), &dirs).unwrap();
        // Replacing a built-in palette keeps its place.
        assert_eq!(config.palettes[1].name, "dmg");
        assert_eq!(config.palettes[1].colors, DMG_PALETTES[3].colors);
        let blue = &config.palettes[config.palette];
        assert_eq!(blue.name, "blue");
        assert_eq!(blue.colors[0], Rgb555(0x7fff));
        assert_eq!(blue.colors[3], Rgb555(0));
        assert_eq!(config.palettes.len(), DMG_PALETTES.len() + 1);
    }

    #[test]
    fn test_invalid_palettes() {
        let dirs = Xdg::new().unwrap();
        let file: ConfigFile = toml::from_str(r#"palette = "nope""#).unwrap();
        assert!(Config::resolve(file, Path::new("config.toml"), &dirs).is_err());
        for colors in [
            r##"["#fff", "#aaaaaa", "#555555", "#000000"]"##,
            r##"["ffffff", "#aaaaaa", "#555555", "#000000"]"##,
            r##"["#gggggg", "#aaaaaa", "#555555", "#000000"]"##,
            r##"["#+fffff", "#aaaaaa", "#555555", "#000000"]"##,
            r##"["#ffffff", "#aaaaaa", "#555555"]"##,
        ] {
            let toml = format!("palettes.bad = {colors}");
            assert!(toml::from_str::<ConfigFile>(&toml).is_err(), "{colors}");
        }
    }

    #[test]
    fn test_set_palette() {
        assert_eq!(set_palette("", "dmg").unwrap(), "palette = \"dmg\"\n");

        // Only the palette changes, and it goes before the tables.
        let content = r##"# My settings
save-dir = "~/saves" # synced

[palettes]
blue = ["#e0f0ff", "#80a8d0", "#305078", "#081828"]
"##;
        let expected = r##"# My settings
save-dir = "~/saves" # synced
palette = "blue"

[palettes]
blue = ["#e0f0ff", "#80a8d0", "#305078", "#081828"]
"##;
        assert_eq!(set_palette(content, "blue").unwrap(), expected);

        // Comments around an existing value are kept.
        let content = "# Start with\npalette   =   'pocket'   # my favourite\nsave-dir = \"x\"\n";
        let expected = "# Start with\npalette   =   \"grey\"   # my favourite\nsave-dir = \"x\"\n";
        assert_eq!(set_palette(content, "grey").unwrap(), expected);

        // A file that isn't valid TOML is left alone.
        assert!(set_palette("palette = ", "grey").is_err());
    }

    #[test]
    fn test_save_palette() {
        let dir = std::env::temp_dir().join(format!("gbrs-config-test-{}", std::process::id()));
        let path = dir.join("gbrs").join("config.toml");
        save_palette(&path, "pocket").unwrap();
        save_palette(&path, "grey").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "palette = \"grey\"\n");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn test_unknown_setting() {
        assert!(toml::from_str::<ConfigFile>(r#"save_dir = "saves""#).is_err());
    }
}
