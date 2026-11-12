//! `steamship achievements`.

use std::fs;
use std::io;
use std::iter;
use std::path::{Path, PathBuf};
use std::time::Duration;

use steamship::webapi::STAND_IN;

use crate::{
    INN_GERMAN, INN_RECORDS, KEY, failure, icon_png, in_game, in_game_with, png_header,
    pushed_game, refusing_web_api, steamship, steamship_in, summary_of, web_api, web_api_variables,
};

/// `GetSchemaForGame` as Steam answered it for Spacewar, trimmed to two achievements.
const SCHEMA: &str = r#"{"game": {"gameName": "Spacewar", "availableGameStats": {"achievements": [
    {"name": "ACH_WIN_ONE_GAME", "displayName": "Winner", "hidden": 0,
     "description": "Win one game.", "icon": "https://cdn/w.jpg", "icongray": "https://cdn/w_bw.jpg"},
    {"name": "ACH_TRAVEL_FAR_SINGLE", "displayName": "Orbiter", "hidden": 1,
     "description": "Travel 500 feet in one life.", "icon": "https://cdn/o.jpg",
     "icongray": "https://cdn/o_bw.jpg"}
]}}}"#;

/// `GetGlobalAchievementPercentagesForApp` as Steam answered it for Team Fortress 2, its two
/// achievements named as Spacewar's are.
const SHARES: &str = r#"{"achievementpercentages":{"achievements":[
    {"name":"ACH_WIN_ONE_GAME","percent":"49.6"},{"name":"ACH_TRAVEL_FAR_SINGLE","percent":"3.1"}
]}}"#;

/// An achievements file for Spacewar, the second achievement's description as `orbiter` says.
fn achievements_file(folder: &Path, orbiter: &str) -> String {
    let path = folder.join("achievements.json");
    fs::write(
        &path,
        format!(
            r#"{{"app": 480, "achievements": [
                {{"api_name": "ACH_WIN_ONE_GAME", "name": "Winner", "description": "Win one game."}},
                {{"api_name": "ACH_TRAVEL_FAR_SINGLE", "name": "Orbiter", "description": "{orbiter}",
                  "hidden": true, "glyph": "rocket"}}
            ]}}"#
        ),
    )
    .unwrap();
    path.to_string_lossy().into_owned()
}

#[test]
fn achievements_are_listed_and_checked_against_a_file_by_api_name() {
    let home = tempfile::tempdir().unwrap();
    let (host, requests) = web_api(vec![
        ("200 OK", SCHEMA),
        ("200 OK", SHARES),
        ("200 OK", SCHEMA),
        ("200 OK", SCHEMA),
        ("500 Internal Server Error", ""),
    ]);
    let set = web_api_variables(&host);

    let (code, listed, _) = steamship(&["achievements", "480"], Some(home.path()), &set);
    assert_eq!(code, Some(0_i32), "{listed}");
    assert!(
        listed.contains("  steam     \u{2713} 2 achievements on Steam\n")
            && listed
                .contains("  steam     \u{2713} shares of 2 achievements and totals of 0 stats\n"),
        "{listed}"
    );
    assert!(
        listed.contains("  ACH_WIN_ONE_GAME      Winner, 49.6 % of players: Win one game.\n"),
        "{listed}"
    );
    assert!(
        listed.contains(
            "  ACH_TRAVEL_FAR_SINGLE Orbiter, hidden, 3.1 % of players: Travel 500 feet in one \
             life.\n"
        ),
        "{listed}"
    );
    let asked = requests.recv_timeout(Duration::from_secs(10)).unwrap();
    assert!(
        asked.starts_with("GET /ISteamUserStats/GetSchemaForGame/v2/?")
            && asked.contains("appid=480")
            && asked.contains("l=english"),
        "{asked}"
    );
    let shared = requests.recv_timeout(Duration::from_secs(10)).unwrap();
    assert!(
        shared.starts_with("GET /ISteamUserStats/GetGlobalAchievementPercentagesForApp/v2/?")
            && shared.contains("gameid=480"),
        "{shared}"
    );

    let files = tempfile::tempdir().unwrap();
    let matching = achievements_file(files.path(), "Travel 500 feet in one life.");
    let (matched, said, _) = steamship(
        &["achievements", "480", "--check", &matching],
        Some(home.path()),
        &set,
    );
    assert_eq!(matched, Some(0_i32), "{said}");
    let fresh = |request: &str| {
        let (_, after) = request.split_once("steamship=").unwrap();
        after.split([' ', '&']).next().unwrap().to_owned()
    };
    let again = requests.recv_timeout(Duration::from_secs(10)).unwrap();
    assert_ne!(
        fresh(&asked),
        fresh(&again),
        "each ask is one Steam's cache has not answered"
    );
    assert!(said.contains("2 achievements\n"), "{said}");
    assert!(
        said.contains("Steam's achievements match the file\n"),
        "{said}"
    );

    let drifted = achievements_file(files.path(), "Go far.");
    let (refused, _, why) = steamship(
        &["achievements", "480", "--check", &drifted],
        Some(home.path()),
        &set,
    );
    assert_eq!(refused, Some(2_i32), "{why}");
    assert!(
        why.contains(
            "ACH_TRAVEL_FAR_SINGLE: the description is \"Travel 500 feet in one life.\" on Steam \
             and \"Go far.\" in the file"
        ),
        "{why}"
    );
    assert!(
        why.ends_with("1 of 2 achievements match: 1 differs\n"),
        "{why}"
    );

    // Steam failing is a failure, not drift.
    let (failed, _, failure) = steamship(
        &["achievements", "480", "--check", &matching],
        Some(home.path()),
        &set,
    );
    assert_eq!(failed, Some(1_i32), "{failure}");
    assert!(failure.contains("Steam answered HTTP 500"), "{failure}");
}

/// `GetSchemaForGame` for Tidal Mill Inn's first sluice in English, as Steam answered it, its
/// icons trimmed.
const CELLAR: &str = r#"{"game": {"gameName": "Tidal Mill Inn", "availableGameStats": {
    "achievements": [{"name": "FIRST_ANCHOR", "displayName": "Down by the Wheel", "hidden": 0,
     "description": "Drain a mill's first sluice.", "icon": "https://cdn/c.jpg",
     "icongray": "https://cdn/c_bw.jpg"}]}}}"#;

/// The same in German.
const KELLER: &str = r#"{"game": {"gameName": "Tidal Mill Inn", "availableGameStats": {
    "achievements": [{"name": "FIRST_ANCHOR", "displayName": "Hinab zum Rad", "hidden": 0,
     "description": "Gib die erste Schleuse eines Mahlwerks frei.", "icon": "https://cdn/c.jpg",
     "icongray": "https://cdn/c_bw.jpg"}]}}}"#;

#[test]
fn achievements_are_checked_in_each_language_a_localisation_has() {
    let home = tempfile::tempdir().unwrap();
    let (host, requests) = web_api(vec![
        ("200 OK", CELLAR),
        ("200 OK", KELLER),
        ("200 OK", CELLAR),
        ("200 OK", CELLAR),
        ("200 OK", CELLAR),
        ("500 Internal Server Error", ""),
        ("200 OK", CELLAR),
    ]);
    let set = web_api_variables(&host);
    let files = tempfile::tempdir().unwrap();
    let records = files.path().join("achievements.json");
    fs::write(
        &records,
        r#"[{"type": "achievement", "steam": "FIRST_ANCHOR", "name": "Down by the Wheel",
            "description": "Drain a mill's first sluice."}]"#,
    )
    .unwrap();
    let localisation = files.path().join("localisation.csv");
    fs::write(
        &localisation,
        "api_name (required; must match an api_name from the definitions CSV),\
         field (name or description),english,german\n\
         FIRST_ANCHOR,name,Down by the Wheel,Hinab zum Rad\n\
         FIRST_ANCHOR,description,Drain a mill's first sluice.,Gib die erste Schleuse eines Mahlwerks frei.\n",
    )
    .unwrap();
    let args = [
        "achievements",
        "1000050",
        "--check",
        &records.to_string_lossy(),
        &localisation.to_string_lossy(),
    ];

    let (code, said, _) = steamship(&args, Some(home.path()), &set);
    assert_eq!(code, Some(0_i32), "{said}");
    assert!(
        said.contains("localisation.csv, 1 achievement in 2 languages\n"),
        "{said}"
    );
    assert!(
        said.contains("  steam     \u{2713} in 1 language as well\n"),
        "{said}"
    );
    assert!(
        said.contains("Steam's achievements and stats match the files\n"),
        "{said}"
    );
    let first = requests.recv_timeout(Duration::from_secs(10)).unwrap();
    let second = requests.recv_timeout(Duration::from_secs(10)).unwrap();
    assert!(first.contains("l=english"), "{first}");
    assert!(second.contains("l=german"), "{second}");

    // Steam answers a language it holds nothing in with the English texts.
    let (refused, _, why) = steamship(&args, Some(home.path()), &set);
    assert_eq!(refused, Some(2_i32), "{why}");
    assert!(
        why.contains(
            "Steam holds nothing in german, so shows 1 achievement in English. Import the \
             localisation in Steamworks, under Stats & Achievements, and publish\n"
        ),
        "{why}"
    );
    assert!(
        why.ends_with("0 of 1 achievements match: 1 differs\n"),
        "{why}"
    );

    // Steam failing in one of the languages is a failure, not drift.
    let (failed, _, failure) = steamship(&args, Some(home.path()), &set);
    assert_eq!(failed, Some(1_i32), "{failure}");
    assert!(failure.contains("Steam answered HTTP 500"), "{failure}");

    fs::write(
        &localisation,
        "api_name,field,english\n\
         FIRST_ANCHOR,name,Down by the Wheel\n\
         FIRST_ANCHOR,description,Drain a mill's first sluice.\n",
    )
    .unwrap();
    let (alone, english, _) = steamship(&args, Some(home.path()), &set);
    assert_eq!(alone, Some(0_i32), "{english}");
    assert!(
        !english.contains("as well"),
        "English alone is what Steam answered first: {english}"
    );

    fs::write(&localisation, "api_name,name,german\n").unwrap();
    let (unread, _, why_not) = steamship(&args, Some(home.path()), &set);
    assert_eq!(unread, Some(2_i32), "{why_not}");
    assert!(
        why_not.contains(
            "localisation.csv: is not the localisation Steamworks' achievement editor exports"
        ),
        "{why_not}"
    );
}

/// A games-kit mod holding [`INN_RECORDS`] under its `data` folder and their icons, with the
/// German catalogue beside it, as the game keeps them.
fn inn_mod() -> tempfile::TempDir {
    let game = tempfile::tempdir().unwrap();
    let base = game.path().join("mods").join("base");
    fs::create_dir_all(base.join("data")).unwrap();
    fs::create_dir_all(base.join("icons")).unwrap();
    fs::write(base.join("manifest.json"), "{}").unwrap();
    fs::write(base.join("data").join("achievements.json"), INN_RECORDS).unwrap();
    for (icon, size, shade) in [
        ("five_levels", 256, 200),
        ("first_anchor", 256, 120),
        ("first_anchor_locked", 64, 60),
    ] {
        fs::write(
            base.join("icons").join(format!("{icon}.png")),
            icon_png(size, shade),
        )
        .unwrap();
    }
    fs::create_dir_all(game.path().join("locale")).unwrap();
    fs::write(game.path().join("locale").join("de.po"), INN_GERMAN).unwrap();
    game
}

/// `steamship achievements --write` run on [`inn_mod`]'s records and catalogue in `game`, into
/// its `steam/achievements`, or checking it is `unchanged`.
fn write_inn(game: &Path, unchanged: bool) -> (Option<i32>, String, String) {
    let named = [
        game.join("mods/base/data/achievements.json"),
        game.join("locale/de.po"),
        game.join("steam").join("achievements"),
    ]
    .map(|path| path.to_string_lossy().into_owned());
    let [records, german, out] = named.each_ref().map(String::as_str);
    let mut args = vec![
        "achievements",
        "1000050",
        "--write",
        records,
        german,
        "--out",
        out,
    ];
    if unchanged {
        args.push("--unchanged");
    }
    steamship(&args, Some(game), &[("STEAMSHIP_NO_UPDATE_CHECK", "1")])
}

#[test]
fn the_achievement_editors_files_are_written_from_records_catalogues_and_icons() {
    let game = inn_mod();
    let out = game.path().join("steam").join("achievements");
    let (missing, _, why) = write_inn(game.path(), true);
    assert_eq!(missing, Some(2_i32), "{why}");
    assert!(
        why.contains("1000050-achievements-definitions.csv is missing\n"),
        "{why}"
    );

    let (code, said, noted) = write_inn(game.path(), false);
    assert_eq!(code, Some(0_i32), "{said}{noted}");
    assert!(said.contains("de.po, german\n"), "{said}");
    assert!(!said.contains("unchecked"), "{said}");
    assert!(
        said.contains("holds the 6 files the achievement editor imports\n"),
        "{said}"
    );
    assert!(
        said.contains("drop every CSV and PNG file in ")
            && said.contains(
                " and Save imported data, then stay on the page until Pending Changes is gone. \
                 Then drop 1000050-achievements-localization.csv again on its own the same way"
            )
            && said.contains("Publish, and run this again with --imported\n"),
        "{said}"
    );
    assert!(
        noted.contains("each set has to be made in the achievement editor before the import\n"),
        "{noted}"
    );
    assert!(
        noted.contains("no locked icon for 1 achievement. Tick Automatically generate"),
        "{noted}"
    );
    let read = |name: &str| fs::read_to_string(out.join(name)).unwrap();
    assert!(read("1000050-achievements-definitions.csv").ends_with(
        "\nFIVE_LEVELS,,Client,false,false,floors,0,5\nFIRST_ANCHOR,1,Client,false,false,,0,0\n"
    ));
    assert!(
        read("1000050-achievements-localization.csv")
            .contains("\nFIRST_ANCHOR,name,Down by the Wheel,Hinab zum Rad\n")
    );
    assert_eq!(
        read("1000050-achievement-sets-localization.csv"),
        "setid (required; the id of a set that already exists),english,german\n\
         1,The Cellar,Der Keller\n"
    );
    assert_eq!(
        fs::read(out.join("FIRST_ANCHOR_UNACHIEVED.png")).unwrap(),
        icon_png(64, 60),
        "64 pixels is the least the editor takes"
    );
    assert!(!noted.contains("no icon for"), "{noted}");
    assert!(out.join("FIVE_LEVELS_ACHIEVED.png").is_file());

    let (same, unchanged, _) = write_inn(game.path(), true);
    assert_eq!(same, Some(0_i32), "{unchanged}");
    assert!(unchanged.contains("the 6 files in "), "{unchanged}");
}

#[test]
fn an_achievement_unlocked_by_the_first_floor_built_is_written_without_a_bar_and_its_old_one_named()
{
    let game = inn_mod();
    let out = game.path().join("steam").join("achievements");
    let (first, said, why) = write_inn(game.path(), false);
    assert_eq!(first, Some(0_i32), "{said}{why}");
    fs::write(
        game.path().join("mods/base/data/achievements.json"),
        INN_RECORDS.replace(r#""at": 5"#, r#""at": 1"#),
    )
    .unwrap();

    let (stale, _, why_stale) = write_inn(game.path(), true);
    assert_eq!(stale, Some(2_i32), "{why_stale}");
    assert!(
        why_stale
            .contains("1000050-achievements-definitions.csv differs from what the files make\n"),
        "{why_stale}"
    );
    let (rewritten, again, noted) = write_inn(game.path(), false);
    assert_eq!(rewritten, Some(0_i32), "{again}{noted}");
    assert!(
        fs::read_to_string(out.join("1000050-achievements-definitions.csv"))
            .unwrap()
            .contains("\nFIVE_LEVELS,,Client,false,false,,0,0\n"),
        "{again}"
    );
    assert!(
        noted.contains(
            "FIVE_LEVELS's progress bar goes from floors 0 - 5 to no progress bar, which Steam's \
             Web API cannot show. Look at it in the editor once imported, and set it there by \
             hand if it has not moved, before publishing\n"
        ),
        "{noted}"
    );
    assert!(
        !noted.contains("each stat a progress bar counts"),
        "no bar is left to need its stat first: {noted}"
    );
}

#[test]
fn an_achievement_reworded_without_new_icons_is_named_to_look_at_them() {
    let game = inn_mod();
    let records = game.path().join("mods/base/data/achievements.json");
    let (first, said, why) = write_inn(game.path(), false);
    assert_eq!(first, Some(0_i32), "{said}{why}");
    let reworded = INN_RECORDS
        .replace("Down by the Wheel", "Under the Taproom")
        .replace(
            "Drain a mill's first sluice.",
            "Dig under an inn's taproom.",
        );
    fs::write(&records, &reworded).unwrap();
    let (code, _, noted) = write_inn(game.path(), false);
    assert_eq!(code, Some(0_i32), "{noted}");
    assert!(
        noted.contains(
            "FIRST_ANCHOR's English name and English description changed and its icons did not. \
             Look at them, so that they still show what it asks for\n"
        ),
        "{noted}"
    );
    assert!(!noted.contains("FIVE_LEVELS's English"), "{noted}");

    // Drawn afresh with the new words, the icons are taken to show them.
    fs::write(
        &records,
        reworded.replace("Under the Taproom", "Below the Taproom"),
    )
    .unwrap();
    fs::write(
        game.path().join("mods/base/icons/first_anchor.png"),
        icon_png(256, 140),
    )
    .unwrap();
    let (redrawn, _, quiet) = write_inn(game.path(), false);
    assert_eq!(redrawn, Some(0_i32), "{quiet}");
    assert!(!quiet.contains("its icons did not"), "{quiet}");
}

#[test]
fn definitions_saved_outside_utf8_are_said_and_nothing_is_compared_with_them() {
    let game = inn_mod();
    let (first, said, why) = write_inn(game.path(), false);
    assert_eq!(first, Some(0_i32), "{said}{why}");
    assert!(!why.contains("was not UTF-8 text"), "{why}");

    let definitions = game
        .path()
        .join("steam/achievements/1000050-achievements-definitions.csv");
    let mut bytes = fs::read(&definitions).unwrap();
    bytes.push(0xFF);
    fs::write(&definitions, bytes).unwrap();
    fs::write(
        game.path().join("mods/base/data/achievements.json"),
        INN_RECORDS
            .replace(r#""steam": "floors""#, r#""steam": "levels""#)
            .replace(r#""stat": "floors""#, r#""stat": "levels""#)
            .replace("Down by the Wheel", "Under the Taproom"),
    )
    .unwrap();
    let (code, _, noted) = write_inn(game.path(), false);
    assert_eq!(code, Some(0_i32), "{noted}");
    let said_so: Vec<&str> = noted
        .lines()
        .map(str::trim)
        .filter(|line| line.contains("was not UTF-8 text"))
        .collect();
    assert!(
        said_so
            == [
                "1000050-achievements-definitions.csv was not UTF-8 text before this write, so \
                 its progress bars and wording cannot be compared. Look at each achievement's \
                 bar and icons in the editor once imported, before publishing"
            ],
        "{noted}"
    );
    assert!(!noted.contains("progress bar goes from"), "{noted}");
    assert!(!noted.contains("its icons did not"), "{noted}");
}

#[test]
fn each_achievement_is_compared_with_what_the_earlier_localisation_gives_of_it() {
    let named_after = |change: fn(&str) -> Option<Vec<u8>>| -> Vec<String> {
        let game = inn_mod();
        let (first, said, why) = write_inn(game.path(), false);
        assert_eq!(first, Some(0_i32), "{said}{why}");
        let localisation = game
            .path()
            .join("steam/achievements/1000050-achievements-localization.csv");
        match change(&fs::read_to_string(&localisation).unwrap()) {
            Some(bytes) => fs::write(&localisation, bytes).unwrap(),
            None => fs::remove_file(&localisation).unwrap(),
        }
        fs::write(
            game.path().join("mods/base/data/achievements.json"),
            INN_RECORDS
                .replace(r#""steam": "floors""#, r#""steam": "levels""#)
                .replace(r#""stat": "floors""#, r#""stat": "levels""#)
                .replace("Down by the Wheel", "Under the Taproom"),
        )
        .unwrap();
        let (code, _, noted) = write_inn(game.path(), false);
        assert_eq!(code, Some(0_i32), "{noted}");
        assert!(!noted.contains("was not UTF-8 text"), "{noted}");
        noted
            .lines()
            .filter_map(|line| {
                line.trim().strip_suffix(
                    " changed and its icons did not. Look at them, so that they still show what \
                     it asks for",
                )
            })
            .map(str::to_owned)
            .collect()
    };
    let all = [
        "FIVE_LEVELS's English name, English description and progress stat",
        "FIRST_ANCHOR's English name and English description",
    ];
    let cases = [
        ("missing", named_after(|_| None), all),
        ("empty", named_after(|_| Some(Vec::new())), all),
        (
            "its heads alone",
            named_after(|text| Some(text.lines().take(1).collect::<String>().into_bytes())),
            all,
        ),
        (
            "no English column",
            named_after(|text| Some(text.replacen(",english,", ",french,", 1).into_bytes())),
            all,
        ),
        (
            "saved in a code page, so not UTF-8",
            named_after(|text| {
                let bytes = text.chars().map(|each| u8::try_from(u32::from(each)));
                Some(bytes.collect::<Result<_, _>>().unwrap())
            }),
            all,
        ),
        (
            "no rows of FIRST_ANCHOR",
            named_after(|text| Some(text.replace("FIRST_ANCHOR,", "LAST_ANCHOR,").into_bytes())),
            [
                "FIVE_LEVELS's progress stat",
                "FIRST_ANCHOR's English name and English description",
            ],
        ),
        (
            "a name twice, which --check refuses",
            named_after(|text| {
                Some(format!("{text}FIVE_LEVELS,name,Twice,Zweimal\n").into_bytes())
            }),
            ["FIVE_LEVELS's progress stat", "FIRST_ANCHOR's English name"],
        ),
    ];
    let wrong: Vec<_> = cases
        .iter()
        .filter(|(_, seen, named)| seen != named)
        .collect();
    assert!(wrong.is_empty(), "{wrong:?}");
}

#[test]
fn a_file_written_otherwise_or_left_over_is_named_and_rewritten_or_removed() {
    let game = inn_mod();
    let out = game.path().join("steam").join("achievements");
    assert_eq!(write_inn(game.path(), false).0, Some(0_i32));
    fs::write(out.join("OLD_ACHIEVED.png"), png_header(64, 64)).unwrap();
    fs::write(out.join("notes.md"), "kept\n").unwrap();
    // The cellar's set dropped, and the floors counted and drawn no more.
    let records = INN_RECORDS
        .replace(
            r#"{"type": "achievement_set", "id": "base:cellar", "steam": 1, "name": "The Cellar"},"#,
            "",
        )
        .replace(r#""set": "cellar", "#, "")
        .replace(
            r#""stat": "floors", "at": 5, "icon": "icons/five_levels.png""#,
            r#""hidden": true"#,
        );
    fs::write(
        game.path().join("mods/base/data/achievements.json"),
        records,
    )
    .unwrap();
    let (stale, _, why_stale) = write_inn(game.path(), true);
    assert_eq!(stale, Some(2_i32), "{why_stale}");
    for why in [
        "1000050-achievements-definitions.csv differs from what the files make\n",
        "1000050-achievement-sets-localization.csv is left over\n",
        "FIVE_LEVELS_ACHIEVED.png is left over\n",
        "OLD_ACHIEVED.png is left over\n",
    ] {
        assert!(why_stale.contains(why), "{why} in {why_stale}");
    }
    let (rewritten, again, noted) = write_inn(game.path(), false);
    assert_eq!(rewritten, Some(0_i32), "{again}");
    for said in [
        "  removed   OLD_ACHIEVED.png\n",
        "  removed   1000050-achievement-sets-localization.csv\n",
        "  wrote     1000050-achievements-definitions.csv\n",
    ] {
        assert!(again.contains(said), "{said} in {again}");
    }
    assert!(
        !again.contains("wrote     1000050-achievements-localization.csv"),
        "a file already as written is left alone: {again}"
    );
    assert!(
        noted.contains("no icon for 1 achievement: Steam shows its own\n")
            && noted.contains(
                "FIVE_LEVELS's progress bar goes from floors 0 - 5 to no progress bar, which \
                 Steam's Web API cannot show. Look at it in the editor once imported, and set it \
                 there by hand if it has not moved, before publishing\n"
            ),
        "{noted}"
    );
    for unsaid in [
        "each set",
        "each stat a progress bar counts",
        "no locked icon",
    ] {
        assert!(!noted.contains(unsaid), "{unsaid} in {noted}");
    }
    assert!(!out.join("OLD_ACHIEVED.png").exists());
    assert!(
        out.join("notes.md").is_file(),
        "a file of no kind written is left"
    );
    assert_eq!(write_inn(game.path(), true).0, Some(0_i32));
}

/// [`INN_SCHEMA`] holding [`INN_RECORDS`]' stat and achievements, with Steam's images of five
/// floors' icon and the cellar's two named by `icons` in turn.
fn inn_schema(icons: [&str; 3]) -> &'static str {
    let [five_levels, first_anchor, first_anchor_locked] = icons;
    INN_SCHEMA
        .replace(
            r#",{"name":"staff","defaultvalue":0,"displayName":"Staff working the stones"}"#,
            "",
        )
        .replace("LONG_JOURNEY", "FIVE_LEVELS")
        .replace("Rising Water", "Five Turns Round")
        .replace(
            "Bring the millpond up to its brim.",
            "Spin the wheel five times in a tide.",
        )
        .replace("CRAFTED_ALONE", "FIRST_ANCHOR")
        .replace("Bushel by Bushel", "Down by the Wheel")
        .replace(
            "Bag a sack of flour by hand.",
            "Drain a mill's first sluice.",
        )
        .replace("f895e737db45e18257a61fac5b6d409aa6872b82", five_levels)
        .replace("41373b0acc59cfbe2c973699975640bc1db1026a", first_anchor)
        .replace(
            "c4d7c246b05fc3bacc39c9106295794b52faee45",
            first_anchor_locked,
        )
        .leak()
}

/// `steamship achievements` in `game`, an [`inn_mod`], with its `steam/achievements` folder as
/// `--out`, and the Web API answering with `answers`: `--write`, `--imported` and its records
/// and catalogue when `imported`, or else `--check` and its records.
fn inn_on_steam<Body>(
    game: &Path,
    imported: bool,
    answers: Vec<(&'static str, Body)>,
) -> (Option<i32>, String, String)
where
    Body: AsRef<[u8]> + Send + 'static,
{
    let (host, _requests) = web_api(answers);
    inn_at(game, imported, &host)
}

/// [`inn_on_steam`] with the Web API's stand-in at `host`.
fn inn_at(game: &Path, imported: bool, host: &str) -> (Option<i32>, String, String) {
    let named = [
        game.join("mods/base/data/achievements.json"),
        game.join("locale/de.po"),
        game.join("steam").join("achievements"),
    ]
    .map(|path| path.to_string_lossy().into_owned());
    let [records, german, out] = named.each_ref().map(String::as_str);
    let mut all = vec!["achievements", "1000050"];
    if imported {
        all.extend(["--write", records, german, "--imported"]);
    } else {
        all.extend(["--check", records]);
    }
    all.extend(["--out", out]);
    steamship(&all, Some(game), &web_api_variables(host))
}

/// What a stand-in answers when steamship is refused before it asks Steam anything.
const UNASKED: [(&str, &str); 0] = [];

/// The icons the inn's import holds, in the order its records give them.
const INN_ICONS: [&str; 3] = [
    "FIVE_LEVELS_ACHIEVED.png",
    "FIRST_ANCHOR_ACHIEVED.png",
    "FIRST_ANCHOR_UNACHIEVED.png",
];

/// A JPEG of the PNG `file`, as Steam serves an icon it took.
fn served(file: &[u8]) -> Vec<u8> {
    let picture = image::load_from_memory_with_format(file, image::ImageFormat::Png)
        .unwrap()
        .to_rgb8();
    let mut bytes = io::Cursor::new(Vec::new());
    picture
        .write_to(&mut bytes, image::ImageFormat::Jpeg)
        .unwrap();
    bytes.into_inner()
}

/// Steam's answers to `--imported` in `game`, an [`inn_mod`] whose files are written, once Steam
/// took them in the import whose images `icons` names: the schema, then each image it serves.
fn inn_served(game: &Path, icons: [&'static str; 3]) -> Vec<(&'static str, Vec<u8>)> {
    let out = game.join("steam").join("achievements");
    iter::once(("200 OK", inn_schema(icons).as_bytes().to_vec()))
        .chain(INN_ICONS.map(|name| ("200 OK", served(&fs::read(out.join(name)).unwrap()))))
        .collect()
}

/// The names [`INN_SCHEMA`] gives Steam's images of five floors' icon and the cellar's two, as
/// the inn's first import made them.
const FIRST_IMPORT: [&str; 3] = [
    "f895e737db45e18257a61fac5b6d409aa6872b82",
    "41373b0acc59cfbe2c973699975640bc1db1026a",
    "c4d7c246b05fc3bacc39c9106295794b52faee45",
];

/// Steam's answers to a check of the inn's records, its icons' images named by `icons`.
fn inn_checked(icons: [&'static str; 3]) -> Vec<(&'static str, &'static str)> {
    iter::once(("200 OK", inn_schema(icons)))
        .chain(TTI_UNAGGREGATED.into_iter().take(1))
        .collect()
}

/// An [`inn_mod`] with its files written and `--imported` run once Steam took them in
/// [`FIRST_IMPORT`], with what `--imported` printed and noted.
fn inn_imported() -> (tempfile::TempDir, String, String) {
    let game = inn_mod();
    let (written, wrote, why) = write_inn(game.path(), false);
    assert_eq!(written, Some(0_i32), "{wrote}{why}");
    let (recorded, said, noted) =
        inn_on_steam(game.path(), true, inn_served(game.path(), FIRST_IMPORT));
    assert_eq!(recorded, Some(0_i32), "{said}{noted}");
    (game, said, noted)
}

/// The receipt in `game`, an [`inn_mod`].
fn inn_receipt(game: &Path) -> PathBuf {
    game.join("steam")
        .join("achievements")
        .join("steam-icons.json")
}

#[test]
fn imported_records_each_icon_steam_holds_and_write_leaves_the_receipt_alone() {
    let (game, said, noted) = inn_imported();
    assert!(
        said.contains("  steam     \u{2713} 3 of 3 icons as the files\n")
            && said.contains("  wrote     steam-icons.json\n")
            && said.contains("records the 3 icons Steam has published, each as its file\n"),
        "{said}"
    );
    assert!(!noted.contains("published:"), "{noted}");
    let receipt = inn_receipt(game.path());
    let kept: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&receipt).unwrap()).unwrap();
    assert_eq!(
        kept.get("app").and_then(serde_json::Value::as_u64),
        Some(1_000_050)
    );
    assert_eq!(
        kept.pointer("/icons/FIRST_ANCHOR_UNACHIEVED.png/steam")
            .and_then(serde_json::Value::as_str),
        Some("c4d7c246b05fc3bacc39c9106295794b52faee45.jpg")
    );
    assert_eq!(
        kept.pointer("/icons/FIVE_LEVELS_ACHIEVED.png/sha256")
            .and_then(serde_json::Value::as_str)
            .map(str::len),
        Some(64)
    );
    assert_eq!(
        write_inn(game.path(), true).0,
        Some(0_i32),
        "the receipt is no file --write makes, nor one left over"
    );
    assert_eq!(write_inn(game.path(), false).0, Some(0_i32));
    assert!(receipt.is_file(), "--write leaves the receipt alone");

    let (matched, shown, _) = inn_on_steam(game.path(), false, inn_checked(FIRST_IMPORT));
    assert_eq!(matched, Some(0_i32), "{shown}");
    assert!(
        shown.contains("Steam's achievements and stats match the file\n"),
        "{shown}"
    );
}

#[test]
fn an_icon_changed_in_the_files_is_named_until_steam_takes_its_import() {
    let (game, _, _) = inn_imported();
    let first = FIRST_IMPORT;
    let checked = inn_checked;
    fs::write(
        game.path().join("mods/base/icons/five_levels.png"),
        icon_png(128, 20),
    )
    .unwrap();
    let (changed, _, why_changed) = inn_on_steam(game.path(), false, checked(first));
    assert_eq!(changed, Some(2_i32), "{why_changed}");
    assert!(
        why_changed.contains(
            "FIVE_LEVELS: the icon changed in the files since the import steam-icons.json \
             records. Import it, publish, then run --imported\n"
        ) && why_changed.contains("1 of 2 achievements match: 1 differs\n")
            && why_changed.contains(
                "Steam answers with what is published. An import saved in Steamworks counts \
                 here once it is published\n"
            ),
        "{why_changed}"
    );

    let (unwritten, told, why_unwritten) = inn_on_steam(game.path(), true, UNASKED.to_vec());
    assert_eq!(unwritten, Some(2_i32), "{why_unwritten}");
    assert!(
        why_unwritten.contains("FIVE_LEVELS_ACHIEVED.png differs from what the files make\n")
            && told.contains("run it without --imported to write "),
        "{told}{why_unwritten}"
    );

    assert_eq!(write_inn(game.path(), false).0, Some(0_i32));
    let (untaken, _, why_untaken) =
        inn_on_steam(game.path(), true, vec![("200 OK", inn_schema(first))]);
    assert_eq!(untaken, Some(2_i32), "{why_untaken}");
    assert!(
        why_untaken.contains(
            "FIVE_LEVELS: Steam still has the icon the last import made, where the file changed \
             since\n"
        ),
        "{why_untaken}"
    );
    let again = [
        "0d2c5c6b7e3a1f4e9b8a7c6d5e4f3a2b1c0d9e8f",
        first[1],
        first[2],
    ];
    let (taken, said_again, noted_again) =
        inn_on_steam(game.path(), true, inn_served(game.path(), again));
    assert_eq!(taken, Some(0_i32), "{said_again}{noted_again}");
    let (matched, shown, _) = inn_on_steam(game.path(), false, checked(again));
    assert_eq!(matched, Some(0_i32), "{shown}");
}

#[test]
fn imported_names_once_an_achievement_steam_does_not_have() {
    let game = inn_mod();
    let (written, wrote, why_written) = write_inn(game.path(), false);
    assert_eq!(written, Some(0_i32), "{wrote}{why_written}");
    let unpublished = inn_schema(FIRST_IMPORT)
        .replace(r#""name":"FIRST_ANCHOR""#, r#""name":"LAST_CELLAR""#)
        .leak();
    let (refused, _, why) = inn_on_steam(game.path(), true, vec![("200 OK", unpublished)]);
    assert_eq!(refused, Some(2_i32), "{why}");
    assert_eq!(
        why.matches("FIRST_ANCHOR").count(),
        1,
        "the achievement, not each of its two icons: {why}"
    );
    assert!(why.contains("FIRST_ANCHOR: not on Steam\n"), "{why}");
    assert!(!inn_receipt(game.path()).exists());
}

#[test]
fn imported_refuses_an_icon_steam_has_that_the_files_give_none_of() {
    let game = inn_mod();
    let records = game.path().join("mods/base/data/achievements.json");
    fs::write(
        &records,
        INN_RECORDS.replace(r#", "icon": "icons/five_levels.png""#, ""),
    )
    .unwrap();
    let (written, wrote, why_written) = write_inn(game.path(), false);
    assert_eq!(written, Some(0_i32), "{wrote}{why_written}");
    let retired = r#""achievements":[{"name":"OLD_TAPROOM","defaultvalue":0,"displayName":"Old Taproom","hidden":0,"description":"Keep the old taproom.","icon":"https://steamcdn-a.akamaihd.net/steamcommunity/public/images/apps/1000050/9e1f0a2b3c4d5e6f7a8b9c0d1e2f3a4b5c6d7e8f.jpg","icongray":""},"#;
    let uploaded = inn_schema(FIRST_IMPORT)
        .replacen(r#""achievements":["#, retired, 1)
        .leak();
    let (refused, told, why) = inn_on_steam(game.path(), true, vec![("200 OK", uploaded)]);
    assert_eq!(refused, Some(2_i32), "{told}{why}");
    assert!(
        why.contains("FIVE_LEVELS: the files give no icon, where Steam has one\n"),
        "{why}"
    );
    assert!(
        !why.contains("OLD_TAPROOM"),
        "an achievement the files leave out is --check's to name: {why}"
    );
    assert!(
        !why.contains("no locked icon"),
        "Steam makes a locked icon itself when asked to: {why}"
    );
    assert!(
        told.contains(
            "give each of those achievements its \"icon\" in the files, and run it without "
        ) && !told.contains("icons as the files"),
        "{told}"
    );
    assert!(!inn_receipt(game.path()).exists());
}

#[test]
fn imported_refuses_an_icon_steam_serves_otherwise_than_the_file() {
    let game = inn_mod();
    let (written, wrote, why_written) = write_inn(game.path(), false);
    assert_eq!(written, Some(0_i32), "{wrote}{why_written}");
    let mut answers = inn_served(game.path(), FIRST_IMPORT);
    let [_, five_levels, first_anchor, first_anchor_locked] = answers.as_mut_slice() else {
        panic!("the schema and three images");
    };
    *five_levels = ("200 OK", served(&icon_png(256, 20)));
    *first_anchor = ("404 Not Found", Vec::new());
    *first_anchor_locked = ("200 OK", b"not a JPEG".to_vec());
    let (host, requests) = web_api(answers);
    let (refused, told, why) = inn_at(game.path(), true, &host);
    assert_eq!(refused, Some(2_i32), "{told}{why}");
    for said in [
        "FIVE_LEVELS: Steam's icon is not the file's: 49152 of 65536 pixels more than 64 off, ",
        "FIRST_ANCHOR: Steam's icon could not be compared with the file: Steam's image host \
         answered HTTP 404\n",
        "FIRST_ANCHOR: Steam's locked icon could not be compared with the file: Steam's image: ",
        "Steam answers with what is published. An import saved in Steamworks counts here once \
         it is published\n",
    ] {
        assert!(why.contains(said), "{said} in {why}");
    }
    assert!(
        told.contains("  steam     \u{2717} 0 of 3 icons as the files\n")
            && told.contains("drop the PNG files in ")
            && told.contains(
                " again and Save imported data, then stay on the page until Pending Changes is \
                 gone. Then publish, and run --imported again\n"
            ),
        "{told}"
    );
    assert!(!inn_receipt(game.path()).exists());

    let asked: Vec<String> = requests.try_iter().collect();
    let images: Vec<&String> = asked
        .iter()
        .filter(|request| request.contains("/community_assets/images/apps/1000050/"))
        .collect();
    assert_eq!(images.len(), 3, "{asked:?}");
    assert!(
        images.first().is_some_and(|request| {
            request.starts_with(
                "GET /community_assets/images/apps/1000050/\
                 f895e737db45e18257a61fac5b6d409aa6872b82.jpg ",
            )
        }),
        "{images:?}"
    );
    assert!(
        images
            .iter()
            .all(|request| !request.to_ascii_lowercase().contains("x-webapi-key")),
        "the key goes to the Web API alone: {images:?}"
    );
}

#[test]
fn an_icon_steam_made_anew_from_the_file_imported_is_named() {
    let (game, _, _) = inn_imported();
    let [five_levels, first_anchor, _] = FIRST_IMPORT;
    let redrawn = [
        five_levels,
        first_anchor,
        "5b1e0c2d9f8a7b6c5d4e3f2a1b0c9d8e7f6a5b4c",
    ];
    let (moved, _, why_moved) = inn_on_steam(game.path(), false, inn_checked(redrawn));
    assert_eq!(moved, Some(2_i32), "{why_moved}");
    assert!(
        why_moved.contains(
            "FIRST_ANCHOR: the locked icon on Steam changed since the import steam-icons.json \
             records, and not from the files. Import it again, publish, then run --imported\n"
        ),
        "{why_moved}"
    );
}

#[test]
fn a_check_against_no_receipt_or_another_apps_is_refused_and_imported_replaces_one() {
    let bare = inn_mod();
    assert_eq!(write_inn(bare.path(), false).0, Some(0_i32));
    let (unrecorded, _, why) = inn_on_steam(bare.path(), false, UNASKED.to_vec());
    assert_eq!(unrecorded, Some(2_i32), "{why}");
    assert!(
        why.contains(
            "achievements: has no steam-icons.json. Import and publish the files --write wrote \
             there, then run --imported\n"
        ),
        "{why}"
    );
    let unreadable = inn_receipt(bare.path());
    fs::create_dir_all(&unreadable).unwrap();
    let (failed, _, why_failed) = inn_on_steam(bare.path(), false, UNASKED.to_vec());
    assert_eq!(failed, Some(1_i32), "{why_failed}");
    assert!(
        why_failed.contains(&format!("{}: ", unreadable.display()))
            && !why_failed.contains("has no steam-icons.json"),
        "a receipt that cannot be read is not taken for none: {why_failed}"
    );

    let (game, _, _) = inn_imported();
    let receipt = inn_receipt(game.path());
    let kept = fs::read_to_string(&receipt).unwrap();
    fs::write(&receipt, kept.replace("1000050", "480")).unwrap();
    let (elsewhere, _, why_elsewhere) = inn_on_steam(game.path(), false, UNASKED.to_vec());
    assert_eq!(elsewhere, Some(2_i32), "{why_elsewhere}");
    assert!(
        why_elsewhere.contains("steam-icons.json: records app 480's icons, not app 1000050's\n"),
        "{why_elsewhere}"
    );
    fs::write(&receipt, "{}").unwrap();
    let (unread, _, why_unread) = inn_on_steam(game.path(), false, UNASKED.to_vec());
    assert_eq!(unread, Some(2_i32), "{why_unread}");
    assert!(
        why_unread.contains("steam-icons.json: \"app\" is not an app ID\n"),
        "{why_unread}"
    );
    let (replaced, _, _) = inn_on_steam(game.path(), true, inn_served(game.path(), FIRST_IMPORT));
    assert_eq!(
        replaced,
        Some(0_i32),
        "--imported replaces a receipt it cannot use"
    );
    assert!(
        fs::read_to_string(&receipt)
            .unwrap()
            .contains("\"app\": 1000050")
    );
}

#[test]
fn a_receipts_flags_are_refused_where_they_mean_nothing() {
    let game = tempfile::tempdir().unwrap();
    for (args, refusal) in [
        (
            ["--out", "steam/achievements", "--json"].as_slice(),
            "the following required arguments were not provided",
        ),
        (
            [
                "--write",
                "a.json",
                "--out",
                "o",
                "--imported",
                "--unchanged",
            ]
            .as_slice(),
            "cannot be used with",
        ),
        (["--imported"].as_slice(), "there is no game in "),
    ] {
        let mut all = vec!["achievements", "1000050"];
        all.extend(args);
        let (refused, _, why) = steamship(&all, Some(game.path()), &[]);
        assert_eq!(refused, Some(2_i32), "{args:?}: {why}");
        assert!(why.contains(refusal), "{args:?}: {why}");
    }
}

#[test]
fn the_achievement_editors_files_are_refused_as_the_editor_would_refuse_them() {
    let game = inn_mod();
    let base = game.path().join("mods/base");
    let records = base.join("data/achievements.json");
    let german = game.path().join("locale/de.po");
    let out = game.path().join("out");
    let write = |files: &[&Path]| {
        let mut args = vec!["achievements", "1000050", "--write"];
        let named: Vec<String> = files
            .iter()
            .map(|file| file.to_string_lossy().into_owned())
            .collect();
        args.extend(named.iter().map(String::as_str));
        let out = out.to_string_lossy().into_owned();
        args.extend(["--out", &out]);
        let (code, _, why) = steamship(
            &args,
            Some(game.path()),
            &[("STEAMSHIP_NO_UPDATE_CHECK", "1")],
        );
        assert_eq!(code, Some(2_i32), "{why}");
        why
    };

    fs::write(base.join("icons/five_levels.png"), png_header(256, 128)).unwrap();
    let oblong = write(&[&records]);
    assert!(
        oblong.contains(
            "FIVE_LEVELS's icon is a PNG of 256 by 128 pixels, where the achievement editor \
             takes a square PNG of 64 pixels or more, 256 at best\n"
        ),
        "{oblong}"
    );
    fs::write(base.join("icons/five_levels.png"), png_header(32, 32)).unwrap();
    let small = write(&[&records]);
    assert!(
        small.contains("FIVE_LEVELS's icon is a PNG of 32 by 32 pixels"),
        "{small}"
    );
    fs::write(base.join("icons/five_levels.png"), png_header(256, 256)).unwrap();

    let english = game.path().join("locale/en.po");
    fs::write(&english, "msgid \"\"\nmsgstr \"Language: en\\n\"\n").unwrap();
    let in_english = write(&[&records, &english]);
    assert!(
        in_english
            .contains("en.po: is in English, the language the game's own files are written in\n"),
        "{in_english}"
    );
    let again = game.path().join("locale/de_DE.po");
    fs::write(&again, INN_GERMAN).unwrap();
    let twice = write(&[&records, &german, &again]);
    assert!(
        twice.contains(&format!(
            "de_DE.po: is in german, as {} is\n",
            german.display()
        )),
        "{twice}"
    );

    fs::write(&german, INN_GERMAN.replace("\"Hinab zum Rad\"", "\"\"")).unwrap();
    let untranslated = write(&[&records, &german]);
    assert!(
        untranslated.contains(
            "de.po has no name of FIRST_ANCHOR, under msgctxt \"achievement.name\" and msgid \
             \"base:first_anchor\"\n"
        ),
        "{untranslated}"
    );

    let klingon = game.path().join("locale/tlh.po");
    fs::write(&klingon, "msgid \"x\"\nmsgstr \"y\"\n").unwrap();
    let unknown = write(&[&records, &klingon]);
    assert!(
        unknown.contains("tlh.po: is in the locale tlh, which Steam has no language for\n"),
        "{unknown}"
    );

    fs::write(&records, INN_RECORDS.replace("\"stat\": \"floors\", ", "")).unwrap();
    let uncounted = write(&[&records]);
    assert!(
        uncounted.contains(
            "FIVE_LEVELS unlocks at 5 but gives no \"stat\" for its progress bar to count\n"
        ),
        "{uncounted}"
    );
    assert!(
        !out.exists(),
        "nothing is written while the files are refused"
    );

    fs::write(&records, INN_RECORDS).unwrap();
    fs::write(&out, "notes\n").unwrap();
    let filed = write(&[&records]);
    assert!(
        filed.contains(&format!(
            "{}: is a file, where --out names the folder the files are written into\n",
            out.display()
        )),
        "{filed}"
    );
    assert_eq!(fs::read_to_string(&out).unwrap(), "notes\n");
}

#[test]
fn an_achievement_and_a_stat_of_one_api_name_are_refused_before_anything_is_written() {
    let game = inn_mod();
    let records = game.path().join("mods/base/data/achievements.json");
    fs::write(
        &records,
        INN_RECORDS.replace("\"steam\": \"floors\"", "\"steam\": \"FIVE_LEVELS\""),
    )
    .unwrap();
    let out = game.path().join("out");
    let named = [&records, &out].map(|path| path.to_string_lossy().into_owned());
    for args in [
        ["--write", &named[0], "--out", &named[1]].as_slice(),
        ["--check", &named[0]].as_slice(),
    ] {
        let (code, _, why) = steamship(
            &[["achievements", "1000050"].as_slice(), args].concat(),
            Some(game.path()),
            &[("STEAMSHIP_NO_UPDATE_CHECK", "1")],
        );
        assert_eq!(code, Some(2_i32), "{args:?}: {why}");
        assert!(
            why.contains(&format!(
                "FIVE_LEVELS is both an achievement and a stat in {}. Steamworks cannot publish \
                 an achievement and a stat of one API name. Rename one\n",
                records.display()
            )),
            "{args:?}: {why}"
        );
    }
    assert!(
        !out.exists(),
        "nothing is written while the files are refused"
    );
}

#[test]
fn more_than_a_hundred_achievements_are_said_to_need_a_vetted_app() {
    let folder = tempfile::tempdir().unwrap();
    let file = folder.path().join("achievements.json");
    let out = folder.path().join("out");
    for (count, said) in [(100_u32, false), (101_u32, true)] {
        let achievements: Vec<String> = (0_u32..count)
            .map(|each| format!(r#"{{"api_name": "A{each}", "name": "A"}}"#))
            .collect();
        fs::write(
            &file,
            format!(r#"{{"achievements": [{}]}}"#, achievements.join(",")),
        )
        .unwrap();
        let (code, _, noted) = steamship(
            &[
                "achievements",
                "480",
                "--write",
                &file.to_string_lossy(),
                "--out",
                &out.to_string_lossy(),
            ],
            Some(folder.path()),
            &[("STEAMSHIP_NO_UPDATE_CHECK", "1")],
        );
        assert_eq!(code, Some(0_i32), "{noted}");
        assert_eq!(
            noted.contains(
                "Steam takes more than 100 achievements only once Valve has vetted the app\n"
            ),
            said,
            "{count}: {noted}"
        );
    }
}

#[test]
fn achievements_are_checked_in_each_language_a_catalogue_is_in() {
    let home = tempfile::tempdir().unwrap();
    let (host, requests) = web_api(vec![("200 OK", CELLAR), ("200 OK", KELLER)]);
    let set = web_api_variables(&host);
    let files = tempfile::tempdir().unwrap();
    let records = files.path().join("achievements.json");
    fs::write(
        &records,
        r#"[{"type": "achievement", "id": "base:first_anchor", "steam": "FIRST_ANCHOR",
            "name": "Down by the Wheel", "description": "Drain a mill's first sluice."}]"#,
    )
    .unwrap();
    let german = files.path().join("de.po");
    fs::write(
        &german,
        "msgctxt \"achievement.name\"\nmsgid \"base:first_anchor\"\nmsgstr \"Hinab zum Rad\"\n\
         msgctxt \"achievement.description\"\nmsgid \"base:first_anchor\"\n\
         msgstr \"Gib die erste Schleuse eines Mahlwerks frei.\"\n",
    )
    .unwrap();
    let args = [
        "achievements",
        "1000050",
        "--check",
        &records.to_string_lossy(),
        &german.to_string_lossy(),
    ];

    let (code, said, _) = steamship(&args, Some(home.path()), &set);
    assert_eq!(code, Some(0_i32), "{said}");
    assert!(said.contains("de.po, german\n"), "{said}");
    assert!(
        said.contains("Steam's achievements and stats match the files\n"),
        "{said}"
    );
    let first = requests.recv_timeout(Duration::from_secs(10)).unwrap();
    let second = requests.recv_timeout(Duration::from_secs(10)).unwrap();
    assert!(first.contains("l=english"), "{first}");
    assert!(second.contains("l=german"), "{second}");
}

/// A sheet of steamship's own kind whose texts sit under contexts the game chose, and the German
/// catalogue that holds them.
fn named_contexts(files: &Path, description_context: &str) -> (PathBuf, PathBuf) {
    let sheet = files.join("achievements.json");
    fs::write(
        &sheet,
        r#"{"stats": [], "achievements": [{"api_name": "FIRST_ANCHOR",
            "name": "Down by the Wheel", "name_context": "deed.first_anchor",
            "description": "Drain a mill's first sluice.",
            "description_context": "deed.first_anchor.about"}]}"#,
    )
    .unwrap();
    let german = files.join("de.po");
    fs::write(
        &german,
        format!(
            "msgctxt \"deed.first_anchor\"\nmsgid \"Down by the Wheel\"\nmsgstr \"Hinab zum Rad\"\n\
             msgctxt \"{description_context}\"\nmsgid \"Drain a mill's first sluice.\"\n\
             msgstr \"Gib die erste Schleuse eines Mahlwerks frei.\"\n"
        ),
    )
    .unwrap();
    (sheet, german)
}

#[test]
fn achievements_are_checked_and_written_under_the_contexts_their_file_names() {
    let home = tempfile::tempdir().unwrap();
    let (host, _requests) = web_api(vec![("200 OK", CELLAR), ("200 OK", KELLER)]);
    let set = web_api_variables(&host);
    let files = tempfile::tempdir().unwrap();
    let (sheet, german) = named_contexts(files.path(), "deed.first_anchor.about");
    let (sheet, german) = (sheet.to_string_lossy(), german.to_string_lossy());

    let (code, said, _) = steamship(
        &["achievements", "1000050", "--check", &sheet, &german],
        Some(home.path()),
        &set,
    );
    assert_eq!(code, Some(0_i32), "{said}");
    assert!(
        said.contains("Steam's achievements and stats match the files\n"),
        "{said}"
    );

    let out = files.path().join("out");
    let (written, printed, why) = steamship(
        &[
            "achievements",
            "1000050",
            "--write",
            &sheet,
            &german,
            "--out",
            &out.to_string_lossy(),
        ],
        Some(home.path()),
        &[("STEAMSHIP_NO_UPDATE_CHECK", "1")],
    );
    assert_eq!(written, Some(0_i32), "{printed}{why}");
    let csv = fs::read_to_string(out.join("1000050-achievements-localization.csv")).unwrap();
    assert!(
        csv.contains("FIRST_ANCHOR,name,Down by the Wheel,Hinab zum Rad"),
        "{csv}"
    );
}

#[test]
fn a_text_missing_under_a_named_context_is_refused_before_steam_is_asked() {
    let home = tempfile::tempdir().unwrap();
    let (host, requests) = web_api(vec![("200 OK", CELLAR), ("200 OK", KELLER)]);
    let set = web_api_variables(&host);
    let files = tempfile::tempdir().unwrap();
    let (sheet, german) = named_contexts(files.path(), "deed.first_anchor.elsewhere");
    let (code, _, why) = steamship(
        &[
            "achievements",
            "1000050",
            "--check",
            &sheet.to_string_lossy(),
            &german.to_string_lossy(),
        ],
        Some(home.path()),
        &set,
    );
    assert_eq!(code, Some(2_i32), "{why}");
    assert!(
        why.contains(
            "de.po has no description of FIRST_ANCHOR, under msgctxt \
             \"deed.first_anchor.about\" and msgid \"Drain a mill's first sluice.\""
        ),
        "{why}"
    );
    assert!(requests.try_recv().is_err(), "Steam was asked: {why}");
}

#[test]
fn achievements_json_prints_them_alone_on_standard_output_and_not_with_check() {
    let home = tempfile::tempdir().unwrap();
    let (host, requests) = web_api(vec![
        ("200 OK", SCHEMA),
        ("403 Forbidden", "{}"),
        ("200 OK", WITH_STATS),
        ("403 Forbidden", "{}"),
        ("200 OK", NOT_AGGREGATED),
        ("200 OK", TOTALS),
    ]);
    let set = web_api_variables(&host);
    let (code, stdout, stderr) =
        steamship(&["achievements", "480", "--json"], Some(home.path()), &set);
    assert_eq!(code, Some(0_i32), "{stdout}{stderr}");
    assert_eq!(
        summary_of(&stdout),
        serde_json::json!({
            "app": 480_u32,
            "achievements": [
                {"api_name": "ACH_WIN_ONE_GAME", "name": "Winner", "description": "Win one game.",
                 "hidden": false, "icon": "https://cdn/w.jpg", "icon_locked": "https://cdn/w_bw.jpg",
                 "percent": null},
                {"api_name": "ACH_TRAVEL_FAR_SINGLE", "name": "Orbiter",
                 "description": "Travel 500 feet in one life.", "hidden": true,
                 "icon": "https://cdn/o.jpg", "icon_locked": "https://cdn/o_bw.jpg", "percent": null},
            ],
            "stats": [],
        })
    );
    assert!(
        stderr.contains("2 achievements on Steam")
            && stderr.contains("Steam gives no shares of players or totals yet"),
        "{stderr}"
    );

    let (with_stats, statted, told) =
        steamship(&["achievements", "480", "--json"], Some(home.path()), &set);
    assert_eq!(with_stats, Some(0_i32), "{statted}{told}");
    assert_eq!(
        summary_of(&statted).get("stats"),
        Some(&serde_json::json!([
            {"api_name": "NumGames", "name": "Games played", "default": 0.0_f64,
             "total": 211_140_488_449_u64},
            {"api_name": "AverageSpeed", "name": "", "default": 1.5_f64, "total": null},
        ]))
    );
    assert!(
        told.contains("shares of 0 achievements and totals of 1 stat"),
        "{told}"
    );
    let totals: Vec<String> =
        iter::repeat_with(|| requests.recv_timeout(Duration::from_secs(10)).unwrap())
            .take(6)
            .filter(|request| request.contains("GetGlobalStatsForGame"))
            .collect();
    assert!(
        totals.len() == 2
            && totals.iter().all(|asked| asked.contains("appid=480"))
            && totals.first().is_some_and(|asked| {
                asked.contains("count=2")
                    && asked.contains("=NumGames")
                    && asked.contains("=AverageSpeed")
            })
            && totals.last().is_some_and(|asked| {
                asked.contains("count=1") && !asked.contains("AverageSpeed")
            }),
        "Steam is asked again without the stat it does not add up: {totals:?}"
    );

    let (refused, nothing, why) = steamship(
        &[
            "achievements",
            "480",
            "--json",
            "--check",
            "achievements.json",
        ],
        Some(home.path()),
        &set,
    );
    assert_eq!((refused, nothing.as_str()), (Some(2_i32), ""), "{why}");
    assert!(why.contains("cannot be used with"), "{why}");
}

/// `GetSchemaForGame` for an app with stats and no achievements.
const WITH_STATS: &str = r#"{"game": {"availableGameStats": {"stats": [
    {"name": "NumGames", "defaultvalue": 0, "displayName": "Games played"},
    {"name": "AverageSpeed", "defaultvalue": 1.5}
]}}}"#;

/// `GetGlobalStatsForGame` refusing a stat it keeps no totals of, as Steam answered for Spacewar,
/// the stat named as `WITH_STATS` names it.
const NOT_AGGREGATED: &str =
    r#"{"response":{"result":8,"error":"Stat 'AverageSpeed' is not an aggregated stat"}}"#;

/// `GetGlobalStatsForGame` as Steam answered it for Spacewar, trimmed to one stat.
const TOTALS: &str =
    r#"{"response":{"globalstats":{"NumGames":{"total":"211140488449"}},"result":1}}"#;

#[test]
fn stats_are_listed_and_checked_only_when_the_file_lists_them() {
    let home = tempfile::tempdir().unwrap();
    let files = tempfile::tempdir().unwrap();
    let (host, _requests) = web_api(vec![
        ("200 OK", WITH_STATS),
        ("403 Forbidden", "{}"),
        // A stat steamship never asked for, as Steam named one for Spacewar: asking again
        // without it would get the same answer, so totals are given up rather than asked for
        // without end.
        (
            "200 OK",
            r#"{"response":{"result":8,"error":"Stat 'NumWins' is not an aggregated stat"}}"#,
        ),
        ("200 OK", WITH_STATS),
        ("200 OK", WITH_STATS),
    ]);
    let set = web_api_variables(&host);

    let (code, listed, _) = steamship(&["achievements", "480"], Some(home.path()), &set);
    assert_eq!(code, Some(0_i32), "{listed}");
    assert!(
        listed.contains("\u{2713} 0 achievements and 2 stats on Steam\n")
            && listed.contains("  stat NumGames     Games played, starts at 0\n")
            && listed.contains("  stat AverageSpeed starts at 1.5\n")
            && !listed.contains("Stats & Achievements"),
        "stats alone are something on Steam: {listed}"
    );

    let unlisted = files.path().join("unlisted.json");
    fs::write(&unlisted, r#"{"achievements": []}"#).unwrap();
    let (matched, said, _) = steamship(
        &[
            "achievements",
            "480",
            "--check",
            &unlisted.to_string_lossy(),
        ],
        Some(home.path()),
        &set,
    );
    assert_eq!(
        matched,
        Some(0_i32),
        "no list, so the stats are not checked: {said}"
    );
    assert!(
        said.contains("Steam's achievements match the file\n"),
        "{said}"
    );

    let drifted = files.path().join("drifted.json");
    fs::write(
        &drifted,
        r#"{"achievements": [], "stats": [
            {"api_name": "NumGames", "name": "Games played", "default": 0},
            {"api_name": "AverageSpeed", "default": 2}
        ]}"#,
    )
    .unwrap();
    let (refused, shown, why) = steamship(
        &["achievements", "480", "--check", &drifted.to_string_lossy()],
        Some(home.path()),
        &set,
    );
    assert_eq!(refused, Some(2_i32), "{shown}{why}");
    assert!(shown.contains("0 achievements and 2 stats\n"), "{shown}");
    assert!(
        why.contains("stat AverageSpeed: the default is \"1.5\" on Steam and \"2\" in the file"),
        "{why}"
    );
    assert!(
        why.ends_with("1 of 2 stats match: 1 differs\n") && !shown.contains("achievements match"),
        "the file lists no achievements, so only the stats are counted: {shown}{why}"
    );
}

#[test]
fn whether_a_stat_is_aggregated_is_checked_against_the_totals_steam_keeps() {
    let home = tempfile::tempdir().unwrap();
    let files = tempfile::tempdir().unwrap();
    let (host, requests) = web_api(vec![
        ("200 OK", WITH_STATS),
        ("200 OK", NOT_AGGREGATED),
        ("200 OK", TOTALS),
        ("200 OK", WITH_STATS),
        // As Steam answered for Tidal Mill Inn before its release.
        (
            "200 OK",
            r#"{"response":{"result":24,"error":"Insufficient Privilege"}}"#,
        ),
    ]);
    let set = web_api_variables(&host);
    let both = files.path().join("both.json");
    fs::write(
        &both,
        r#"{"achievements": [], "stats": [
            {"api_name": "NumGames", "name": "Games played", "aggregated": true},
            {"api_name": "AverageSpeed", "default": 1.5, "aggregated": true},
            {"api_name": "Unmade", "aggregated": true}
        ]}"#,
    )
    .unwrap();
    let (refused, shown, why) = steamship(
        &["achievements", "480", "--check", &both.to_string_lossy()],
        Some(home.path()),
        &set,
    );
    assert_eq!(refused, Some(2_i32), "{shown}{why}");
    assert!(
        shown.contains("  steam     \u{2713} 1 of 2 stats aggregated\n"),
        "{shown}"
    );
    assert!(
        why.contains(
            "stat AverageSpeed: aggregated is \"false\" on Steam and \"true\" in the file\n"
        ) && why.ends_with("1 of 3 stats match: 1 differs, 1 not on Steam\n")
            && !why.contains("went unchecked"),
        "Steam is asked only of the stats it holds, and says of each: {why}"
    );
    let asked: Vec<String> =
        iter::repeat_with(|| requests.recv_timeout(Duration::from_secs(10)).unwrap())
            .take(3)
            .collect();
    assert!(
        asked.get(1).is_some_and(|first| first
            .starts_with("GET /ISteamUserStats/GetGlobalStatsForGame/v1/?")
            && first.contains("count=2")
            && !first.contains("Unmade"))
            && asked
                .last()
                .is_some_and(|again| again.contains("count=1") && !again.contains("AverageSpeed")),
        "{asked:?}"
    );

    let held = files.path().join("held.json");
    fs::write(
        &held,
        r#"{"achievements": [], "stats": [
            {"api_name": "NumGames", "name": "Games played", "aggregated": true},
            {"api_name": "AverageSpeed", "default": 1.5, "aggregated": true}
        ]}"#,
    )
    .unwrap();
    let (matched, said, noted) = steamship(
        &["achievements", "480", "--check", &held.to_string_lossy()],
        Some(home.path()),
        &set,
    );
    assert_eq!(matched, Some(0_i32), "{said}{noted}");
    assert!(
        noted.contains(
            "whether 2 stats are aggregated went unchecked: Steam answered \"Insufficient \
             Privilege\""
        ) && said.contains("Steam's achievements and stats match the file\n"),
        "{said}{noted}"
    );
}

#[test]
fn a_check_against_an_app_with_nothing_on_steam_says_so_once_for_each_kind_in_any_language() {
    let game = inn_mod();
    let named = [
        game.path().join("mods/base/data/achievements.json"),
        game.path().join("locale/de.po"),
    ]
    .map(|path| path.to_string_lossy().into_owned());
    let [records, german] = named.each_ref().map(String::as_str);
    // One answer only: asking for the achievements in German as well would find none.
    let (host, _requests) = web_api(vec![("200 OK", r#"{"game": {}}"#)]);
    let (refused, shown, why) = steamship(
        &["achievements", "1000050", "--check", records, german],
        Some(game.path()),
        &web_api_variables(&host),
    );
    assert_eq!(refused, Some(2_i32), "{shown}{why}");
    assert!(
        why.contains(
            "Steam holds none of the file's 2 achievements. They count once entered and \
             published in Steamworks\n"
        ) && why.contains(
            "Steam holds none of the file's 1 stat. It counts once entered and published in \
             Steamworks\n"
        ) && !why.contains("stat floors"),
        "{why}"
    );
    assert!(!shown.contains("more language"), "{shown}");
}

#[test]
fn an_app_with_nothing_on_steam_is_listed_as_none_with_where_they_are_made() {
    let home = tempfile::tempdir().unwrap();
    let files = tempfile::tempdir().unwrap();
    let (host, _requests) = web_api(vec![
        ("200 OK", r#"{"game": {}}"#),
        ("200 OK", r#"{"game": {}}"#),
    ]);
    let set = web_api_variables(&host);

    let (code, listed, _) = steamship(&["achievements", "480"], Some(home.path()), &set);
    assert_eq!(code, Some(0_i32), "{listed}");
    assert!(
        listed.contains("\u{2713} 0 achievements on Steam\n")
            && listed.contains(
                "\n  next      make and publish app 480's achievements and stats in Steamworks, \
                 under Stats & Achievements\n"
            ),
        "in the same column as the lines above it: {listed}"
    );

    let file = achievements_file(files.path(), "Travel 500 feet in one life.");
    let (refused, shown, why) = steamship(
        &["achievements", "480", "--check", &file],
        Some(home.path()),
        &set,
    );
    assert_eq!(refused, Some(2_i32), "{shown}{why}");
    assert!(
        why.contains("Steam holds none of the file's 2 achievements")
            && why.ends_with("0 of 2 achievements match: 2 not on Steam\n")
            && !shown.contains("Stats & Achievements"),
        "{shown}{why}"
    );
    assert!(
        !shown.contains("more language"),
        "Steam has a text in no language of an achievement it does not hold: {shown}"
    );
}

#[test]
fn a_check_with_drift_in_its_stats_still_says_its_achievements_match() {
    let home = tempfile::tempdir().unwrap();
    let files = tempfile::tempdir().unwrap();
    let (host, _requests) = web_api(vec![("200 OK", SCHEMA)]);
    let set = web_api_variables(&host);
    let file = achievements_file(files.path(), "Travel 500 feet in one life.");
    let listed = fs::read_to_string(&file).unwrap();
    fs::write(
        &file,
        listed.replacen(
            r#""achievements":"#,
            r#""stats": [{"api_name": "years_open", "default": 0}], "achievements":"#,
            1,
        ),
    )
    .unwrap();

    let (refused, shown, why) = steamship(
        &["achievements", "480", "--check", &file],
        Some(home.path()),
        &set,
    );
    assert_eq!(refused, Some(2_i32), "{shown}{why}");
    assert!(shown.contains("2 of 2 achievements match\n"), "{shown}");
    assert!(
        why.contains("Steam holds none of the file's 1 stat. It counts once entered and published")
            && why.ends_with("0 of 1 stats match: 1 not on Steam\n"),
        "{why}"
    );
}

/// `GetSchemaForGame` as Steam answered it for Tidal Mill Inn, trimmed to two stats and two
/// achievements.
const INN_SCHEMA: &str = r#"{"game":{"gameName":"Tidal Mill Inn","gameVersion":"2","availableGameStats":{"stats":[{"name":"floors","defaultvalue":0,"displayName":"Floodwater held behind the sluice"},{"name":"staff","defaultvalue":0,"displayName":"Staff working the stones"}],"achievements":[{"name":"LONG_JOURNEY","defaultvalue":0,"displayName":"Rising Water","hidden":0,"description":"Bring the millpond up to its brim.","icon":"https://steamcdn-a.akamaihd.net/steamcommunity/public/images/apps/1000050/f895e737db45e18257a61fac5b6d409aa6872b82.jpg","icongray":"https://steamcdn-a.akamaihd.net/steamcommunity/public/images/apps/1000050/a25a9ca72da3dae23b05f2fb6f712527cf3634d9.jpg"},{"name":"CRAFTED_ALONE","defaultvalue":0,"displayName":"Bushel by Bushel","hidden":0,"description":"Bag a sack of flour by hand.","icon":"https://steamcdn-a.akamaihd.net/steamcommunity/public/images/apps/1000050/41373b0acc59cfbe2c973699975640bc1db1026a.jpg","icongray":"https://steamcdn-a.akamaihd.net/steamcommunity/public/images/apps/1000050/c4d7c246b05fc3bacc39c9106295794b52faee45.jpg"}]}}}"#;

/// What Steam answers for Tidal Mill Inn when asked for the totals of its stats, which it keeps
/// none of. It names each in turn.
const TTI_UNAGGREGATED: [(&str, &str); 2] = [
    (
        "200 OK",
        r#"{"response":{"result":8,"error":"Stat 'floors' is not an aggregated stat"}}"#,
    ),
    (
        "200 OK",
        r#"{"response":{"result":8,"error":"Stat 'staff' is not an aggregated stat"}}"#,
    ),
];

/// Steam's answers to two checks of Tidal Mill Inn's records.
fn tti_checked_twice() -> Vec<(&'static str, &'static str)> {
    let once = iter::once(("200 OK", INN_SCHEMA)).chain(TTI_UNAGGREGATED);
    once.clone().chain(once).collect()
}

#[test]
fn achievements_saved_from_steam_are_a_file_that_check_finds_steam_matches() {
    let home = tempfile::tempdir().unwrap();
    let files = tempfile::tempdir().unwrap();
    let (host, _requests) = web_api(vec![("200 OK", INN_SCHEMA), ("200 OK", INN_SCHEMA)]);
    let set = web_api_variables(&host);
    let file = files.path().join("achievements.json");
    let path = file.to_str().unwrap();

    let (code, said, stderr) = steamship(
        &["achievements", "1000050", "--save", path],
        Some(home.path()),
        &set,
    );
    assert_eq!(code, Some(0_i32), "{said}{stderr}");
    assert!(
        said.contains(&format!(
            "app 1000050: 2 achievements and 2 stats saved to {path}"
        )),
        "{said}"
    );
    let saved = fs::read_to_string(&file).unwrap();
    assert!(saved.contains("\"api_name\": \"LONG_JOURNEY\""), "{saved}");
    assert!(
        saved.contains("\"name\": \"Floodwater held behind the sluice\""),
        "{saved}"
    );
    assert!(
        !saved.contains("steamcdn"),
        "icons are Steam's, not settings: {saved}"
    );

    let (checked, matching, why) = steamship(
        &["achievements", "1000050", "--check", path],
        Some(home.path()),
        &set,
    );
    assert_eq!(checked, Some(0_i32), "{matching}{why}");
    assert!(
        matching.contains("Steam's achievements and stats match the file"),
        "{matching}"
    );

    let (both, _, refused) = steamship(
        &["achievements", "1000050", "--save", path, "--json"],
        Some(home.path()),
        &set,
    );
    assert_eq!(both, Some(2_i32), "{refused}");
    assert!(refused.contains("cannot be used with"), "{refused}");

    // A file kept by hand is never written over, and Steam is not asked.
    let kept = r#"{"achievements": [{"api_name": "LONG_JOURNEY", "name": "Rising Water",
        "icon": "rising.png"}]}"#;
    fs::write(&file, kept).unwrap();
    let (held, _, losing) = steamship(
        &["achievements", "1000050", "--save", path],
        Some(home.path()),
        &set,
    );
    assert_eq!(held, Some(2_i32), "{losing}");
    assert_eq!(
        failure(&losing),
        format!(
            "{path} holds \"icon\" in achievement 1, which --save would lose. Save to another file"
        )
    );
    assert_eq!(fs::read_to_string(&file).unwrap(), kept);
}

#[test]
fn games_kits_records_are_checked_saying_what_steam_does_not_give() {
    let home = tempfile::tempdir().unwrap();
    let files = tempfile::tempdir().unwrap();
    let (host, _requests) = web_api(tti_checked_twice());
    let set = web_api_variables(&host);
    let records = |by_hand: &str| {
        let path = files.path().join("achievements.json");
        fs::write(
            &path,
            format!(
                r#"[
                {{"type": "stat", "id": "base:floors", "steam": "floors", "kind": "int"}},
                {{"type": "stat", "id": "base:staff", "steam": "staff", "kind": "int"}},
                {{"type": "achievement", "id": "base:long_journey", "steam": "LONG_JOURNEY",
                  "name": "Rising Water", "description": "Bring the millpond up to its brim.",
                  "stat": "base:floors", "at": 2}},
                {{"type": "achievement", "id": "base:crafted_alone", "steam": "CRAFTED_ALONE",
                  "name": "Bushel by Bushel", "description": "{by_hand}"}}
            ]"#
            ),
        )
        .unwrap();
        path.to_string_lossy().into_owned()
    };

    let matching = records("Bag a sack of flour by hand.");
    let (matched, said, _) = steamship(
        &["achievements", "1000050", "--check", &matching],
        Some(home.path()),
        &set,
    );
    assert_eq!(matched, Some(0_i32), "{said}");
    assert!(said.contains("2 achievements and 2 stats\n"), "{said}");
    assert!(
        said.contains(
            "  unchecked the stats' types and the achievements' progress, which Steam's Web API \
             does not give\n"
        ) && said.contains("  steam     \u{2713} 0 of 2 stats aggregated\n"),
        "a record that does not say so is a stat Steam keeps no total of: {said}"
    );
    assert!(
        said.contains("Steam's achievements and stats match the file\n"),
        "{said}"
    );

    let drifted = records("Bag the flour yourself.");
    let (refused, _, why) = steamship(
        &["achievements", "1000050", "--check", &drifted],
        Some(home.path()),
        &set,
    );
    assert_eq!(refused, Some(2_i32), "{why}");
    assert!(
        why.contains(
            "CRAFTED_ALONE: the description is \"Bag a sack of flour by hand.\" on Steam and \"Bag \
             the flour yourself.\" in the file"
        ) && why.contains("1 of 2 achievements match: 1 differs\n"),
        "{why}"
    );
}

#[test]
fn records_split_over_several_files_are_checked_as_one_and_none_may_repeat() {
    let home = tempfile::tempdir().unwrap();
    let files = tempfile::tempdir().unwrap();
    let (host, _requests) = web_api(tti_checked_twice());
    let set = web_api_variables(&host);
    let file = |name: &str, records: &str| {
        let path = files.path().join(name);
        fs::write(&path, records).unwrap();
        path.to_string_lossy().into_owned()
    };
    let stats = file(
        "stats.json",
        r#"[{"type": "stat", "steam": "floors"}, {"type": "stat", "steam": "staff"}]"#,
    );
    let building = file(
        "building.json",
        r#"[{"type": "achievement", "steam": "LONG_JOURNEY", "name": "Rising Water",
             "description": "Bring the millpond up to its brim."}]"#,
    );
    let by_hand = file(
        "by_hand.json",
        r#"[{"type": "achievement", "steam": "CRAFTED_ALONE", "name": "Bushel by Bushel",
             "description": "Bag a sack of flour by hand."}]"#,
    );

    for args in [
        vec![
            "achievements",
            "1000050",
            "--check",
            &stats,
            &building,
            &by_hand,
        ],
        vec![
            "achievements",
            "1000050",
            "--check",
            &stats,
            "--check",
            &building,
            "--check",
            &by_hand,
        ],
    ] {
        let (matched, said, _) = steamship(&args, Some(home.path()), &set);
        assert_eq!(matched, Some(0_i32), "{said}");
        assert!(
            said.contains("by_hand.json, 1 achievement and 0 stats\n")
                && said.contains("Steam's achievements and stats match the files"),
            "{said}"
        );
    }

    let (refused, _, why) = steamship(
        &[
            "achievements",
            "1000050",
            "--check",
            &by_hand,
            &stats,
            &by_hand,
        ],
        Some(home.path()),
        &set,
    );
    assert_eq!(refused, Some(2_i32), "{why}");
    assert!(
        why.contains(&format!("CRAFTED_ALONE is in both {by_hand} and {by_hand}")),
        "{why}"
    );
}

#[test]
fn an_achievements_file_that_is_not_one_or_names_another_app_is_refused_before_steam() {
    let home = tempfile::tempdir().unwrap();
    let files = tempfile::tempdir().unwrap();
    // No stand-in is given: asking Steam would be a failure to reach it, exit 1, not 2.
    let set = [
        ("STEAMSHIP_WEB_API_KEY", KEY),
        ("STEAMSHIP_NO_UPDATE_CHECK", "1"),
    ];
    let broken = files.path().join("broken.json");
    fs::write(&broken, r#"{"achievements": [{"name": "A"}]}"#).unwrap();
    let other = achievements_file(files.path(), "x");
    for (file, why) in [
        (
            broken.to_string_lossy().into_owned(),
            "achievement 1 has no \"api_name\" text",
        ),
        (other, "is for app 480, not app 1000010"),
    ] {
        let (code, _, stderr) = steamship(
            &["achievements", "1000010", "--check", &file],
            Some(home.path()),
            &set,
        );
        assert_eq!(code, Some(2_i32), "{stderr}");
        assert!(stderr.contains(why), "{why:?} in {stderr}");
    }
}

#[test]
fn a_game_writes_its_achievement_import_from_its_own_records_and_catalogues() {
    let (_temp, root) = pushed_game(None);
    let (code, stdout, stderr) = in_game(&root, &["achievements", "--write"]);
    assert_eq!(code, Some(0_i32), "{stdout}{stderr}");
    let out = root.join("steam").join("achievements");
    assert!(
        out.join("1000050-achievements-definitions.csv").is_file(),
        "{stdout}"
    );
    assert!(
        stdout.contains("achievements.json, 2 achievements and 1 stat"),
        "the records file alone of the base mod's data: {stdout}"
    );
    assert!(stdout.contains("de.po"), "{stdout}");

    let (same, unchanged, why_not) = in_game(&root, &["achievements", "--write", "--unchanged"]);
    assert_eq!(same, Some(0_i32), "{unchanged}{why_not}");

    let inner = root.join("mods").join("base");
    let (from_inside, said, why) = steamship_in(
        &inner,
        &["achievements", "--write", "--unchanged"],
        Some(&root.parent().unwrap().join("home")),
        &[("STEAMSHIP_NO_UPDATE_CHECK", "1")],
    );
    assert_eq!(
        from_inside,
        Some(0_i32),
        "found from a folder inside it: {said}{why}"
    );
}

#[test]
fn a_game_with_no_achievement_records_has_no_import_due_unless_one_is_left() {
    let (_temp, root) = pushed_game(None);
    fs::remove_file(root.join("mods/base/data/achievements.json")).unwrap();
    let unchanged = ["achievements", "--write", "--unchanged"];
    let (code, said, why) = in_game(&root, &unchanged);
    assert_eq!(code, Some(0_i32), "{why}");
    assert!(
        said.contains("the game holds no achievement records, so there is no import to write"),
        "{said}"
    );

    let import = root.join("steam").join("achievements");
    fs::create_dir_all(&import).unwrap();
    for name in [
        "1000050-achievements-definitions.csv",
        "FIVE_LEVELS_ACHIEVED.png",
        "steam-icons.json",
    ] {
        fs::write(import.join(name), "kept from before").unwrap();
    }
    let (left, _, named) = in_game(&root, &unchanged);
    assert_eq!(left, Some(2_i32), "{named}");
    for line in [
        "1000050-achievements-definitions.csv is left over",
        "FIVE_LEVELS_ACHIEVED.png is left over",
    ] {
        assert!(named.contains(line), "{line:?} in {named}");
    }
    assert!(!named.contains("steam-icons.json"), "{named}");

    fs::remove_dir_all(&import).unwrap();
    fs::write(&import, "a file where the import's folder goes").unwrap();
    let (unread, _, failed) = in_game(&root, &unchanged);
    assert_eq!(unread, Some(1_i32), "{failed}");
    assert!(failed.contains("achievements: "), "{failed}");

    let (written, _, refused) = in_game(&root, &["achievements", "--write"]);
    assert_eq!(written, Some(2_i32), "{refused}");
    assert!(
        refused.contains("--write names no files, and the game holds no achievement records"),
        "{refused}"
    );
}

#[test]
fn a_games_own_records_are_what_achievements_reads_when_it_is_named_no_files() {
    let (_temp, root) = pushed_game(None);
    let outside = tempfile::tempdir().unwrap();
    let records = outside.path().join("achievements.json");
    fs::write(&records, INN_RECORDS).unwrap();
    let (unwritten, stdout, stderr) = steamship(
        &["achievements", "480", "--write", &records.to_string_lossy()],
        Some(outside.path()),
        &[("STEAMSHIP_NO_UPDATE_CHECK", "1")],
    );
    assert_eq!(unwritten, Some(2_i32), "{stdout}{stderr}");
    assert_eq!(
        failure(&stderr),
        "--write needs --out, the folder it writes into"
    );

    let import = root.join("steam").join("achievements");
    fs::create_dir_all(&import).unwrap();
    fs::write(import.join("steam-icons.json"), "{}").unwrap();
    let host = refusing_web_api(8);
    let (asked, said, why) = in_game_with(
        &root,
        &["achievements", "--check"],
        &[("STEAMSHIP_WEB_API_KEY", KEY), (STAND_IN, &host)],
    );
    assert_ne!(asked, Some(0_i32), "{said}{why}");
    assert!(said.contains("achievements.json, 2 achievements"), "{said}");

    fs::remove_dir_all(root.join("mods")).unwrap();
    let (none, shown, reason) = in_game(&root, &["achievements", "--check"]);
    assert_eq!(none, Some(2_i32), "{shown}{reason}");
    assert_eq!(
        failure(&reason),
        "--check names no files, and the game holds no achievement records for it to read"
    );
}
