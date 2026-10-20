//! A Godot game's export presets, read for where each export goes and the custom features it is
//! made with, so that a build marked as the demo's ships only as the demo.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::scripts;

/// The file Godot keeps a project's export presets in, beside `project.godot`.
pub const FILE: &str = "export_presets.cfg";

/// One export preset.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Preset {
    /// Its name in Godot's export dialog.
    pub name: String,
    /// Its custom features, which the exported game tells with `OS.has_feature`.
    pub features: Vec<String>,
    /// The file it exports, from the project's folder.
    pub export: PathBuf,
}

/// Every preset in the project at `root` that exports somewhere; none when it has no presets.
///
/// # Errors
///
/// When the presets file is there and cannot be read.
pub fn read(root: &Path) -> Result<Vec<Preset>, String> {
    let path = root.join(FILE);
    match fs::read_to_string(&path) {
        Ok(text) => Ok(parse(root, &text)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(format!("{}: {error}", path.display())),
    }
}

/// The presets in `text`, an export presets file of the project at `root`.
#[must_use]
pub fn parse(root: &Path, text: &str) -> Vec<Preset> {
    let mut presets = Vec::new();
    let mut current: Option<(Option<String>, Vec<String>, Option<String>)> = None;
    let mut finish = |read: Option<(Option<String>, Vec<String>, Option<String>)>| {
        if let Some((name, features, Some(export))) = read
            && !export.is_empty()
        {
            presets.push(Preset {
                name: name.unwrap_or_default(),
                features,
                export: scripts::resolve(root, &export),
            });
        }
    };
    for line in text.lines().map(str::trim) {
        if let Some(section) = line
            .strip_prefix('[')
            .and_then(|rest| rest.strip_suffix(']'))
        {
            finish(current.take());
            // A preset's options are a section of their own, `[preset.0.options]`.
            let preset = section.strip_prefix("preset.").is_some_and(|index| {
                !index.is_empty() && index.bytes().all(|digit| digit.is_ascii_digit())
            });
            if preset {
                current = Some((None, Vec::new(), None));
            }
            continue;
        }
        let Some((name, features, export)) = current.as_mut() else {
            continue;
        };
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let Some(value) = quoted(value.trim()) else {
            continue;
        };
        match key.trim() {
            "name" => *name = Some(value),
            // Godot splits the features at commas and trims each.
            "custom_features" => {
                *features = value
                    .split(',')
                    .map(str::trim)
                    .filter(|feature| !feature.is_empty())
                    .map(str::to_owned)
                    .collect();
            }
            "export_path" => *export = Some(value),
            _ => {}
        }
    }
    finish(current);
    presets
}

/// The text of a value Godot wrote as a quoted string, or nothing for any other value.
fn quoted(value: &str) -> Option<String> {
    let inner = value.strip_prefix('"')?.strip_suffix('"')?;
    let mut text = String::with_capacity(inner.len());
    let mut characters = inner.chars();
    while let Some(character) = characters.next() {
        if character == '\\' {
            match characters.next()? {
                'n' => text.push('\n'),
                't' => text.push('\t'),
                other => text.push(other),
            }
        } else {
            text.push(character);
        }
    }
    Some(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two presets as Godot writes them, the second for the demo, each followed by its options.
    const PRESETS: &str = "[preset.0]\n\nname=\"Windows Desktop\"\nplatform=\"Windows Desktop\"\n\
        runnable=true\ncustom_features=\"\"\nexport_filter=\"all_resources\"\n\
        export_path=\"export/windows/sample.exe\"\nseed=0\n\n[preset.0.options]\n\n\
        custom_template/debug=\"\"\nexport_path=\"elsewhere.exe\"\n\n\
        [preset.1]\n\nname=\"Windows \\\"Demo\\\"\"\nplatform=\"Windows Desktop\"\n\
        custom_features=\"demo, trial\"\nexport_path=\"export/demo/windows/sample.exe\"\n\n\
        [preset.1.options]\n\n[preset.2]\n\nname=\"Linux\"\ncustom_features=\"\"\n\
        export_path=\"\"\n";

    #[test]
    fn each_preset_s_name_features_and_export_are_read_and_its_options_are_not() {
        let root = Path::new("game");
        assert_eq!(
            parse(root, PRESETS),
            [
                Preset {
                    name: "Windows Desktop".to_owned(),
                    features: Vec::new(),
                    export: scripts::resolve(root, "export/windows/sample.exe"),
                },
                Preset {
                    name: "Windows \"Demo\"".to_owned(),
                    features: vec!["demo".to_owned(), "trial".to_owned()],
                    export: scripts::resolve(root, "export/demo/windows/sample.exe"),
                },
            ],
            "a preset exporting nowhere yet is left out"
        );
    }

    #[test]
    fn a_project_without_presets_has_none_and_one_unreadable_is_named() {
        let folder = tempfile::tempdir().unwrap();
        assert_eq!(read(folder.path()), Ok(Vec::new()));
        fs::create_dir_all(folder.path().join(FILE)).unwrap();
        assert!(
            read(folder.path())
                .unwrap_err()
                .starts_with(&folder.path().join(FILE).display().to_string())
        );
    }

    #[test]
    fn only_a_quoted_value_is_text_with_its_escapes_undone() {
        assert_eq!(quoted("\"a\\\\b\\nc\\td\"").as_deref(), Some("a\\b\nc\td"));
        assert_eq!(quoted("true"), None);
        assert_eq!(quoted("\"open"), None);
        assert_eq!(quoted("\"trailing\\\""), None);
    }
}
