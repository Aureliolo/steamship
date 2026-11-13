# Shipping a game

In a game's folder, or any folder inside it, steamship finds the game's files itself. The game is
the nearest folder, from where steamship runs upwards, that holds a `steamship.toml` or else
`steam/app_build.vdf`.

```sh
steamship check                 # the scripts, records and rich presence against Steam's
steamship upload --dry-run      # Valve's dry run of the pushed commit, built and checked
steamship upload                # the same, uploaded and set live
steamship workshop              # each example mod, as its Workshop item
```

## Where steamship looks

Without a `steamship.toml`, steamship expects the layout of a game that keeps its content as
[games-kit records](uploading.md#games-kit-records) in JSON files:

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
| The version | the newest release heading in `CHANGES.md`, or else `config/version` in `project.godot` |

steamship skips any of these the game does not have.

### Record files

Each command reads only the records of its own types. `achievements` reads games-kit's records
typed `achievement`, `stat` and `achievement_set`. `leaderboards` reads those typed
`leaderboard`. Both also read steamship's own record files.

## `steamship.toml`

The file names where the game keeps its files, how it is built and which steamship version it
ships with. Every setting is optional. Each path is relative to the file and stays inside the
game. A setting steamship does not take is refused.

```toml
steamship = "0.9.30"
build = ["python", "tools/build.py"]
records = ["mods/base/data"]
locale = "mods/base/locale"

[workshop]
footer = "The game's own example mod, dedicated to the public domain under CC0 1.0."
```

### steamship and the build

- `steamship`: the one steamship version the game is checked against and ships with. Any other
  version refuses every command that takes the game from its folder.
- `build`: the program and its arguments that build the game into the folders its depots ship,
  run in the game's folder. Without it, the content goes up as it is.
- `cores`: the most processors steamcmd and the build run on. The default is 4. The other
  processors stay free for other work.

On Windows and Linux, steamcmd and the build run on at most `cores` processors. On macOS they run
on every processor.

A command given its app or files works as it does outside a game, whatever `steamship` pins.

### The branch

- `branch`: the one branch the app build script may set live, if not `testing`.
- `description`: the branch's description that players see when they choose it, if not the
  version each upload ships.
- `released`: `true` once the game is released. An upload then sets the build live on the game's
  branch only, never on default. Under `[demo]` it does the same for the demo's app.
- `green`: a branch on origin that your tests move forward to each commit they pass. An upload
  then ships only a commit on that branch, unless `--allow-ungreen` is given.

The default branch is never named here. steamship sets it through the Web API while the game is
unreleased and `released` is not set. Once the game is released, the publisher sets it.

### The game's files

- `script`: the app build script, if not `steam/app_build.vdf`. The settings snapshot, rich
  presence, import and items file are then looked for beside it.
- `records`: folders and files of records, if not `mods/base/data`.
- `locale`: the folder of catalogues, if not `game/locale`.
- `import`: the achievement editor's import, if not `achievements` beside the scripts.

A folder or file that `steamship.toml` names has to be there.

### The example mods

- `workshop.mods`: the folder of example mods, if not `modding`.
- `workshop.items`: the file the items' IDs are kept in, if not `workshop_items.json` beside the
  scripts.
- `workshop.footer`: text set below every example mod's own description.
- `workshop.held_until_launch`: `true` to send no Workshop item for the game's app while Steam
  shows the game unreleased. The run that readies the launch gives `--for-launch`.

## Shipping

With no script, `steamship upload` ships the game's pushed commit in these steps. It stops at the
first that fails:

1. The tree has to be the pushed commit.
2. The version is read from `CHANGES.md` or `project.godot`.
3. The app build script and the settings snapshot are checked.
4. The `build` command runs, held to `cores` processors as steamcmd is. The tree is then checked
   again.
5. Steam's settings are compared with the snapshot. The achievements, stats, leaderboards and
   rich presence the game has are compared with its files.
6. The build is uploaded and set live on its branch. While Steam shows the game unreleased, it
   goes live on the default branch too.
7. The branch's description becomes the version, or `description`. The branches and last builds
   are shown.

### The tree and the scripts

The tree is refused while it holds anything its commit does not, untracked files included. It is
refused too while origin's default branch is at another commit.

With `green` set, the commit has to be on that branch of origin. The check runs before the build.
When it refuses, it names the commit and where the branch is. `--dry-run` skips the check.
`--allow-ungreen` uploads a commit that is not on the branch, such as a hotfix.

The app build script has to set the game's branch live, and no other. The settings snapshot has
to be there, with an install folder of letters and digits alone.

### The version

The version comes from the first second-level heading in `CHANGES.md` that starts with one, such
as `## 1.2.0, 2026-10-30`, `## v1.2.0` or `## [1.2.0]`. With none, it is `project.godot`'s.
`--version` gives another. A version of all zeros is refused, such as the 0.0.0 a project
keeps in `project.godot` between releases.

### After the build

The tree is refused again if the build changed it. So is every file the depots map that the build
did not write, make or rename, such as an export from an earlier build.

On Windows a copied file keeps its source's times. steamship can tell it is new only from its
creation time. A build that copies over a file already there should empty the folder first.

### The first upload

On the first upload of an app with no branch but default, the build goes live on default alone.
steamship then names the commands that set it live on the game's branch and describe it. Run them
once that branch is made in Steamworks.

### Preview and check

`--dry-run` stops after Valve's dry run, with nothing uploaded. `steamship check` with no script
runs the comparisons alone, after checking the scripts and the branch they set live. It needs no
login.

## The other commands

Each command run in a game takes from it whatever is not given:

- `steamship settings --save` and `--check` with no file use the snapshot. Either refuses an
  install folder of anything but letters and digits.
- `steamship rich-presence` with neither the app nor files sends every language's file, from a
  pushed tree only. `--dry-run` checks them and sends nothing.
- `steamship builds`, `steamship promote` and `steamship fetch` take the game's app.

### Achievements and leaderboards

- `steamship achievements --check` and `--write` with no files read the game's records and
  catalogues. Once there is an icon receipt, which only `--imported` writes, `--check` compares
  the icons with it.
- `steamship achievements` alone lists the game's achievements and stats on Steam. It shows the
  share of players who have each achievement and each aggregated stat's total.
- `steamship leaderboards --check` with no files, or `--create` or `--delete-missing` alone,
  reads the game's leaderboard records. For a leaderboard renamed in the records, `--create`
  makes the new one and `--delete-missing` deletes the old one while the game is unreleased.

### Workshop items

`steamship workshop` uploads every example mod, or the one `--name` names, from a pushed tree
only. Its title, description and version come from its `manifest.json`. Its preview is the
`preview.png` beside it, which every example mod needs.

A new item needs `--visibility`, which applies only to items being made. One already published
keeps its visibility.

A game that sets `workshop.held_until_launch` has steamship ask Steam first. Until Steam shows the
game released, steamship refuses any item for its app, unless `--for-launch` is given. That holds
for an item given by its fields or by a script.

### A build's folder

`steamship check export/windows` checks a build's folder as soon as it is built. It judges the
folder's Windows files against the redistributables the snapshot ticks. It refuses the folder
without the Steam Input action manifest the snapshot names. It refuses it too when it holds none
of the programs its launch options start, as after renaming the game.

A folder that only the demo's depots ship is judged against the demo's snapshot. `--demo` asks
for that whichever depots ship it.

The check also reads the game's rich presence files as `rich-presence --dry-run` does. A push gate
then refuses a line Steam cannot fill, without asking Steam.

## A demo

Steam keeps a demo as an app of its own, with depots and builds of its own. In General
Application Settings it is typed Demo, with the full game as its base game. A game ships its demo
from the same folder and commit, by the demo's own app build script. That script is at
`steam/demo/app_build.vdf`, or where `steamship.toml` names it.

### The demo's settings

```toml
[demo]
script = "steam/demo/app_build.vdf"
build = ["python", "tools/build.py", "--demo"]
branch = "testing"
description = "testing"
```

- `demo.script`: the demo's app build script, if not `steam/demo/app_build.vdf`. Its settings
  snapshot is the `settings.vdf` beside it.
- `demo.build`: the program and its arguments that build the demo. Without it, the demo's
  content goes up as it is.
- `demo.branch`: the one branch the demo's script may set live, if not the game's.
- `demo.description`: what the demo's branch is described as, if not the game's `description`.
- `demo.feature`: the custom feature of the Godot export presets that make the demo, if not
  `demo`. The game tells it is the demo with `OS.has_feature("demo")`.

### The demo's commands

`check`, `upload`, `builds`, `fetch`, `settings`, `rich-presence` and `promote` take `--demo`.
Each then works on the demo in place of the full game, as it does for the game:

```sh
steamship settings --save --demo   # the demo's snapshot, in steam/demo/settings.vdf
steamship upload --demo            # the pushed commit, built as the demo, uploaded and set live
steamship builds --demo --json     # the demo's branches and last builds
```

Steamworks names a new demo's install folder after the game, followed by a space and Demo. The
first `settings --save --demo` refuses that name. Rename the folder to letters and digits alone,
such as `SpacewarDemo`, under Installation, General Installation. Then save the settings again.

### The first demo upload

Until the demo has a build live on its default branch, it has no other branch. So `check --demo`
says the script's branch is not on Steam yet. The first `upload --demo --default` then sets the
build live on default alone. It names the `promote` command to run once the branch is made in
Steamworks, under SteamPipe, Builds, as [Uploading a build](uploading.md#uploading-a-build)
describes.

### What the demo shares

The demo's rich presence is the game's, sent from the files beside the game's script.
`check --demo` leaves out achievements and leaderboards, since the game's records are the full
game's. `steamship drm-wrap` wraps the demo's executable with the
demo's script, so that it runs under the demo's app.

`steamship init` makes the demo's scripts, with each depot's folder given from the game's folder:

```sh
steamship init 1000070 --depot 1000071=export/demo/windows \
  --depot 1000072=export/demo/linux --output steam/demo
```

### Keeping the demo and the game apart

Both the game's upload and the demo's are refused while the two scripts name the same app or
share a depot. So neither ever goes up into the other. A demo script steamship cannot read stops
the demo's commands alone. One kept at an `AppID` of 0 until the demo's app exists is such a
script.

The demo's check and upload are refused too while its snapshot shows an app not typed Demo. They
are refused while it shows another base game than the one the game's script names.

For the demo and the game alike, these are refused:

- a depot the script uploads that is not among the app's own
- a system the app's depots install on that no launch option runs on
- a build without the program a launch option starts

### The demo's controller configuration

The demo's check and upload are refused while it lacks a Default Controller Configuration or
Default Touch Configuration the game's snapshot shows. They are refused too while it has another.
A new app has none. Without them, a Steam Deck gives the demo no input from its
controls. Choose the game's on the demo's Steam Input page,
`https://partner.steamgames.com/apps/controller/<app>`, and save the demo's settings again.

A demo that bundles a Custom Configuration of its own has its defaults whatever the game's are.
It goes up only with its action manifest at the path its snapshot names.

### Godot export presets

When the game has Godot's `export_presets.cfg`, each preset's `export_path` is matched against
the depots' mappings as steamcmd matches files. That holds whether or not the export is there yet.
The full game's depots are refused while they ship an export made with the demo's feature. The
demo's are refused while they ship one made without it. So a game whose depot maps all of
`export/` has to keep the demo's exports out of it, such as by mapping `windows/*` alone.

## Text in every language

`steamship text` writes what Steam shows of the game in every language its catalogues are in. It
writes from English kept by hand beside the build scripts. `--unchanged` writes nothing. It exits
2 naming each file that differs, is missing or is left over, for a gate to run.

| English | Written beside the scripts | Where it goes |
| --- | --- | --- |
| `store_english.json` | `store/<language>.json` | the store page's Localization tab, every file in one upload |
| its recommended system requirements | `basic_info.json` | the store page's Basic Info form, one language at a time |
| its `screenshots` | `screenshots.json` | the screenshots' alt text panel on the Store Page tab, one language at a time |
| its `disclosure` | `ai_disclosure.json` | the content survey's Generative AI section, one box a language |
| `workshop_english.json` | `workshop/<language>.json` | the Workshop's Steamworks page, one language to a save |
| `presence_english.json` | `rich_presence_<language>.vdf` | `steamship rich-presence` |

### The store page

`store_english.json` holds:

- the store page's `itemid`, named in its Localization tab's export. It is not the app's ID.
- the `disclosure`
- each screenshot's alt text, in the store's order of them
- the store's own list of its images, as `extras`
- the page's text, as `blocks`, each with its `kind`

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

A field or block of another name is refused, and the error names it. An image is named as the
Description tab's custom images name it, the same in every language. Items in a row make one
list. A `field` is any other text Steam shows in the player's language, such as a system
requirement, by its name on the form.

Until the `itemid` is set no store upload is written, since one naming no item cannot go up.

### Image alt text

Steam numbers an image's alt text by the image's place in the store's own list of its images. It
counts from 0, as `app[assets][extra][0][alt_text]`. That list need not follow the About's order.
So with any `alt` given, `extras` names them all in the store's order.

The store page's edit page holds the list in the `data-rggamepageconfig` of its
`application_config`, as `rgExtraAssetsData.rgExtraAssets`. Each entry's `extra_asset_name` is
the image's name after `{STEAM_APP_IMAGE}/extras/`. The list holds every image uploaded, the
About's own and any it does not show. `extras` lists them all. The Store Page tab's extra assets
list is in the About's order instead, so it is no guide.

Every image the About shows has to be in `extras`. Each image `extras` lists gets its `alt`, or an
empty one. An empty one clears what the store held, such as for a decorative image.

### Screenshot alt text

The upload carries no screenshot's alt text. So the screenshots' alt text goes into
`screenshots.json`, as `german 1` for the first one in German, to be typed into their panel.

That panel keeps an `&` as `&amp;` and a straight double quote as `&quot;`, which screen readers
then read out. So a screenshot's alt text holding either, in any language, is refused. Write the
language's word for "and", and its own quotation marks.

### System requirements

A system requirement's field is named for its platform, its tier and its part, as
`app[content][sysreqs][windows][min][osversion]`. The Localization tab takes the minimum ones,
`[min]`. It leaves out every recommended one, `[rec]`, without saying so. Its export leaves them
out too.

So the recommended ones go into `basic_info.json` instead, each under its name on the Basic Info
form. That name ends in the language, as `app[content][sysreqs][windows][rec][osversion][german]`.
An upload on the Localization tab leaves the recommended ones typed there as they are. The form
can be filled before or after it.

### Checking the store page

Once the store page is public, Coming Soon included, `steamship text --steam` reads it in each
language. It exits 2 naming every requirement, minimum or recommended, that the page does not
show as the files give it. Before then Steam shows them only on the Basic Info form.

The store page rewords memory, storage, the DirectX version and VR support, so those are named as
not compared. The operating system, processor, graphics, sound card and notes are compared word
for word.

### The Workshop page and rich presence

`workshop_english.json` holds the Workshop page's `title` and `description`.

`presence_english.json` holds each rich presence token and its English line. A token name ending
in `_` with `{"records": "bishop"}` makes one token for each game record of type `bishop` that
has a name. For `base:alder` the token is `#Bishop_alder` and its line is the record's name.

Without `presence_english.json`, a `rich_presence_english.vdf` kept by hand is the English. It is
left as it is.

### Translations

Each catalogue translates the English under the context `store page`, `steam workshop` or
`rich presence`, with the English line as the `msgid`. A rich presence line's fields are written
`{field}` there, not Steam's `%field%`. A record's name is translated as games-kit's are, under
`bishop.name` and the record's ID.

Anything a catalogue does not translate, or marks fuzzy, is refused and nothing is written. So is
each of these, in English or any language:

- a short description past the 300 characters the store page shows
- a Workshop description past the 200 characters it takes
- a rich presence line holding a slot Steam cannot fill, as `rich-presence` refuses one

### Files no longer written

A file that would no longer be written is removed, but only of a kind whose English the game
gives. Without `store_english.json`, for instance, the store's files and `ai_disclosure.json` are
left to whatever else writes them.

### A gettext template

`steamship text --template` writes nothing and prints those English lines as a gettext template,
for the game's own template to take in. Each is under its context, with a note for its translator.
Where Steam's room is short it carries `#, max-length:300` or `#, max-length:200`, which
translation tools show.

It checks the English as `text` does first, so a short description already past 300 characters is
refused there. A record's name that rich presence shows is left out, since the records give it to
the template already.

### Locale names

`steamship languages` lists each locale a catalogue may be named by, with Steam's name for its
language. `--json` prints the same as one object, `{"pt-BR": "brazilian", ...}`, for a tool that
keeps its own copy to check it against.
