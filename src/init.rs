//! Starter build scripts for `steamship init`: an app build script and a depot build script for
//! each depot, in Valve's own format and commented, for `check` and `upload` to read as they are.

use std::fmt::{self, Write as _};
use std::path::{Component, Path};

/// A depot to write a script for: its ID, and the folder its files ship from, relative to the
/// folder `init` runs in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Depot {
    pub id: u32,
    pub folder: String,
}

/// Reads `ID=FOLDER`, as `--depot` takes it.
///
/// # Errors
///
/// When there is no `=`, the ID is not a number, or the folder is empty.
pub fn depot(text: &str) -> Result<Depot, String> {
    let (id, folder) = text
        .split_once('=')
        .ok_or_else(|| format!("\"{text}\" is not ID=FOLDER, such as 1001=build/windows"))?;
    let id = id
        .trim()
        .parse::<u32>()
        .map_err(|_not_a_number| format!("\"{id}\" is not a depot ID"))?;
    let folder = folder.trim().replace('\\', "/");
    if folder.is_empty() {
        return Err(format!("depot {id} names no folder"));
    }
    Ok(Depot { id, folder })
}

/// The scripts for `app` and its `depots`, by file name, to be written into `folder`, which is
/// relative to where the depots' folders are.
///
/// # Errors
///
/// When `folder` climbs out of where it is written from, which the scripts could not name a way
/// back from.
pub fn scripts(app: u32, depots: &[Depot], folder: &Path) -> Result<Vec<(String, String)>, String> {
    let mut back = String::new();
    for part in folder.components() {
        match part {
            Component::Normal(_) => back.push_str("../"),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(format!(
                    "{} is not a folder inside this one, which the scripts could find their way \
                     back from",
                    folder.display()
                ));
            }
        }
    }
    if back.is_empty() {
        back.push_str("./");
    }
    let listed = depots.iter().fold(String::new(), |mut listed, depot| {
        let _said: fmt::Result = writeln!(listed, "\t\t\"{0}\" \"depot_build_{0}.vdf\"", depot.id);
        listed
    });
    let app_script = format!(
        "// The app build script steamcmd runs, written by steamship init. Check it with\n\
         // steamship check, and upload it with steamship upload.\n\
         \"AppBuild\"\n\
         {{\n\
         \t\"AppID\" \"{app}\"\n\
         \t// Where the depots' folders are found from, relative to this file.\n\
         \t\"ContentRoot\" \"{back}\"\n\
         \t// The branch to set each build live on once it is uploaded. Not \"default\", which\n\
         \t// only the Steamworks site may set live.\n\
         \t// \"SetLive\" \"beta\"\n\
         \t\"Depots\"\n\
         \t{{\n\
         {listed}\
         \t}}\n\
         }}\n"
    );
    let mut written = vec![("app_build.vdf".to_owned(), app_script)];
    for depot in depots {
        let depot_script = format!(
            "// Depot {id}'s build script, written by steamship init.\n\
             \"DepotBuild\"\n\
             {{\n\
             \t\"DepotID\" \"{id}\"\n\
             \t// The folder whose files ship in this depot, from the app's ContentRoot.\n\
             \t\"ContentRoot\" \"{folder}\"\n\
             \t// Every file in it, folders and all, at the root of the installed game.\n\
             \t\"FileMapping\"\n\
             \t{{\n\
             \t\t\"LocalPath\" \"*\"\n\
             \t\t\"DepotPath\" \".\"\n\
             \t\t\"Recursive\" \"1\"\n\
             \t}}\n\
             \t// Debug symbols stay out of what players download.\n\
             \t\"FileExclusion\" \"*.pdb\"\n\
             }}\n",
            id = depot.id,
            folder = depot.folder
        );
        written.push((format!("depot_build_{}.vdf", depot.id), depot_script));
    }
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vdf;

    #[test]
    fn a_depot_is_read_from_id_and_folder() {
        assert_eq!(
            depot("1001=build\\windows"),
            Ok(Depot {
                id: 1001,
                folder: "build/windows".to_owned()
            })
        );
        assert_eq!(
            depot("1001"),
            Err("\"1001\" is not ID=FOLDER, such as 1001=build/windows".to_owned())
        );
        assert_eq!(depot("x=build"), Err("\"x\" is not a depot ID".to_owned()));
        assert_eq!(
            depot("1001= "),
            Err("depot 1001 names no folder".to_owned())
        );
    }

    #[test]
    fn the_scripts_are_valves_format_and_find_the_depots_from_their_folder() {
        let depots = [
            Depot {
                id: 1001,
                folder: "build/windows".to_owned(),
            },
            Depot {
                id: 1002,
                folder: "build/linux".to_owned(),
            },
        ];
        let written = scripts(1000, &depots, Path::new("steam")).unwrap();
        let names: Vec<&str> = written.iter().map(|(name, _)| name.as_str()).collect();
        assert_eq!(
            names,
            [
                "app_build.vdf",
                "depot_build_1001.vdf",
                "depot_build_1002.vdf"
            ]
        );
        let text = |index: usize| {
            written
                .get(index)
                .ok_or("no such script")
                .unwrap()
                .1
                .clone()
        };
        let app = vdf::parse(&text(0)).unwrap();
        let app = app.block("AppBuild").unwrap();
        assert_eq!(app.text("AppID"), Some("1000"));
        assert_eq!(app.text("ContentRoot"), Some("../"));
        assert_eq!(app.text("SetLive"), None, "left for the author to choose");
        let listed = app.block("Depots").unwrap();
        assert_eq!(listed.text("1002"), Some("depot_build_1002.vdf"));
        let depot = vdf::parse(&text(2)).unwrap();
        let depot = depot.block("DepotBuild").unwrap();
        assert_eq!(depot.text("ContentRoot"), Some("build/linux"));
        assert_eq!(depot.text("FileExclusion"), Some("*.pdb"));
    }

    #[test]
    fn the_way_back_counts_the_folders_the_scripts_are_in() {
        let one = [Depot {
            id: 481,
            folder: "build".to_owned(),
        }];
        let content = |folder: &str| {
            let written = scripts(480, &one, Path::new(folder)).unwrap();
            let app = vdf::parse(&written.first().unwrap().1).unwrap();
            app.block("AppBuild")
                .unwrap()
                .text("ContentRoot")
                .unwrap()
                .to_owned()
        };
        assert_eq!(content("steam"), "../");
        assert_eq!(content("tools/steam"), "../../");
        assert_eq!(content("."), "./");
        assert_eq!(
            scripts(480, &one, Path::new("../steam")).unwrap_err(),
            "../steam is not a folder inside this one, which the scripts could find their way \
             back from"
        );
    }
}
