# Shipping a game

Run in a game's folder, or any folder inside it, steamship needs none of the game's files named.
It finds the game by its `steamship.toml`, or else by `steam/app_build.vdf`, in the folder or one
above it, and takes everything else from where the game keeps it.

```sh
steamship check                 # the scripts, records and rich presence against Steam's
steamship upload --preview      # the pushed commit, built and checked; Valve's dry run of it
steamship upload                # the same, uploaded and set live
steamship workshop              # each example mod, as its Workshop item
```

## Where steamship looks

Without a `steamship.toml`, a game is laid out the way a games-kit game is:

| What | Where |
| --- | --- |
| The app build script and its depot scripts | `steam/app_build.vdf` |
| The settings snapshot | `steam/settings.vdf`, beside the scripts |
| Every language's rich presence | `steam/rich_presence_<language>.vdf` |
| The achievement editor's import and its icon receipt | `steam/achievements/` |
| The Workshop items' IDs | `steam/workshop_items.json` |
| The records: achievements, stats, sets, leaderboards | each JSON file in `mods/base/data` |
| Every language's catalogue | each `.po` file in `game/locale` |
| The example mods, each a Workshop item | each folder in `modding/` with a `manifest.json` |
| The version | `config/version` in `project.godot` |

Of the record files, each command takes those holding what it reads: games-kit's records typed
`achievement`, `stat` and `achievement_set` for `achievements`, `leaderboard` for
`leaderboards`, or steamship's own files of them. A folder the layout expects that is not there
is left out.

## `steamship.toml`

The file names what the game keeps elsewhere, how it is built, and the one steamship it ships
with. Every setting is optional, each path is relative to the file and stays inside the game, and
a setting steamship does not take is refused.

```toml
steamship = "0.9.30"
build = ["python", "tools/build.py"]
records = ["mods/base/data"]
locale = "mods/base/locale"

[workshop]
footer = "The game's own example mod, dedicated to the public domain under CC0 1.0."
```

- `steamship`: the version the game ships with. Any other steamship run in the game refuses
  every command, so that a game is never shipped by one it was not checked against.
- `build`: the program and its arguments that build the game into the folders its depots ship,
  run in the game's folder. Without it, the content goes up as it is.
- `branch`: the one branch the app build script may set live, if not `testing`. The default
  branch is never named here: steamship sets it through the Web API while the game is
  unreleased, and the publisher sets it once it is released.
- `cores`: the most processors steamcmd and the build run on, if not 4, so that shipping leaves
  the rest of the machine to whoever works on it. Windows and Linux hold them to it; macOS
  holds no program to fewer than all.
- `script`: the app build script, if not `steam/app_build.vdf`. The settings snapshot, rich
  presence, import and items file are then looked for beside it.
- `records`: folders and files of records, if not `mods/base/data`.
- `locale`: the folder of catalogues, if not `game/locale`.
- `import`: the achievement editor's import, if not `achievements` beside the scripts.
- `workshop.mods`: the folder of example mods, if not `modding`.
- `workshop.items`: the file the items' IDs are kept in, if not `workshop_items.json` beside the
  scripts.
- `workshop.footer`: text set below every example mod's own description.

A folder or file the file names has to be there.

## Shipping

`steamship upload` with no script ships the game, in this order, and stops at the first step
that fails:

1. The tree is refused while it holds anything its commit does not, untracked files among it, or
   while origin's default branch is at another commit: what goes up is a pushed commit.
2. The version is `project.godot`'s, unless `--version` gives another.
3. The app build script has to set the game's branch live, and no other. The settings snapshot
   has to be there, and its install folder letters and digits alone.
4. The `build` command runs, held to `cores` processors as steamcmd is, and the tree is refused
   again if it changed it. So is every file the depots map that the build did not write, make or
   rename, such as an export left over from an earlier build. On Windows a copy keeps its
   source's times and is told only by when it was made, so a build that copies over a file
   already there should empty the folder first.
5. Steam's settings are compared with the snapshot, and the achievements, stats, leaderboards and
   rich presence with the game's files, each that the game has.
6. The build is uploaded, and set live on its branch and, while Steam shows the game unreleased,
   on the default branch too.
7. The branch's description becomes the version, and the branches and last builds are shown.

`--preview` stops after Valve's dry run, with nothing uploaded. `steamship check` with no script
runs the comparisons alone, after checking the scripts and the branch they set live, and needs
no login.

## The other commands

Each command run in a game takes from it whatever is not given:

- `steamship settings --save` and `--check` with no file use the snapshot. Either refuses an
  install folder of anything but letters and digits.
- `steamship achievements --check` and `--write` with no files read the game's records and
  catalogues. `--check` compares the icons with the receipt once there is one, and
  `--imported` alone writes it.
- `steamship leaderboards --check` with no files, or `--create` alone, reads the game's
  leaderboard records.
- `steamship rich-presence` with neither the app nor files sends every language's file, from a
  pushed tree only; `--preview` checks them and sends nothing.
- `steamship workshop` uploads every example mod, or the one `--name` names, from a pushed tree
  only. Its title, description and version come from its `manifest.json`, and its preview is
  the `preview.png` beside it, which every example mod needs. A new item needs `--visibility`,
  which is given to the items being made alone: one already published keeps its own.
- `steamship achievements` alone lists the game's achievements and stats on Steam, with the
  share of players who have each achievement and each aggregated stat's total.
- `steamship builds` and `steamship promote` take the game's app.
- `steamship check export/windows` checks a build's folder as soon as it is built, judging its
  Windows files against the redistributables the snapshot ticks.

## A demo

Steam keeps a demo as an app of its own, typed Demo in General Application Settings with the full
game as its base game, and with depots and builds of its own. A game ships its demo from the same
folder and commit, by its own app build script at `steam/demo/app_build.vdf`, or as
`steamship.toml` names it:

```toml
[demo]
script = "steam/demo/app_build.vdf"
build = ["python", "tools/build.py", "--demo"]
branch = "testing"
```

- `demo.script`: the demo's app build script, if not `steam/demo/app_build.vdf`. Its settings
  snapshot is the `settings.vdf` beside it.
- `demo.build`: the program and its arguments that build the demo. Without it, the demo's
  content goes up as it is.
- `demo.branch`: the one branch the demo's script may set live, if not the game's.
- `demo.feature`: the custom feature of the Godot export presets that make the demo, if not
  `demo`. The game tells it is the demo with `OS.has_feature("demo")`.

`check`, `upload`, `builds`, `settings`, `rich-presence` and `promote` take `--demo` to work on
the demo in place of the full game, each as it does for the game:

```sh
steamship settings --save --demo   # the demo's snapshot, in steam/demo/settings.vdf
steamship upload --demo            # the pushed commit, built as the demo, uploaded and set live
steamship builds --demo --json     # the demo's branches and last builds
```

Steamworks names a new demo's install folder after the game followed by a space and Demo, which
the first `settings --save --demo` refuses: rename it to letters and digits alone, such as
`SpacewarDemo`, under Installation, General Installation, and save the settings again.

So that neither ever goes up into the other, both the game's upload and the demo's are refused
while the two scripts name the same app or share a depot. A demo script steamship cannot read,
such as one kept at an `AppID` of 0 until the demo's app exists, holds back only the demo's
commands, not the full game's. The demo's check and upload are refused too while
its snapshot shows an app not typed Demo, or with another base game than the one the game's
script names. For the demo and the game alike, a depot the script uploads that is not among the
app's own is refused, and so is a system the app's depots install on that no launch option runs
on.

When the game has Godot's `export_presets.cfg`, each preset's `export_path` is matched against
the depots' mappings as steamcmd matches files, whether or not the export is there yet. The full
game's depots are refused while they ship an export made with the demo's feature, and the
demo's while they ship one made without it, so a game whose depot maps all of `export/` has to
keep the demo's exports out of it, such as by mapping `windows/*` alone.

The demo's rich presence is the game's, sent from the files beside the game's script. Its
achievements and leaderboards are left out of `check --demo`, since a demo is best without
them and the game's records are the full game's. `steamship drm-wrap` wraps the demo's
executable with the demo's script, so that it runs under the demo's app. `steamship init` makes
the demo's scripts, each depot's folder named from the game's folder:

```sh
steamship init 1000070 --depot 1000071=export/demo/windows \
  --depot 1000072=export/demo/linux --folder steam/demo
```

Until the demo has a build live on its default branch, it has no other branch, so `check --demo`
says the script's branch is not on Steam yet. The first `upload --demo --default` then sets the
build live on default alone, and names the `promote` command for the branch once it is made in
Steamworks under SteamPipe, Builds, as [Uploading a build](uploading.md#uploading-a-build)
describes.

## Text in every language

`steamship text` writes what Steam shows of the game in every language its catalogues are in,
from English kept by hand beside the build scripts. `--unchanged` writes nothing and exits 2
naming each file that differs, is missing or is left over, so a gate can run it.

| English | Written beside the scripts | Where it goes |
| --- | --- | --- |
| `store_english.json` | `store/<language>.json` | the store page's Localization tab, every file in one upload |
| its recommended system requirements | `basic_info.json` | the store page's Basic Info form, one language at a time |
| its `screenshots` | `screenshots.json` | the screenshots' alt text panel on the Store Page tab, one language at a time |
| its `disclosure` | `ai_disclosure.json` | the content survey's Generative AI section, one box a language |
| `workshop_english.json` | `workshop/<language>.json` | the Workshop's Steamworks page, one language to a save |
| `presence_english.json` | `rich_presence_<language>.vdf` | `steamship rich-presence` |

`store_english.json` holds the store page's `itemid`, which its Localization tab's export names
and which is not the app's, the `disclosure`, each screenshot's alt text in the store's order of
them, the store's own list of its images as `extras`, and the page's text as `blocks`, each with
its `kind`:

```json
{
  "itemid": "1000090",
  "disclosure": "Made with AI assistance for coding.",
  "screenshots": ["The first screenshot, as a screen reader says it.", "The second."],
  "extras": ["about_board", "about_score"],
  "blocks": [
    { "kind": "short", "text": "The short description." },
    { "kind": "heading", "text": "A heading in About This Game" },
    { "kind": "paragraph", "text": "A paragraph." },
    { "kind": "image", "image": "about_score", "alt": "What the image shows." },
    { "kind": "item", "text": "One item of a list" },
    { "kind": "field", "field": "app[content][legal]", "text": "The legal line." }
  ]
}
```

A field or block of another name is refused, naming it. An image is named as the Description
tab's custom images name it, the same in every language.

Steam numbers an image's alt text by the image's place in the store's own list of its images,
counted from 0, as `app[assets][extra][0][alt_text]`. That list need not follow the About's
order, so with any `alt` given, `extras` names them all in the store's order. The store page's
edit page holds it in the `data-rggamepageconfig` of its `application_config`, as
`rgExtraAssetsData.rgExtraAssets`, each entry's `extra_asset_name` the image's name after
`{STEAM_APP_IMAGE}/extras/`. It holds every image uploaded, the About's own and any it does not
show, and `extras` lists them all. The Store Page tab's extra assets list is in the About's
order instead, and so is no guide. Every image the About shows has to be in `extras`. Each image it
lists gets its `alt`, or an empty one, which clears what the store held, such as for a
decorative image.

The upload carries no screenshot's alt text, so the screenshots' go into `screenshots.json`, as
`german 1` for the first one in German, to be typed into their panel.

Items in a row make one list. A `field` is any other text Steam shows in the player's language,
such as a system requirement, by its name on the form. Until the `itemid` is set no store upload
is written, since one naming no item cannot go up.

A system requirement's field is named for its platform, its tier and its part, as
`app[content][sysreqs][windows][min][osversion]`. The Localization tab takes the minimum ones,
`[min]`, and leaves out every recommended one, `[rec]`, without saying so, and its export leaves
them out too. So the recommended ones go into `basic_info.json` instead, each under its name on the
Basic Info form, which ends in the language, as `app[content][sysreqs][windows][rec][osversion][german]`.
An upload on the Localization tab leaves the recommended ones typed there as they are, so the
form can be filled before or after it.

Once the store page is public, Coming Soon included, `steamship text --steam` reads it in each
language and exits 2 naming every requirement, minimum or recommended, that it does not show as
the files give it. Before then Steam shows them only on the Basic Info form. The store page
rewords memory, storage, the DirectX version and VR support in its own words, so those are named
as not compared; the operating system, processor, graphics, sound card and notes are compared
word for word.

`workshop_english.json` holds the Workshop page's `title` and `description`.
`presence_english.json` holds each rich presence token and its English line. A name ending in
`_` with `{"records": "bishop"}` makes a token for each of the game's records of that type with a
name: `#Bishop_alder` for `base:alder`, its line the record's name. Without
`presence_english.json`, a `rich_presence_english.vdf` kept by hand is the English, and is left
as it is.

Each catalogue translates the English under the context `store page`, `steam workshop` or `rich
presence`, with the English line as the `msgid`; a rich presence line's fields are written
`{field}` there, not Steam's `%field%`. A record's name is translated as games-kit's are, under
`bishop.name` and the record's ID. Anything a catalogue does not translate, or marks fuzzy, is
refused and nothing is written. A file that would no longer be written is removed, but only of a
kind whose English the game gives: without `store_english.json`, for instance, the store's files
and `ai_disclosure.json` are left to whatever else writes them.
