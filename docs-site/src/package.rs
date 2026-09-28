//! What each release archive carries beside the program: a man page for every command and tab
//! completion for every shell clap knows, made from the same definitions as `--help`.

use std::fs;
use std::path::Path;

use clap::CommandFactory as _;
use clap::ValueEnum as _;
use clap_complete::Shell;
use steamship::cli::Cli;

/// Writes `man/` and `completions/` into `out`, the folder an archive is packed from.
///
/// # Errors
///
/// When either folder or any file in them cannot be written.
pub fn write(out: &Path) -> Result<(), String> {
    let man = out.join("man");
    fs::create_dir_all(&man).map_err(|error| format!("{}: {error}", man.display()))?;
    clap_mangen::generate_to(Cli::command(), &man)
        .map_err(|error| format!("{}: {error}", man.display()))?;
    let completions = out.join("completions");
    fs::create_dir_all(&completions)
        .map_err(|error| format!("{}: {error}", completions.display()))?;
    for &shell in Shell::value_variants() {
        let _written =
            clap_complete::generate_to(shell, &mut Cli::command(), "steamship", &completions)
                .map_err(|error| format!("{}: {error}", completions.display()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{env, process};

    use super::*;

    #[test]
    fn every_command_gets_a_man_page_and_every_shell_its_completion() {
        let out = env::temp_dir().join(format!("steamship-package-{}", process::id()));
        let written = write(&out);
        let listed = |folder: &str| -> Vec<String> {
            let mut names: Vec<String> = fs::read_dir(out.join(folder))
                .map(|entries| {
                    entries
                        .filter_map(Result::ok)
                        .map(|entry| entry.file_name().to_string_lossy().into_owned())
                        .collect()
                })
                .unwrap_or_default();
            names.sort();
            names
        };
        let (man, completions) = (listed("man"), listed("completions"));
        let page =
            fs::read_to_string(out.join("man").join("steamship-upload.1")).unwrap_or_default();
        fs::remove_dir_all(&out).unwrap_or_default();
        assert_eq!(written, Ok(()));
        for command in [
            "steamship.1",
            "steamship-upload.1",
            "steamship-workshop.1",
            "steamship-completions.1",
        ] {
            assert!(man.contains(&command.to_owned()), "{command} in {man:?}");
        }
        assert_eq!(
            completions,
            [
                "_steamship",
                "_steamship.ps1",
                "steamship.bash",
                "steamship.elv",
                "steamship.fish"
            ]
        );
        assert!(
            page.contains("\\-\\-version"),
            "the upload page lists its options: {page}"
        );
    }
}
