//! A game's folder of scripts and content, at a path with spaces and brackets the way a Steam
//! library's or a build worktree's can have.

use std::fs;
use std::path::PathBuf;

/// A game's folder, in a temporary folder of its own.
#[derive(Debug)]
pub struct Project {
    /// Deletes both folders when the project is dropped.
    pub temp: tempfile::TempDir,
    pub root: PathBuf,
}

impl Default for Project {
    fn default() -> Self {
        Self::new()
    }
}

impl Project {
    /// The game's folder, `gate (x86) 1a2b`, made in a temporary folder.
    ///
    /// # Panics
    ///
    /// When either folder cannot be made.
    #[must_use]
    pub fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("gate (x86) 1a2b");
        fs::create_dir_all(&root).unwrap();
        Self { temp, root }
    }

    /// Writes `contents` at `relative`, making its folders, and answers where.
    ///
    /// # Panics
    ///
    /// When the file or one of its folders cannot be written.
    #[must_use]
    pub fn file(&self, relative: &str, contents: &[u8]) -> PathBuf {
        let path = self.root.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, contents).unwrap();
        path
    }

    /// The same, for a file nothing refers to by path afterwards.
    pub fn put(&self, relative: &str, contents: &[u8]) {
        drop(self.file(relative, contents));
    }

    /// Two depots in files of their own, content for both, and a debug file to exclude.
    #[must_use]
    pub fn shipping(app_extra: &str) -> (Self, PathBuf) {
        let project = Self::new();
        project.put("export/windows/game.exe", b"MZ");
        project.put("export/windows/game.pck", b"pack");
        project.put("export/windows/game.pdb", b"symbols");
        project.put("export/linux/game.x86_64", b"not really elf");
        project.put("export/linux/data/game.pck", b"pack");
        project.put(
            "steam/depot_windows.vdf",
            br#""DepotBuild"
{
	"DepotID" "1001"
	"FileMapping" { "LocalPath" "windows\*" "DepotPath" "." "Recursive" "1" }
	"FileExclusion" "*.pdb"
}
"#,
        );
        project.put(
            "steam/depot_linux.vdf",
            br#""DepotBuild"
{
	"DepotID" "1002"
	"FileMapping" { "LocalPath" "linux\*" "DepotPath" "." "Recursive" "1" }
}
"#,
        );
        let app = project.file(
            "steam/app_build.vdf",
            format!(
                r#""AppBuild"
{{
	"AppID" "1000"
	"ContentRoot" "..\export\"
	"SetLive" "testing"
	{app_extra}
	"Depots"
	{{
		"1001" "depot_windows.vdf"
		"1002" "depot_linux.vdf"
	}}
}}
"#
            )
            .as_bytes(),
        );
        (project, app)
    }
}
