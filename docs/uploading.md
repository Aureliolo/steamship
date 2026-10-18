# Uploading

steamship uploads with your own `app_build` and `depot_build` scripts, the same ones the
Steamworks SDK's ContentBuilder uses; if you already upload with steamcmd, you already have them.
Log in once, then check and upload as often as you like.

## The build account

Valve recommends a Steam account of its own for uploads, with only **Edit App Metadata** and
**Publish App Changes To Steam**, in a
[permission group](https://partner.steamgames.com/pub/groups/) holding only the apps it uploads.
Its login token can do everything those permissions allow, so treat steamship's home, where the
token lives, like a password.

## Logging in

```sh
steamship login
```

Without `--account`, steamship uses the account you logged in with last, or asks for its name.
It then asks for the password, and for a Steam Guard code or approval in the Steam Mobile app.
What you type is passed directly to steamcmd, never logged or saved. steamcmd keeps a token, and
later runs use it without a password. When it expires, an upload stops with exit code 3 and tells
you to log in again.

`steamship status` shows the home, the build account and steamcmd, then logs in with the saved
token as an upload would and says whether Steam takes it. It asks for nothing, prints neither the
account's name nor any key, and exits 0 when an upload would log in and 3 when it would not.
`steamship logout` forgets the token, the account and the Web API key.

## Starting scripts

If you have no build scripts yet, `init` writes them, in Valve's format and commented:

```sh
steamship init 480 --depot 481=build/windows --depot 482=build/linux
```

It writes `steam/app_build.vdf` and a `depot_build_<ID>.vdf` for each depot, each shipping every
file in its folder except debug symbols. Without `--depot` the one depot is the app's ID plus
one, shipping `build/`; `--folder` puts the scripts somewhere other than `steam/`. It never
overwrites a script that is already there.

## Checking scripts

```sh
steamship check steam/app_build.vdf
```

`check` reads the app script and every depot script it names, without logging in, and refuses:

- `SetLive` naming `default` or `public`, which steamcmd never sets live (see
  [the default branch](#the-default-branch)), or a
  branch the app does not have, when a Web API key is set or kept (with neither, it says the
  branch was not checked);
- `Preview` or `Local` set in the file (use `--preview` instead);
- a `ContentRoot` or `LocalPath` that does not exist or matches no files;
- an `InstallScript` that is not a file in the content, or that the depot does not map;
- a `steam_appid.txt` in the content, and the folders Unity names as not to ship;
- a Linux or macOS program, or a script starting with `#!`, without its executable bit, where
  the file system has one.

It also counts the debug symbols each depot ships (`.pdb`, `.dSYM`, `.debug`) without refusing
them, since some teams ship them on purpose.

Before the build exists, as in a checkout where only CI builds the game, `--scripts-only` checks
the scripts alone: everything above except what needs the content, which it says was not
checked.

```sh
steamship check steam/app_build.vdf --scripts-only
```

Keys steamship does not know are passed to steamcmd unchanged. `upload` runs the same check
first, so a script it would refuse is never sent.

## Uploading a build

```sh
# a rehearsal: builds everything and sends nothing to Steam
steamship upload steam/app_build.vdf --version 1.4.0 --preview
# the real thing
steamship upload steam/app_build.vdf --version 1.4.0
```

The build's description is the version and the Git commit the app script's own repository is
at, whatever folder steamship runs in: a script in a worktree names that worktree's commit. Build
output (logs, manifests, the chunk cache) goes to steamship's own folder for the app, never into
your content, and is kept between runs so later uploads are faster. A failure or an expired token
ends the run rather than waiting at a prompt. It prints the BuildID and, when the script names
one, the branch it was set live on; in a GitHub Actions step it also gives the BuildID as the
step's `build-id` output.

With a Web API key at hand, a build set live on a branch is then confirmed with Steam: if Steam
goes on showing another build there, the run fails with the `promote` command, so a script
wrapping steamship can rely on its exit code. Steam that cannot be asked holds nothing up.

When Valve builds the upload but steamcmd fails after, as when setting it live, the run still
fails, and says the BuildID and the `steamship promote` command that sets it live, so nothing
has to be uploaded again. The `build-id` output is given then too.

When Steam keeps the build but will not set it live on the `SetLive` branch, as for a branch the
app does not have, steamcmd reports only "Failed to commit build" and no BuildID. steamship says
the build is on Steam but not live, and why that happens. With a Web API key at hand it finds the
build by its description and gives its BuildID and the `promote` command, as above.

steamcmd builds each depot against the manifest of the branch's last build, which Steam's CDN
serves only some minutes after that build is made. An upload soon after the last one can fail
on it, with `Failed to download manifest` and `(HTTP 404)`. steamship then asks the CDN for that
manifest every 30 seconds and runs steamcmd again once it is served, for up to half an hour and
four runs in all, so a second upload straight after the first one waits rather than fails.
When steamcmd reaches Steam but not its build server, `Failed to initialize build on server (No
Connection)`, or Steam drops it once it has connected, `disconnected from steam with result 3 (No
Connection)`, steamship runs it once more.

`--preview` is Valve's dry run: the whole build is computed and logged, nothing is uploaded and
nothing is set live.

For a script that runs steamship, `--json` prints what came of the upload on standard output as
one line of JSON, here spread out, once steamcmd has run, and sends everything meant for a
person to standard error:

```json
{
  "app": 1000,
  "branch": "testing",
  "build_id": 4242,
  "default": false,
  "description": "1.4.0 4f3c2a1e9b07",
  "live": true,
  "outcome": "uploaded"
}
```

`outcome` is `uploaded`, `previewed`, `not-live` (built, but Steam shows another build on the
branch, or did not set it live on default when asked), `built-then-failed`, `failed` or
`not-logged-in`; `build_id` is the build Steam made, when it made one, and `branch` the one the
script sets live, when it names one. `live` says whether the build is live on that branch, and
`default` whether on [the default branch](#the-default-branch). The exit code is as without it,
and a run refused before steamcmd starts prints no JSON.

## Workshop items

```sh
steamship workshop workshop/item.vdf
```

`workshop` uploads an item from Valve's `workshopitem` script, with the saved login. It refuses,
before anything is sent, an `appid` or `publishedfileid` that is not a number, a `contentfolder`
that is missing or empty, a `previewfile` that is missing or larger than the 1 MB Steam takes, a
`visibility` other than 0 (public), 1 (friends only), 2 (private) or 3 (unlisted), and a title,
description or change note longer than Steam takes. Paths are relative to the script.

A script with no `publishedfileid`, or `0`, makes a new item: steamship prints its ID and the
line to add to the script, so that later uploads update the same item. Your script is never
rewritten. In CI nobody adds that line, so every run would make another item: there, a script
with no `publishedfileid` is refused unless `--new` is given.

A game that would rather not write Valve's script gives the item's fields, and steamship
writes the script itself:

```sh
steamship workshop --app 1000070 --items steam/workshop_items.json --name example \
  --content mods/example --preview art/example.png --title "Example mod" \
  --description "What the mod shows." --change-note "1.0.0" --visibility private
```

`--app`, `--items`, `--name`, `--content` and `--title` are needed; `--preview`,
`--description` and `--change-note` may be left out. They are checked as a script's are.
`--visibility` is `public`, `friends-only`, `private` or `unlisted`. It is needed to make an
item, since steamship never chooses who sees one; left out, an item already made keeps what it
has on Steam.

`--items` names a file kept with the game that holds the ID of each item made, by the name
`--name` gives it, of letters, digits, `-`, `_` and `.`, not first. When the file has no item
of that name, a new one is made, and steamship
writes its ID into the file, making the file if missing. When the file has one, that item is
updated. Commit the file, so that a run from any machine, CI among them, updates the items
made before:

```json
{
  "app": 1000070,
  "items": {
    "example": 3000000001
  }
}
```

A new item is refused in CI unless `--new` is given, as for a script.

Each item has a folder of its own in steamship's home, named by `--name`, or for a script by
its `publishedfileid`, or `new` when it has none. The script steamcmd is given and steamcmd's
console go there, so uploading several items in a row leaves each one's console to read.

With `--json`, either form prints what came of the upload on standard output as one line of
JSON, here spread out, once steamcmd has run, and everything meant for a person on standard
error:

```json
{
  "app": 1000070,
  "item": 3000000001,
  "log": "/home/you/.local/share/steamship/workshop/1000070/example/steamcmd.log",
  "name": "example",
  "outcome": "made"
}
```

`outcome` is `made`, `updated`, `failed` or `not-logged-in`, `item` is the item's ID when there
is one, `log` is where steamcmd's console was saved, and `name` is the item's name in the
`--items` file, or `null` for a script. The exit code is as without it, and a run refused
before steamcmd starts prints no JSON.

## Branches and promoting

```sh
steamship builds 480
steamship promote 480 --build 12345678 --branch testing
steamship branch 480 testing --description "0.3.0, the season update"
```

`builds` lists each branch with the build live on it, and the last builds uploaded, with the day
each was uploaded and where it is live. `promote` sets an uploaded build live on a beta branch
without uploading it again, and on the default branch while the app is unreleased, as below.
`branch` sets the description players see beside a beta branch when they choose it in the
game's Betas properties, such as the version live on it; nothing is sent when it already reads
so, and the default branch's description is set in Steamworks only. An app is named by its ID
or by its app build script.

A new app has only its default branch, and Steamworks lets you create others only once a build
is live on it. So the first upload of a script that sets a beta branch live leaves the build on
Steam, live nowhere. Set that build live on the default branch with `promote`, as below, create
the branch the script names in Steamworks under SteamPipe, Builds, then upload again or
`promote` the build.

### The default branch

```sh
steamship upload steam/app_build.vdf --version 1.4.0 --default
steamship promote 480 --build 12345678 --branch default
```

Until a game is released, nobody plays its default branch but its developers, and Valve sets a
build live there for the publisher key alone. `upload --default` sets the build live on the
default branch too, after the branch the script sets live, and `promote --branch default` sets
an uploaded one live there. steamcmd's `SetLive` cannot name the default branch, so both go
through the Web API, with the key as for `promote`.

Before anything is sent, steamship asks steamcmd, logged in as the build account, for the app's
release state, and goes on only while Steam shows it `unavailable` (no store page yet) or
`prerelease` (coming soon). Once the game is released, its default branch is the one players
get, and Valve asks for a Steam Mobile confirmation, so it is set live in Steamworks. Then
`upload --default` says so and uploads to the script's branch alone, so a game's tools can pass
`--default` on every upload from before launch to after it; `promote --branch default` is
refused. steamship never sends the SteamID that confirmation needs, so Valve refuses too.

With `--json`, `default` says whether the build is live on the default branch, and `outcome`
is `not-live` when the app is unreleased and Steam did not set it there.

For a script that runs steamship, `builds --json` prints the branches and builds on standard
output as one line of JSON, here spread out, and sends everything meant for a person to
standard error:

```json
{
  "app": 480,
  "branches": [
    {"name": "testing", "build_id": 12345678, "description": "0.3.0", "password": true}
  ],
  "builds": [
    {
      "build_id": 12345678,
      "created": "2026-10-13T17:01:31Z",
      "description": "0.3.0 a1b2c3d4e5f6",
      "version": "0.3.0",
      "commit": "a1b2c3d4e5f6",
      "live_on": ["testing"]
    }
  ]
}
```

`password` says whether players need the branch's password. A build's `version` and `commit`
are read back from the description `upload` gave it, and are null for a build described
otherwise; Steam does not say when a build was set live. When Steam does not answer, the line
is an `error` with a `code` and a `message`, and the exit code is 1:

```json
{"app": 480, "error": {"code": "no-build-live", "message": "Steam answered HTTP 500; ..."}}
```

`no-build-live` is Steam's answer while no build is live on the app's default branch. The
other codes are `key-refused`, `http-error`, `unreachable`, `unreadable` and `steam-failed`. A
run refused before Steam is asked, such as one with no key, prints no JSON and exits 2; with
`--json`, steamship never asks for a key.

All three use Steam's partner Web API, which steamcmd cannot reach, with the publisher key of
a group that holds the app, from Steamworks under Users & Permissions, Manage Groups. The key
comes from `STEAMSHIP_WEB_API_KEY`, or else from the one steamship keeps. At a terminal with
neither, it is asked for, and once the command works, offered to be kept. `steamship login`
offers to keep one too, and `steamship login --web-api-key` does only that, after Steam has
checked it.

A kept key is in the system's credential store: Windows Credential Manager, the macOS Keychain,
or GNOME Keyring or KWallet on Linux. It is sent in a request header, never in an address. The
key can do everything its group may, for every app the group holds, so give it a group of its
own with only the apps and permissions it needs.

## Settings in Steamworks

```sh
steamship settings 480
steamship settings 480 --save steam/settings.vdf
steamship settings 480 --check steam/settings.vdf
```

A build runs only as Steamworks is set up for it: the install folder, each launch option's
executable and system, the Linux runtime, each depot's system, and where Steam Cloud keeps
saves. None of that is in the build scripts, and none of it can be changed but by hand in
Steamworks. `settings` asks Steam for it through steamcmd with the saved login, as the build
account sees the app (which works before the app is released), and shows it in Valve's own
format. It also says whether Steam Cloud is set up, since an app with none has no Cloud section
at all, and a snapshot without one would otherwise read the same either way.

`--save` keeps it in a file beside your scripts, and `--check` exits 2 naming every setting
that differs from that file, such as `config/launch/0/executable`. Run the check before a
release, so that a launch option changed in Steamworks, by mistake or by someone else, stops
it; when the change was meant, save the file again and commit it. Steam's own bookkeeping, the
manifests of each depot and the build live on each branch, and the store details of a released
app are left out, so the file changes only when the app's settings do. Nothing on Steam is
changed.

## Store and library artwork

```sh
steamship assets steam/store
```

Steamworks takes its artwork by hand, one upload at a time, and says only then that a picture
is the wrong size. `assets` checks a folder of it first, without sending anything: every PNG,
JPEG and icon in it, each named for the asset it is, against Valve's sizes and formats. It exits
2 naming each one that is wrong, each named for no asset, and too few screenshots.

| File | Size | Format |
| --- | --- | --- |
| `header_capsule` | 920x430 | PNG or JPEG |
| `small_capsule` | 462x174 | PNG or JPEG |
| `main_capsule` | 1232x706 | PNG or JPEG |
| `vertical_capsule` | 748x896 | PNG or JPEG |
| `screenshot…`, at least five | 1920x1080 or larger, 16:9 | PNG or JPEG |
| `page_background` | 1438x810 | PNG or JPEG |
| `bundle_header` | 707x232 | PNG or JPEG |
| `library_capsule` | 600x900 | PNG |
| `library_header` | 920x430 | PNG |
| `library_hero` | 3840x1240 | PNG |
| `library_logo` | 1280 wide or 720 tall, no larger | PNG with transparency |
| `shortcut_icon` | 256x256 | icon or PNG |
| `app_icon` | 184x184 | JPEG |
| `event_cover` | 800x450 | PNG or JPEG |
| `event_header` | 1920x622 | PNG or JPEG |

Names match regardless of case, and screenshots are any names starting with `screenshot`,
such as `screenshot_01.jpg`. Other files in the folder are left alone.

## Steam DRM

```sh
steamship drm-wrap 480 build/windows/game.exe
steamship drm-wrap 480 build/windows/game.exe --output wrapped/game.exe --compatibility
```

`drm-wrap` has Valve's servers add the app's Steam DRM to a Windows executable, through steamcmd
with the saved login, before the build is uploaded. The executable is replaced by the wrapped
one, or that is written to `--output`; either way nothing is replaced unless the wrap
succeeded, and the wrapped file keeps the original's permissions. `--compatibility` asks for
Valve's compatibility mode, for an executable the default wrap breaks.

Valve's tool adds Steam DRM only to an executable that already imports `GetModuleHandleA` from
`kernel32.dll`, as it says when it refuses one. Executables made by Godot and by Rust do not
import it, so they cannot be wrapped; steamship passes the refusal on with that explanation.

## Achievements and stats

```sh
steamship achievements 480
steamship achievements 480 --check steam/achievements.json
```

Steamworks is where achievements and stats are made and published, and steamship changes
nothing there. `achievements` lists those Steam holds for the app, in English, with the same
Web API key as `builds`, each achievement with the share of players who have it and each stat
with its total over all players. Steam gives those once players have the game, and a total only
for a stat marked as aggregated in Steamworks. With `--check`, it compares them with a file kept
beside your scripts, and exits 2 naming each achievement that is in the file but not on Steam or
the other way round, each display name, description or hidden flag that differs, and each icon
Steam has none of; with an import receipt, also each icon changed since that import, as
[Which icons Steam has](#which-icons-steam-has) describes. It ends with how many of the file's
achievements match, and of its stats when it lists any, such as
`14 of 15 achievements match: 1 differs`. Run it before a release, and the
release waits while the two have drifted apart.

`achievements --json` prints what Steam holds on standard output as one line of JSON, with the
achievements and stats in the keys the file below uses, each achievement's `icon` and
`icon_locked` added as addresses, or null where Steam has none. Each achievement's `percent` is
the share of players who have it, and each stat's `total` its total over all players, either
null where Steam gives none. Steam failing is an `error` as `builds --json` gives it. It does not
go with `--check`.

The file lists each achievement by the API name the game unlocks it with, and each stat by the
one the game sets it by:

```json
{
  "app": 480,
  "achievements": [
    {
      "api_name": "ACH_WIN_ONE_GAME",
      "name": "Winner",
      "description": "Win one game.",
      "hidden": false
    }
  ],
  "stats": [
    {
      "api_name": "NumGames",
      "name": "Games played",
      "default": 0
    }
  ]
}
```

`description` and `hidden` may be left out, for none and `false`. The stats are checked only
when the file has a `stats` list: each stat missing on either side, and each display name or
default that differs. A stat's `name` and `default` may be left out, for none and 0. A stat may
also say whether it is `aggregated`, true or false, which is checked against the totals Steam
keeps of it over all players; where Steam answers neither a total nor that it keeps none, the
check says so and leaves it. Its minimum, maximum, maximum change, whether it only goes up and
who may set it are not in Steam's Web API, so only Steamworks shows them. `app`, when there,
must be the app checked, and other keys, such as your own tools', are left alone. Achievements
and stats count on Steam only once published in Steamworks, so one entered or imported but not
yet published is missing from Steam's, or shows as it was before; a check that fails says so.

A game made with games-kit can be checked against its own achievement records instead, a list
in which those typed `achievement` and `stat` are read as above, by the API name in `steam`:

```json
[
  { "type": "stat", "id": "base:floors", "steam": "floors", "kind": "int" },
  {
    "type": "achievement",
    "id": "base:long_journey",
    "steam": "LONG_JOURNEY",
    "name": "Rising Water",
    "description": "Bring the millpond up to its brim.",
    "stat": "base:floors",
    "at": 2
  }
]
```

Records of other types are left alone. The records are all of the game's stats, so they are
always checked: a stat's `name` only when its record sets it, and its `default` and `aggregated`
as games-kit gives them, 0 and `false` where the record says nothing.

`--check` takes several files, after it or each after its own `--check`, and reads them
together, so achievements and stats kept in more than one file are checked as one list. An
achievement or stat in two of them is refused, naming both, and so is an achievement and a stat
of one API name, in one file or two: Steamworks saves such an import but refuses to publish it,
saying the achievement "has API name conflict with stat". Steam's Web API does not give a
stat's type, bounds, whether it only goes up or who may set it, nor an achievement's progress
stat and maximum, its set, whether it is archived or who may unlock it. So a stat's `kind`,
`min`, `max`, `max_change`, `increment_only` and `permission`, and an achievement's `stat`, `at`,
`set`, `archived` and `permission`, go unchecked, and the check says so. They are still checked
against each other, as the next section describes.

### Every language

```sh
steamship achievements 480 --check steam/achievements.json steam/achievements/localisation.csv
```

A `.csv` file among them is a localisation in the columns Steamworks' achievement editor
exports under Bulk Import/Export: the API name, then `name` or `description`, then a column for
each language headed with Steam's name for it, such as `german` or `schinese`. The editor's
heads for the first two columns say more, but begin with `api_name` and `field`, which is all
steamship needs of them.

```text
api_name,field,english,german
FIRST_ANCHOR,name,Down by the Wheel,Hinab zum Rad
FIRST_ANCHOR,description,Drain a mill's first sluice.,Gib die erste Schleuse eines Mahlwerks frei.
```

`--check` then asks Steam for the achievements in each of its languages too, and names each
name or description that differs, with its language. Steam answers a text it has no
translation of in English, so one the localisation has and Steam lacks is said to be missing,
and a language Steam holds nothing in is said once. The editor's import can apply only part of
a localisation the first time, which this catches. Every achievement the other files list must
be in the localisation, and nothing else may be.

A game that keeps its translations in gettext catalogues gives those instead, each `.po` file in
the language its header's `Language` names, or else its file name, such as `de.po` or
`pt_BR.po`, which steamship turns into Steam's name for it with Valve's table. An achievement's
name and description are under the contexts `achievement.name` and `achievement.description`,
and a set's name under `achievement_set.name`, each by the record's `id`, as games-kit keeps a
record's translations; a file of the kind above, with no `id`, uses the API name or set ID. A
text a catalogue lacks, leaves empty or marks fuzzy is refused, naming the catalogue, the
context and the id.

```sh
steamship achievements 480 --check mods/base/data/achievements.json game/locale/*.po
```

### The achievement editor's import

```sh
steamship achievements 480 --write mods/base/data/achievements.json game/locale/*.po --out steam/achievements
steamship achievements 480 --write mods/base/data/achievements.json game/locale/*.po --out steam/achievements --unchanged
```

Steamworks' achievement editor takes a game's achievements as files dropped on it under Bulk
Import/Export, and no Web API sets them. `--write` writes those files into the `--out` folder,
from the same files `--check` reads, without asking Steam anything:

- `480-achievements-definitions.csv`: each achievement's set, who may unlock it, whether it is
  a spoiler (hidden) or archived, and the stat its progress bar counts, from 0 up to its `at`;
- `480-achievements-localization.csv`: each name and description in English and in each
  language of the catalogues or localisation given;
- `480-achievement-sets-localization.csv`, when there are sets: each set's name in the same
  languages;
- `<API name>_ACHIEVED.png` and `<API name>_UNACHIEVED.png`, each achievement's `icon` and
  `icon_locked`, which the editor puts on the achievement they are named for.

Besides what the check reads, an achievement may give its `set`, `archived` as true for one that
can no longer be earned, which Valve asks for in place of deleting one players hold, and its
`permission`: `Client`, the default, `GameServer` or `OfficialGameServer`, which a games-kit
record words as `client`, `game_server` or `official_game_server`. Its icons are paths
inside the games-kit mod its records are in, the folder holding the mod's `manifest.json`, or
beside the file of the kind above; each has to be a square PNG of 64 pixels or more, 256 at
best. A set is listed with its set ID and English name, as `sets` in that file:

```json
{ "setid": "1", "name": "The Cellar" }
```

or as a games-kit record, which an achievement's `set` names by its `id`:

```json
{ "type": "achievement_set", "id": "base:cellar", "steam": 1, "name": "The Cellar" }
```

A set is made in the editor, which gives it its ID, before an import can put achievements in it.
The ID may be given as text or as a whole number.
Before anything is written, and before `--check` asks Steam anything, the files are refused,
exiting 2, where the editor would refuse them: an achievement that counts a `stat` with no
`at`, or unlocks `at` a value with no stat, at 0 or below, or at a fraction of a stat whose
`kind` is `int`; one that counts a stat or is in a set no file lists; a set with no English
name; and more than 40 sets with the base set, or more than 5,000 achievements, archived ones
included. Over 100, steamship says that Steam takes them only once Valve has vetted the app.

A file of the kinds written for the app that `--write` would no longer write, such as the icon
of an achievement since removed, is removed from the folder; other files are left alone. With
`--unchanged` nothing is written, and it exits 2 naming each file that is missing, differs or
is left over, which keeps the folder in step with the records in a game's own checks.

Once written, drop every CSV and PNG file on the editor and Save imported data, then drop the
`achievements-localization.csv` again on its own: the editor applies only part of a
localisation in the import that makes the achievements it names. Each time, stay on the page
until Pending Changes is gone; a page left sooner has applied only some of the icons. Once
published, `--check` reads them back from Steam in every language.

An import leaves the progress bar of an achievement Steam already has as it was, and Steam's
Web API does not give the bar, so `--check` cannot see it. When `--write` changes a bar the
folder's definitions gave before, it names the achievement with both ranges, to set by hand in
the editor before publishing.

### Which icons Steam has

```sh
steamship achievements 480 --write mods/base/data/achievements.json game/locale/*.po --out steam/achievements --imported
steamship achievements 480 --check mods/base/data/achievements.json --out steam/achievements
```

Once the import is saved and published, `--imported` fetches each icon Steam serves, for an
unreleased app too, and compares it with its file pixel by pixel, the file scaled to the size
Steam serves. Steam serves a JPEG, which leaves every pixel of an icon it took within 48 of the
file in each channel, out of 255; an icon is not the file's when more than one pixel in a
thousand is more than 64 off, so a changed detail ten pixels square is caught. Pixels the file
has see-through are left out, since what Steam puts behind them is not known. When every icon
is alike, `--imported` writes `steam-icons.json` into the `--out` folder, recording each icon's
file by its SHA-256 and the image Steam made of it by the name it serves it under, and writes
nothing else.

It is refused, exiting 2, when the folder is not as `--write` would write it, when Steam has an
icon's achievement or the icon itself not yet, when an icon's file changed since the last
receipt but Steam still has the image the last import made, when Steam has an icon for an
achievement the files give none, as one uploaded in the editor by hand, and when an icon Steam
serves is not its file, saying how many pixels are off and by how much on average. A locked
icon the files lack is left to Steam, which makes one when the import is told to. Steam answers
with what is published, the Web API and the images alike, so an import saved in the editor but
not yet published still shows the icons before it.

`--check` with that folder as `--out` then reads the receipt as well, and names each icon whose
file changed since the import it records, each whose image on Steam changed though its file did
not, and each it has no import of, without fetching the images; a folder with no receipt is
refused. `--write` and `--unchanged` leave `steam-icons.json` alone, so a game's own checks can
run `--unchanged` without Steam. Keep the receipt with the records.

## Leaderboards

```sh
steamship leaderboards 480
steamship leaderboards 480 --check steam/leaderboards.json
steamship leaderboards 480 --check steam/leaderboards.json --create
```

`leaderboards` lists those Steam holds for the app, with the same Web API key as `builds`.
With `--check`, it compares them with a file by name, and exits 2 naming each leaderboard that
is in the file but not on Steam or the other way round, and each setting that differs. With
`--create` too, the file's leaderboards that Steam lacks are made first, as the file has them,
so a release can bring in a new one before the build that uses it.

Steam keeps an existing leaderboard's settings and its scores: asked to make one that is already
there, it answers with the one it has. So steamship never changes, remakes or deletes a
leaderboard. A setting that differs is changed in Steamworks, under Stats & Achievements,
Leaderboards, and one on Steam but not in the file is left for you to delete there or add to
the file.

```json
{
  "app": 480,
  "leaderboards": [
    {
      "name": "fastest_season",
      "sort": "ascending",
      "display": "seconds",
      "trusted_writes": false,
      "friends_only": false
    }
  ]
}
```

`sort` is `ascending` (the lowest score first) or `descending`, and `display` is `numeric`,
`seconds` or `milliseconds`; both are needed, so that no leaderboard is made with one guessed.
`trusted_writes` (only the Web API may set scores, never the game) and `friends_only` (players
see only their friends' scores) may be left out, for `false`. `app`, when there, must be the app
checked, and other keys are left alone.

A game built on games-kit keeps its leaderboards as records, beside its achievements and stats,
and `--check` reads them as it reads those: a list of records, of which each typed `leaderboard`
is read the same way by its Steamworks name in `steam`. Its `upload` and `details` are what the
game sends with each score, not settings Steam keeps, so they are left alone. `--check` takes
several files, after it or each after its own `--check`, so the files given to `achievements
--check` can be given here too, and a leaderboard in two of them is refused, naming both.

```sh
steamship leaderboards 1000050 --check mods/base/data/achievements.json mods/base/data/boards.json
```

## Rich presence

```sh
steamship rich-presence 480 steam/rich_presence_english.vdf steam/rich_presence_german.vdf
steamship rich-presence 480 steam/rich_presence_*.vdf --preview
```

`rich-presence` uploads the localisation files Steamworks takes under Community, Rich Presence,
in Valve's own format, one per language, with the same Web API key as `builds`:

```vdf
"lang"
{
    "Language"  "english"
    "Tokens"
    {
        "#menu"             "At the menu"
        "#running"          "Grinding at %mill% in {#season_%season%}"
        "#season_rainy"    "Autumn"
    }
}
```

Each file replaces all of its language's tokens on Steam, so the files are the whole truth: a
token taken out of a file is gone from Steam after the next upload, and languages with no file
are left as they are. Every file is checked before anything is sent, and one that names no
language, holds no tokens, has a token that does not start with `#` or is there twice, or a
text that refers to tokens (`{#season_%season%}`) that none starts as, is refused with exit 2;
so are two files for the same language. `--preview` checks the files and sends nothing.
