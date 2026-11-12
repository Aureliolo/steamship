//! `steamship text`.

use std::fs;
use std::path::PathBuf;

use steamship::webapi::STAND_IN;

use crate::{in_game, in_game_with, pushed_game, web_api};

#[test]
fn a_game_writes_its_text_in_every_language_and_then_finds_it_unchanged() {
    let (_temp, root) = pushed_game(None);
    let steam = root.join("steam");
    let english = fs::read(steam.join("rich_presence_english.vdf")).unwrap();
    fs::write(
        steam.join("workshop_english.json"),
        r#"{"title": "Harbours", "description": "Made by players."}"#,
    )
    .unwrap();
    fs::write(steam.join("rich_presence_french.vdf"), "left over").unwrap();
    fs::write(
        steam.join("store_english.json"),
        r#"{"blocks": [{"kind": "short", "text": "An inn."}]}"#,
    )
    .unwrap();

    let (untranslated, _, refusals) = in_game(&root, &["text"]);
    assert_eq!(untranslated, Some(2_i32), "{refusals}");
    for line in [
        "de.po does not translate \"Harbours\", under msgctxt \"steam workshop\"",
        "de.po does not translate \"At the menu\", under msgctxt \"rich presence\"",
    ] {
        assert!(refusals.contains(line), "{line:?} in {refusals}");
    }
    assert!(!steam.join("workshop").exists(), "nothing is written");

    let locale = root.join("game/locale/de.po");
    let mut german = fs::read_to_string(&locale).unwrap();
    german.push_str(
        "\nmsgctxt \"rich presence\"\nmsgid \"At the menu\"\nmsgstr \"Im Men\u{fc}\"\n\n\
         msgctxt \"steam workshop\"\nmsgid \"Harbours\"\nmsgstr \"H\u{e4}fen\"\n\n\
         msgctxt \"steam workshop\"\nmsgid \"Made by players.\"\n\
         msgstr \"Von Spielern gemacht.\"\n",
    );
    fs::write(&locale, german).unwrap();
    let (stale, said, missing) = in_game(&root, &["text", "--unchanged"]);
    assert_eq!(stale, Some(2_i32), "{said}");
    for line in [
        "rich_presence_german.vdf is missing",
        "workshop/english.json is missing",
        "workshop/german.json is missing",
        "rich_presence_french.vdf is left over",
    ] {
        assert!(missing.contains(line), "{line:?} in {missing}");
    }

    let (code, wrote, why) = in_game(&root, &["text"]);
    assert_eq!(code, Some(0_i32), "{why}");
    let told = format!("{wrote}{why}");
    for line in [
        "  wrote     rich_presence_german.vdf\n",
        "  wrote     workshop/german.json\n",
        "  removed   rich_presence_french.vdf\n",
        " holds the 3 files of the game's text in 2 languages\n",
        "store_english.json names no itemid, the store page's own, so no store upload is written",
    ] {
        assert!(told.contains(line), "{line:?} in {told}");
    }
    assert_eq!(
        wrote
            .lines()
            .filter(|line| line.starts_with("  next"))
            .collect::<Vec<_>>(),
        [
            "  next      enter each file in workshop on the Workshop's Steamworks page, one \
             language to a save",
            "  next      steamship rich-presence sends the rich presence files",
        ]
    );
    assert_eq!(
        fs::read_to_string(steam.join("rich_presence_german.vdf")).unwrap(),
        "\"lang\"\n{\n\t\"Language\"\t\t\"german\"\n\t\"Tokens\"\n\t{\n\
         \t\t\"#menu\"\t\t\"Im Men\u{fc}\"\n\t}\n}\n"
    );
    assert_eq!(
        fs::read_to_string(steam.join("workshop/german.json")).unwrap(),
        "{\n  \"language\": \"german\",\n  \"title\": \"H\u{e4}fen\",\n  \
         \"description\": \"Von Spielern gemacht.\"\n}\n"
    );
    assert!(!steam.join("rich_presence_french.vdf").exists());
    assert_eq!(
        fs::read(steam.join("rich_presence_english.vdf")).unwrap(),
        english,
        "the English kept by hand is left as it is"
    );

    let (same, unchanged, failures) = in_game(&root, &["text", "--unchanged"]);
    assert_eq!(same, Some(0_i32), "{failures}");
    assert!(
        unchanged.contains("are as the files make them"),
        "{unchanged}"
    );
}

#[test]
fn a_game_s_rich_presence_names_each_of_its_records_of_a_type_in_every_language() {
    let (_temp, root) = pushed_game(None);
    fs::write(
        root.join("steam/presence_english.json"),
        r##"{"#Status": "{#Bishop_%bishop%} plays", "#Bishop_": {"records": "bishop"}}"##,
    )
    .unwrap();
    fs::write(
        root.join("mods/base/data/bishops.json"),
        r#"[{"type": "bishop", "id": "base:alder", "name": "The Alder Witch"}]"#,
    )
    .unwrap();
    let locale = root.join("game/locale/de.po");
    let mut german = fs::read_to_string(&locale).unwrap();
    german.push_str(
        "\nmsgctxt \"rich presence\"\nmsgid \"{#Bishop_{bishop}} plays\"\n\
         msgstr \"{#Bishop_{bishop}} spielt\"\n\n\
         msgctxt \"bishop.name\"\nmsgid \"base:alder\"\nmsgstr \"Die Erlenhexe\"\n",
    );
    fs::write(&locale, german).unwrap();
    let (code, said, why) = in_game(&root, &["text"]);
    assert_eq!(code, Some(0_i32), "{why}");
    assert!(
        said.contains("  wrote     rich_presence_english.vdf\n"),
        "{said}"
    );
    assert!(
        fs::read_to_string(root.join("steam/rich_presence_english.vdf"))
            .unwrap()
            .contains("\t\t\"#Bishop_alder\"\t\t\"The Alder Witch\"\n")
    );
    assert_eq!(
        fs::read_to_string(root.join("steam/rich_presence_german.vdf")).unwrap(),
        "\"lang\"\n{\n\t\"Language\"\t\t\"german\"\n\t\"Tokens\"\n\t{\n\
         \t\t\"#Bishop_alder\"\t\t\"Die Erlenhexe\"\n\
         \t\t\"#Status\"\t\t\"{#Bishop_%bishop%} spielt\"\n\t}\n}\n"
    );

    fs::write(root.join("mods/base/data/bishops.json"), "[").unwrap();
    let (unread, _, refusal) = in_game(&root, &["text"]);
    assert_eq!(unread, Some(2_i32));
    assert!(refusal.contains("bishops.json: "), "{refusal}");
}

/// App details for app 1000050 showing `minimum` and `recommended` as its Windows operating
/// system, as Steam's store marks them up.
fn store_details(minimum: &str, recommended: &str) -> String {
    let list = |title: &str, line: &str| {
        format!(
            "<strong>{title}:</strong><br><ul class=\\\"bb_ul\\\"><li><strong>OS:</strong> \
             {line}<br></li></ul>"
        )
    };
    format!(
        r#"{{"1000050":{{"success":true,"data":{{"pc_requirements":{{"minimum":"{}","recommended":"{}"}},"mac_requirements":[],"linux_requirements":[]}}}}}}"#,
        list("Minimum", minimum),
        list("Recommended", recommended)
    )
}

/// [`pushed_game`] giving a minimum and a recommended Windows operating system and a recommended
/// amount of memory, each translated into German.
fn game_with_requirements() -> (tempfile::TempDir, PathBuf) {
    let (temp, root) = pushed_game(None);
    let steam = root.join("steam");
    fs::write(
        steam.join("store_english.json"),
        r#"{"itemid": 1000090, "blocks": [
            {"kind": "short", "text": "An inn."},
            {"kind": "field", "field": "app[content][sysreqs][windows][min][osversion]",
             "text": "Windows 10"},
            {"kind": "field", "field": "app[content][sysreqs][windows][rec][osversion]",
             "text": "Windows 11"},
            {"kind": "field", "field": "app[content][sysreqs][windows][rec][memory][amount]",
             "text": "8"}
        ]}"#,
    )
    .unwrap();
    let locale = root.join("game/locale/de.po");
    let mut german = fs::read_to_string(&locale).unwrap();
    german.push_str(
        "\nmsgctxt \"rich presence\"\nmsgid \"At the menu\"\nmsgstr \"Im Men\u{fc}\"\n\n\
         msgctxt \"store page\"\nmsgid \"An inn.\"\nmsgstr \"Gasthaus.\"\n\n\
         msgctxt \"store page\"\nmsgid \"Windows 10\"\nmsgstr \"Windows 10\"\n\n\
         msgctxt \"store page\"\nmsgid \"Windows 11\"\nmsgstr \"Windows 11, 64 Bit\"\n\n\
         msgctxt \"store page\"\nmsgid \"8\"\nmsgstr \"8\"\n",
    );
    fs::write(&locale, german).unwrap();
    (temp, root)
}

#[test]
fn recommended_requirements_leave_the_upload_for_the_basic_info_sheet() {
    let (_temp, root) = game_with_requirements();
    let steam = root.join("steam");
    let (written, wrote, unwritten) = in_game(&root, &["text"]);
    assert_eq!(written, Some(0_i32), "{unwritten}");
    assert_eq!(
        fs::read_to_string(steam.join("basic_info.json")).unwrap(),
        "{\n  \"app[content][sysreqs][windows][rec][osversion][english]\": \"Windows 11\",\n  \
         \"app[content][sysreqs][windows][rec][memory][amount][english]\": \"8\",\n  \
         \"app[content][sysreqs][windows][rec][osversion][german]\": \"Windows 11, 64 Bit\",\n  \
         \"app[content][sysreqs][windows][rec][memory][amount][german]\": \"8\"\n}\n"
    );
    let upload = fs::read_to_string(steam.join("store/german.json")).unwrap();
    assert!(
        upload.contains("\"app[content][sysreqs][windows][min][osversion]\": \"Windows 10\"")
            && !upload.contains("[rec]"),
        "{upload}"
    );
    assert!(
        wrote.contains(
            "  next      type each language's recommended system requirements in basic_info.json \
             on the store page's Basic Info form, as the upload leaves them out. Once the page \
             is public, steamship text --steam finds any missed\n"
        ),
        "{wrote}"
    );

    let english = steam.join("store_english.json");
    let without = fs::read_to_string(&english)
        .unwrap()
        .replace("[rec][osversion]", "[min][processor]")
        .replace("[rec][memory][amount]", "[min][graphics]");
    fs::write(&english, without).unwrap();
    let (stale, _, left) = in_game(&root, &["text", "--unchanged"]);
    assert_eq!(stale, Some(2_i32), "{left}");
    assert!(left.contains("basic_info.json is left over"), "{left}");
    let (removed, said, why) = in_game(&root, &["text"]);
    assert_eq!(removed, Some(0_i32), "{why}");
    assert!(said.contains("  removed   basic_info.json\n"), "{said}");
    assert!(!steam.join("basic_info.json").exists());
}

#[test]
fn screenshots_alt_text_goes_to_its_own_sheet_and_an_about_image_s_into_the_upload() {
    let (_temp, root) = pushed_game(None);
    let steam = root.join("steam");
    let english = steam.join("store_english.json");
    fs::write(
        &english,
        r#"{"itemid": 1000090, "screenshots": ["The inn at dusk."], "extras": ["about_hearth"],
            "blocks": [
            {"kind": "short", "text": "An inn."},
            {"kind": "image", "image": "about_hearth", "alt": "A fire in the hearth."}
        ]}"#,
    )
    .unwrap();
    let locale = root.join("game/locale/de.po");
    let mut german = fs::read_to_string(&locale).unwrap();
    german.push_str(
        "\nmsgctxt \"rich presence\"\nmsgid \"At the menu\"\nmsgstr \"Im Men\u{fc}\"\n\n\
         msgctxt \"store page\"\nmsgid \"An inn.\"\nmsgstr \"Gasthaus.\"\n\n\
         msgctxt \"store page\"\nmsgid \"The inn at dusk.\"\nmsgstr \"Das Gasthaus am Abend.\"\n\n\
         msgctxt \"store page\"\nmsgid \"A fire in the hearth.\"\nmsgstr \"Das Feuer im Herd.\"\n",
    );
    fs::write(&locale, german).unwrap();
    let (written, wrote, unwritten) = in_game(&root, &["text"]);
    assert_eq!(written, Some(0_i32), "{unwritten}");
    assert_eq!(
        fs::read_to_string(steam.join("screenshots.json")).unwrap(),
        "{\n  \"english 1\": \"The inn at dusk.\",\n  \
         \"german 1\": \"Das Gasthaus am Abend.\"\n}\n"
    );
    assert!(
        fs::read_to_string(steam.join("store/german.json"))
            .unwrap()
            .contains("\"app[assets][extra][0][alt_text]\": \"Das Feuer im Herd.\""),
    );
    assert!(
        wrote.contains(
            "  next      type each language's screenshot alt text in screenshots.json into the \
             screenshots' Edit Accessibility Text panel on the Store Page tab, one language at a \
             time. The upload carries none\n"
        ),
        "{wrote}"
    );

    let without = fs::read_to_string(&english)
        .unwrap()
        .replace(r#""screenshots": ["The inn at dusk."], "#, "");
    fs::write(&english, without).unwrap();
    let (stale, _, left) = in_game(&root, &["text", "--unchanged"]);
    assert_eq!(stale, Some(2_i32), "{left}");
    assert!(left.contains("screenshots.json is left over"), "{left}");
    let (removed, said, why) = in_game(&root, &["text"]);
    assert_eq!(removed, Some(0_i32), "{why}");
    assert!(said.contains("  removed   screenshots.json\n"), "{said}");
}

/// [`store_details`] with `padding` bytes before the requirements.
fn padded_details(recommended: &str, padding: usize) -> String {
    store_details("Windows 10", recommended).replacen(
        "\"data\":{",
        &format!("\"data\":{{\"padding\":\"{}\",", "x".repeat(padding)),
        1,
    )
}

#[test]
fn a_store_answer_of_megabytes_is_read_whole_and_one_past_eight_is_refused() {
    let (_temp, root) = game_with_requirements();
    let (host, _) = web_api(vec![
        ("200 OK", padded_details("Windows 11", 2 << 20)),
        ("200 OK", padded_details("Windows 11, 64 Bit", 2 << 20)),
    ]);
    let (read, said, why) = in_game_with(&root, &["text", "--steam"], &[(STAND_IN, &host)]);
    assert_eq!(read, Some(0_i32), "{said}{why}");

    let (huge, _) = web_api(vec![("200 OK", padded_details("Windows 11", 9 << 20))]);
    let (refused, _, failure) = in_game_with(&root, &["text", "--steam"], &[(STAND_IN, &huge)]);
    assert_eq!(refused, Some(1_i32), "{failure}");
    assert!(
        failure.contains(
            "The answer from Steam's store could not be read whole: it is longer than 8 MiB"
        ),
        "{failure}"
    );
}

#[test]
fn the_store_page_is_read_in_every_language_and_each_requirement_it_lacks_named() {
    let (_temp, root) = game_with_requirements();
    let (host, requests) = web_api(vec![
        ("200 OK", store_details("Windows 10", "Windows 11")),
        ("200 OK", store_details("Windows 10", "Windows 11")),
    ]);
    let (lacking, said, why) = in_game_with(&root, &["text", "--steam"], &[(STAND_IN, &host)]);
    assert_eq!(lacking, Some(2_i32), "{said}{why}");
    let told = format!("{said}{why}");
    for line in [
        "german: the recommended windows osversion is not on the store page",
        "\"Windows 11, 64 Bit\"",
        "Steam rewords windows memory amount on the store page, so they are not compared",
        "type the recommended ones from basic_info.json on the store page's Basic Info form. \
         Upload the files in store/ on its Localization tab for the minimum ones, and run it again",
    ] {
        assert!(told.contains(line), "{line:?} in {told}");
    }
    assert!(!told.contains("english: the"), "{told}");
    for language in ["english", "german"] {
        let asked = requests.recv().unwrap();
        assert!(
            asked.starts_with(&format!("GET /api/appdetails?appids=1000050&l={language} ")),
            "{asked}"
        );
    }

    let (fixed, _) = web_api(vec![
        ("200 OK", store_details("Windows 10", "Windows 11")),
        ("200 OK", store_details("Windows 10", "Windows 11, 64 Bit")),
    ]);
    let (shown, all, unshown) = in_game_with(&root, &["text", "--steam"], &[(STAND_IN, &fixed)]);
    assert_eq!(shown, Some(0_i32), "{unshown}");
    assert!(
        all.contains(
            "app 1000050's store page shows the 4 system requirements of the files in 2 languages"
        ),
        "{all}"
    );
}

#[test]
fn a_store_page_steam_does_not_show_or_cannot_give_is_named() {
    let (_temp, root) = game_with_requirements();
    let (host, _) = web_api(vec![("200 OK", r#"{"1000050":{"success":false}}"#)]);
    let (private, _, hidden) = in_game_with(&root, &["text", "--steam"], &[(STAND_IN, &host)]);
    assert_eq!(private, Some(2_i32), "{hidden}");
    assert!(
        hidden.contains(
            "Steam shows no public store page for app 1000050, so its system requirements can \
             be checked only on its Basic Info form"
        ),
        "{hidden}"
    );

    let (failing, _) = web_api(vec![("500 Internal Server Error", "")]);
    let (broken, _, failure) = in_game_with(&root, &["text", "--steam"], &[(STAND_IN, &failing)]);
    assert_eq!(broken, Some(1_i32), "{failure}");
    assert!(
        failure.contains("Steam's store answered HTTP 500"),
        "{failure}"
    );
}

#[test]
fn a_store_page_check_of_a_game_giving_no_requirements_is_refused() {
    let (_temp, root) = pushed_game(None);
    fs::write(
        root.join("steam/store_english.json"),
        r#"{"blocks": [{"kind": "short", "text": "An inn."}]}"#,
    )
    .unwrap();
    let (code, _, why) = in_game(&root, &["text", "--steam"]);
    assert_eq!(code, Some(2_i32), "{why}");
    assert!(
        why.contains(
            "store_english.json gives no system requirements, so there are none to look for on \
             the store page"
        ),
        "{why}"
    );
}

#[test]
fn a_file_of_steam_s_text_that_cannot_be_read_is_named() {
    let (_temp, root) = pushed_game(None);
    fs::create_dir_all(root.join("steam/workshop_english.json")).unwrap();
    let (code, _, why) = in_game(&root, &["text"]);
    assert_eq!(code, Some(1_i32), "{why}");
    assert!(why.contains("workshop_english.json: "), "{why}");

    fs::remove_dir(root.join("steam/workshop_english.json")).unwrap();
    fs::remove_file(root.join("steam/rich_presence_english.vdf")).unwrap();
    fs::write(root.join("steam/store_english.json"), "{}").unwrap();
    fs::write(
        root.join("steam/store"),
        "a file where the store's folder goes",
    )
    .unwrap();
    let (kept, _, refusal) = in_game(&root, &["text"]);
    assert_eq!(kept, Some(1_i32), "{refusal}");
    assert!(refusal.contains("store: "), "{refusal}");
}

#[test]
fn a_game_with_no_english_for_steam_s_text_is_told_so_and_has_nothing_written_or_removed() {
    let (_temp, root) = pushed_game(None);
    let steam = root.join("steam");
    fs::remove_file(steam.join("rich_presence_english.vdf")).unwrap();
    let elsewhere = [
        "store/english.json",
        "workshop/german.json",
        "ai_disclosure.json",
        "rich_presence_german.vdf",
    ];
    for name in elsewhere {
        let path = steam.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, "written by the game's own tool").unwrap();
    }

    let (same, kept, why_not) = in_game(&root, &["text", "--unchanged"]);
    assert_eq!(same, Some(0_i32), "{kept}{why_not}");
    assert!(!why_not.contains("left over"), "{why_not}");
    let (code, said, why) = in_game(&root, &["text"]);
    assert_eq!(code, Some(0_i32), "{why}");
    let all = format!("{said}{why}");
    for line in [
        "there is no store_english.json, workshop_english.json, presence_english.json or \
         rich_presence_english.vdf beside the build scripts, so there is no English to write \
         from",
        " holds none of the game's text\n",
    ] {
        assert!(all.contains(line), "{line:?} in {all}");
    }
    assert!(!all.contains("  next"), "{all}");
    assert!(!all.contains("removed"), "{all}");
    for name in elsewhere {
        assert_eq!(
            fs::read_to_string(steam.join(name)).unwrap(),
            "written by the game's own tool",
            "{name}"
        );
    }
}

/// The template `text --template` prints for [`TEMPLATED_STORE`], [`TEMPLATED_WORKSHOP`] and
/// [`TEMPLATED_PRESENCE`]: every entry once, by context and English.
const TEMPLATE: &str = concat!(
    "# The Steam text of a game, for its translation template, by steamship: the store page ",
    "under \"store page\",\n",
    "# the Workshop page under \"steam workshop\", rich presence under \"rich presence\" and\n",
    "# the achievements under \"steam achievements\".\n",
    "msgid \"\"\nmsgstr \"\"\n\"Content-Type: text/plain; charset=UTF-8\\n\"\n\n",
    "#. What Steam shows the player's friends. Keep every {field}, which Steam fills with a ",
    "value, and every {#Token_{field}} whole, which Steam fills with that token's own line.\n",
    "msgctxt \"rich presence\"\nmsgid \"At the menu\"\nmsgstr \"\"\n\n",
    "#. What Steam shows the player's friends. Keep every {field}, which Steam fills with a ",
    "value, and every {#Token_{field}} whole, which Steam fills with that token's own line.\n",
    "msgctxt \"rich presence\"\nmsgid \"Grinding at {mill}\"\nmsgstr \"\"\n\n",
    "#. On the game's Steam Workshop page, beside its picture: the title a short headline, the ",
    "description one or two lines. Plain prose, no markup.\n",
    "msgctxt \"steam workshop\"\nmsgid \"Harbours\"\nmsgstr \"\"\n\n",
    "#. On the game's Steam Workshop page, beside its picture: the title a short headline, the ",
    "description one or two lines. Plain prose, no markup.\n",
    "#, max-length:200\n",
    "msgctxt \"steam workshop\"\nmsgid \"Made by players.\"\nmsgstr \"\"\n\n",
    "#. On the game's Steam store page. Plain prose, no markup.\n",
    "#, max-length:300\n",
    "msgctxt \"store page\"\nmsgid \"A mill on a tidal river.\"\nmsgstr \"\"\n\n",
    "#. A line of the game's Steam store page outside its description, such as a system ",
    "requirement. Plain text, every product and model name kept as written.\n",
    "msgctxt \"store page\"\nmsgid \"All rights reserved.\"\nmsgstr \"\"\n\n",
    "#. On the game's Steam store page. Plain prose, no markup.\n",
    "msgctxt \"store page\"\nmsgid \"Grind \\\"grain\\\" at high water.\"\nmsgstr \"\"\n\n",
    "#. The game's statement on its Steam store page of how AI was used in making it, shown ",
    "under About This Game. Plain prose, no markup, and the same plain claim in every ",
    "language.\n",
    "msgctxt \"store page\"\nmsgid \"Made with AI assistance for coding.\"\nmsgstr \"\"\n\n",
    "#. Alt text for a screenshot on the game's Steam store page, read aloud by screen readers. ",
    "Plain prose.\n",
    "msgctxt \"store page\"\nmsgid \"The harbour at dawn.\"\nmsgstr \"\"\n\n",
    "#. Alt text for a picture in the About This Game of the game's Steam store page, read aloud ",
    "by screen readers. Plain prose.\n",
    "msgctxt \"store page\"\nmsgid \"The wheel turning.\"\nmsgstr \"\"\n",
);

const TEMPLATED_STORE: &str = r#"{
  "itemid": 1000090,
  "disclosure": "Made with AI assistance for coding.",
  "screenshots": ["The harbour at dawn."],
  "extras": ["about_mill"],
  "blocks": [
    {"kind": "short", "text": "A mill on a tidal river."},
    {"kind": "paragraph", "text": "Grind \"grain\" at high water."},
    {"kind": "image", "image": "about_mill", "alt": "The wheel turning."},
    {"kind": "field", "field": "app[content][legal]", "text": "All rights reserved."},
    {"kind": "image", "image": "about_mill"},
    {"kind": "paragraph", "text": "A mill on a tidal river."}
  ]
}"#;
const TEMPLATED_WORKSHOP: &str = r#"{"title": "Harbours", "description": "Made by players."}"#;
const TEMPLATED_PRESENCE: &str = r##"{"#menu": "At the menu", "#grinding": "Grinding at %mill%",
    "#Mill_": {"records": "achievement"}}"##;

#[test]
fn a_games_steam_text_is_printed_as_a_template_its_own_takes_in() {
    let (_temp, root) = pushed_game(None);
    let steam = root.join("steam");
    fs::write(steam.join("store_english.json"), TEMPLATED_STORE).unwrap();
    fs::write(steam.join("workshop_english.json"), TEMPLATED_WORKSHOP).unwrap();
    fs::write(steam.join("presence_english.json"), TEMPLATED_PRESENCE).unwrap();

    let (code, template, told) = in_game(&root, &["text", "--template"]);
    assert_eq!(code, Some(0_i32), "{told}");
    assert_eq!(template, TEMPLATE, "{told}");
    assert!(
        told.contains("the template holds 10 messages of Steam's text"),
        "{told}"
    );
    assert!(
        !steam.join("workshop").exists(),
        "nothing is written beside the scripts"
    );
}

#[test]
fn achievements_kept_in_steamships_own_file_are_in_the_template_by_their_english() {
    let (_temp, root) = pushed_game(Some("records = [\"steam/achievements.json\"]\n"));
    fs::write(
        root.join("steam/achievements.json"),
        r#"{"app": 1000050, "achievements": [
            {"api_name": "FIRST_GRIST", "name": "First grist",
             "description": "Grind your first sack of grain."}]}"#,
    )
    .unwrap();

    let (code, template, told) = in_game(&root, &["text", "--template"]);
    assert_eq!(code, Some(0_i32), "{told}");
    assert!(
        template.contains(
            "#. The name of the game's Steam achievement FIRST_GRIST. Plain text.\n\
             msgctxt \"steam achievements\"\nmsgid \"First grist\"\nmsgstr \"\"\n"
        ),
        "{template}"
    );
    assert!(
        template.contains(
            "msgctxt \"steam achievements\"\nmsgid \"Grind your first sack of grain.\"\n"
        ),
        "{template}"
    );
}

#[test]
fn achievements_that_name_their_contexts_are_left_out_of_the_template() {
    let (_temp, root) = pushed_game(Some("records = [\"steam/achievements.json\"]\n"));
    fs::write(
        root.join("steam/achievements.json"),
        r#"{"app": 1000050, "achievements": [
            {"api_name": "FIRST_GRIST", "name": "First grist",
             "name_context": "deed.first_grist",
             "description": "Grind your first sack of grain.",
             "description_context": "deed.first_grist.about"},
            {"api_name": "HUSH", "name": "Hush"}]}"#,
    )
    .unwrap();

    let (code, template, told) = in_game(&root, &["text", "--template"]);
    assert_eq!(code, Some(0_i32), "{told}");
    assert!(
        template.contains("msgctxt \"steam achievements\"\nmsgid \"Hush\"\nmsgstr \"\"\n"),
        "{template}"
    );
    assert!(!template.contains("First grist"), "{template}");
    assert!(
        !template.contains("Grind your first sack of grain."),
        "{template}"
    );
}

#[test]
fn a_short_description_steam_would_cut_is_refused_before_any_template() {
    let (_temp, root) = pushed_game(None);
    let long = "A mill. ".repeat(40);
    fs::write(
        root.join("steam/store_english.json"),
        format!(r#"{{"blocks": [{{"kind": "short", "text": "{long}"}}]}}"#),
    )
    .unwrap();

    let (code, template, told) = in_game(&root, &["text", "--template"]);
    assert_eq!(code, Some(2_i32), "{told}");
    assert_eq!(template, "");
    assert!(
        told.contains(&format!(
            "store_english.json: {long:?} is 320 characters in english, past the 300 Steam shows. \
             Shorten it"
        )),
        "{told}"
    );
}

#[test]
fn a_translation_past_steams_room_is_refused_and_nothing_is_written() {
    let (_temp, root) = pushed_game(None);
    fs::write(root.join("steam/workshop_english.json"), TEMPLATED_WORKSHOP).unwrap();
    let long = "Von Spielern gemacht. ".repeat(10);
    let locale = root.join("game/locale/de.po");
    let mut german = fs::read_to_string(&locale).unwrap();
    german.push_str(
        "\nmsgctxt \"rich presence\"\nmsgid \"At the menu\"\nmsgstr \"Im Men\u{fc}\"\n\n\
         msgctxt \"steam workshop\"\nmsgid \"Harbours\"\nmsgstr \"H\u{e4}fen\"\n\n\
         msgctxt \"steam workshop\"\nmsgid \"Made by players.\"\n",
    );
    german.push_str(&["msgstr \"", &long, "\"\n"].concat());
    fs::write(&locale, german).unwrap();

    let (code, _, told) = in_game(&root, &["text"]);
    assert_eq!(code, Some(2_i32), "{told}");
    assert!(
        told.contains(&format!(
            "de.po: {long:?} is 220 characters in german, past the 200 Steam shows. Shorten it"
        )),
        "{told}"
    );
    assert!(!root.join("steam/workshop").exists(), "nothing is written");
}
