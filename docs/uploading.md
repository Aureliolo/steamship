# Uploading

steamship uploads with your own `app_build` and `depot_build` scripts, the same ones the
Steamworks SDK's ContentBuilder uses. If you already upload with steamcmd, you already have them.
Log in once, then check and upload as often as you like.

## The build account

Valve recommends a Steam account of its own for uploads. It should have only **Edit App
Metadata** and **Publish App Changes To Steam**, in a
[permission group](https://partner.steamgames.com/pub/groups/) holding only the apps it uploads.
Its login token can do everything those permissions allow. Treat
[steamship's home](install.md#the-home-folder), where the token lives, like a password.

## Logging in

```sh
steamship login
```

Without `--account`, steamship uses the account you logged in with last, or asks for its name.
It then asks for the password and for a Steam Guard code or approval in the Steam Mobile app.
What you type is passed directly to steamcmd, never logged or saved.

steamcmd keeps a token. Later runs use it without a password. When it expires, an upload stops
with exit code 3 and tells you to log in again.

`steamship status` shows the home, the build account and steamcmd. It then logs in with the
saved token as an upload would and says whether Steam takes it. It asks for nothing and prints
neither the account's name nor any key. It exits 0 when an upload would log in and 3 when it
would not. `steamship logout` forgets the token, the account and the Web API key.

## Starting scripts

If you have no build scripts yet, `init` writes them, commented, in Valve's format:

```sh
steamship init 480 --depot 481=build/windows --depot 482=build/linux
```

It writes `steam/app_build.vdf` and a `depot_build_<ID>.vdf` for each depot. Each ships every
file in its folder except debug symbols. Without `--depot`, the one depot is the app's ID plus one
and ships `build/`. `--folder` puts the scripts somewhere other than `steam/`. When any of the
scripts is already there, it writes none.

In a [game's folder](games.md), the game's own app script sets the game's branch live. That is
`testing` unless `steamship.toml` names another. The demo's app script sets the demo's branch live.
A script written anywhere else leaves `SetLive` commented out for you to choose.

## Checking scripts

```sh
steamship check steam/app_build.vdf
```

`check` reads the app script and every depot script it names, without logging in. It refuses a
script with any of these:

- `SetLive` naming `default` or `public`, which steamcmd never sets live (see
  [the default branch](#the-default-branch)).
- `SetLive` naming a branch the app does not have, when a Web API key is set or kept. With
  neither, `check` says the branch was not checked.
- `Preview` or `Local` set in the file. Use `--preview` instead.
- A `ContentRoot` or `LocalPath` that does not exist or matches no files.
- An `InstallScript` that is not a file in the content, or that the depot does not map.

Keys steamship does not know are passed to steamcmd unchanged. `upload` runs the same check
first. A script it would refuse is never sent.

### What the content may not hold

`check` also refuses a build whose content holds any of these:

- A `steam_appid.txt`, or a folder Unity names as not to ship, such as
  `<game>_BurstDebugInformation_DoNotShip`.
- A Steam Input Action Manifest listing a controller configuration that the build does not ship
  beside it.
- A Linux or macOS program, or a script starting with `#!`, without its executable bit, where the
  file system has one.

### What the settings snapshot asks for

A [settings snapshot](#settings-in-steamworks) beside the scripts may choose a Custom
Configuration as the Default Controller Configuration. The build must then install its action
manifest at the path the snapshot names, on each system the depots install on. `check` refuses a
build that does not, and a snapshot that names no path.

`check` refuses a build that does not install the program a launch option in the snapshot starts,
on a system the option runs on. It names the program and the programs the build installs in its
place. Linux tells names apart by case. Windows and macOS do not. A macOS app's folder is the
program.

A system with content from a depot the build leaves out is not judged. Neither is an option
offered on a beta branch the build is not set live on.

### Windows programs

Windows programs and libraries may import only DLLs that Windows 10 and later, a depot installing
on Windows or a Steamworks Common Redistributable has. `check` refuses one that imports any other
DLL.

`VCRUNTIME140.dll` and `XINPUT1_3.dll` are among the DLLs only a Common Redistributable installs.
A program importing one is refused unless the app ticks it in Steamworks (Installation,
Redistributables). So is a program that delay-loads one.

`check` also refuses a .NET program whose `runtimeconfig.json` names a runtime to be installed
apart from it. Neither Windows nor a Common Redistributable installs such a runtime.

### What check names without refusing

The settings snapshot tells `check` which redistributables the app ticks and which depots install
on Windows. Without a snapshot, `check` names each DLL a redistributable would install and says
it was not checked. `upload` asks Steam which redistributables the app ticks.

A DLL that an `InstallScript` of the build may install is named rather than refused. So is a
delay-loaded DLL that nothing installs, such as a graphics driver's.

`check` counts the debug symbols each depot ships (`.pdb`, `.dSYM`, `.debug`). It does not refuse
them.

### A build's folder

A build's folder can be checked on its own as soon as it is built, before any script maps it. A
build tool does this after exporting each platform.

```sh
steamship check export/windows
```

The files in it, and in every folder in it, are checked as a depot's are. In a game's folder, its
Windows files are judged against the redistributables the game's settings snapshot ticks. The
build is refused without the Steam Input action manifest the snapshot names. It is also refused
when it holds a program but none that a launch option starts. A folder holding no program, such as
a depot of data alone, is not judged by its launch options.

The demo's snapshot is used in place of the game's when only the demo's depots ship the folder, or
with `--demo`. A demo can tick fewer redistributables than its game.

### Scripts alone

Before the build exists, as in a checkout where only CI builds the game, `--scripts-only` checks
the scripts alone. It runs every check above but those that need the content. It says those were
not checked.

```sh
steamship check steam/app_build.vdf --scripts-only
```

## Uploading a build

```sh
# a rehearsal: builds everything and sends nothing to Steam
steamship upload steam/app_build.vdf --version 1.4.0 --preview
# the upload
steamship upload steam/app_build.vdf --version 1.4.0
```

In a game's folder, `steamship upload` alone takes the script and version from the game. It ships
only a pushed commit, built and checked first. [Shipping a game](games.md) has the rest.

The build's description is the version and the Git commit the app script's own repository is at.
That holds whatever folder steamship runs in. A script in a worktree names that worktree's commit.

Build output (logs, manifests, the chunk cache) goes to steamship's own folder for the app, never
into your content. It is kept between runs so that later uploads are faster. A failure or an
expired token ends the run rather than waiting at a prompt.

The run prints the BuildID and, when the script names one, the branch it was set live on. In a
GitHub Actions step, it also gives the BuildID as the step's `build-id` output.

### Setting the build live

With a Web API key at hand, steamship then confirms with Steam that the build is live on its
branch. If Steam goes on showing another build there, the run fails and gives the `promote`
command. A script wrapping steamship can rely on its exit code. When the Web API does not answer,
the run says the build is not confirmed and does not fail.

When Valve builds the upload but steamcmd fails afterwards, as when setting it live, the run still
fails. It gives the BuildID and the `steamship promote` command that sets it live. Nothing has to
be uploaded again. The `build-id` output is given then too.

### When Steam keeps the build but not the branch

Steam may keep the build but not set it live on the `SetLive` branch, as for a branch the app does
not have. steamcmd then reports only "Failed to commit build" and no BuildID. steamship says the
build is on Steam but not live and explains why that happens. With a Web API key at hand it finds
the build by its description and gives its BuildID and the `promote` command, as above.

### An app with no default build

An app with no build live on its default branch, such as a new demo, has
[no other branch yet](#the-first-branch-of-a-new-app). With a Web API key at hand, steamship sees this
before uploading. It then has steamcmd upload the build without setting it live on the `SetLive`
branch.

With `--default`, the build is set live on default. The run gives the `promote` command that sets
it live on the branch once the branch is made in Steamworks under SteamPipe, Builds. Without
`--default`, the build is live nowhere. The run fails and gives the `promote` command for default.

### Retries

steamcmd builds each depot against the manifest of the branch's last build. Steam's CDN serves
that manifest only some minutes after the build is made. An upload soon after the last one can
fail on it, with `Failed to download manifest` and `(HTTP 404)`.

steamship then asks the CDN for that manifest every 30 seconds. It runs steamcmd again once the
manifest is served, for up to half an hour and four runs in all. A second upload straight after
the first one waits rather than fails.

steamship also runs steamcmd once more when it fails in one of these ways:

- steamcmd reaches Steam but not its build server, `Failed to initialize build on server (No
  Connection)`.
- Steam drops it once it has connected, `disconnected from steam with result 3 (No Connection)`.
- Steam sends it no client config as it connects, `Failed to load platform info`.

### Rehearsals and timings

`--preview` is Valve's dry run. The whole build is computed and logged. Nothing is uploaded and
nothing is set live.

`--times`, given to any command, ends each step's line with how long it took, such as
`pinned and verified (0.2 s)`. Times are to the tenth of a second up to two minutes. A log then
shows where an upload's time went.

### Upload results as JSON

For a script that runs steamship, `--json` prints what came of the upload as one line of JSON on
standard output, once steamcmd has run. Everything meant for a person goes to standard error.
Here the line is spread out:

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

- `outcome` is `uploaded`, `previewed`, `not-live`, `built-then-failed`, `failed` or
  `not-logged-in`.
- `not-live` is a build made but not set live as asked. Steam shows another build on the branch,
  the app cannot have the branch yet, or Steam did not set it live on default when asked.
- `build_id` is the build Steam made, when it made one.
- `branch` is the one the script sets live, when it names one.
- `live` says whether the build is live on that branch.
- `default` says whether it is live on [the default branch](#the-default-branch).

The exit code is the same as without `--json`. A run refused before steamcmd starts prints no
JSON.

## Workshop items

```sh
steamship workshop workshop/item.vdf
```

`workshop` uploads an item from Valve's `workshopitem` script, with the saved login. Before
anything is sent, it refuses a script with any of these:

- An `appid` or `publishedfileid` that is not a number.
- A `contentfolder` that is missing or empty.
- A `previewfile` that is missing or larger than the 1 MB Steam takes.
- A `visibility` other than 0 (public), 1 (friends only), 2 (private) or 3 (unlisted).
- A title longer than the 128 bytes Steam takes, or a description or change note longer than
  8,000 bytes.

Paths are relative to the script.

### Making a new item

A script with no `publishedfileid`, or `0`, makes a new item and needs a `previewfile`. Without
one, Steam still makes the item, silently, with its grey default picture. steamship prints the new
item's ID and the line to add to the script. With that line, later uploads update the same item.
Your script is never rewritten.

In CI, nobody adds that line. There a script with no `publishedfileid` is refused unless `--new`
is given, since every run would make another item.

### Items without a script

A game that would rather not write Valve's script gives the item's fields. steamship writes the
script itself:

```sh
steamship workshop --app 1000070 --items steam/workshop_items.json --name example \
  --content mods/example --preview art/example.png --title "Example mod" \
  --description "What the mod shows." --change-note "1.0.0" --visibility private
```

`--app`, `--items`, `--name`, `--content` and `--title` are needed. `--description` and
`--change-note` may be left out. They are checked as a script's are.

`--preview` is needed to make an item, as a script's `previewfile` is. `--visibility` is
`public`, `friends-only`, `private` or `unlisted`. It is needed to make an item, since steamship
never chooses who sees one. For an item already made, either may be left out. Steam then keeps
what it has.

### The items file

`--items` names a file kept with the game. It holds the ID of each item made, by the name `--name`
gives it. A name may not begin with `.` and holds only letters, digits, `-`, `_` and `.`.

When the file has no item of that name, a new one is made. steamship writes its ID into the file
and makes the file if it is missing. When the file has one, that item is updated. Commit the file.
Then a run from any machine, CI among them, updates the items made before:

```json
{
  "app": 1000070,
  "items": {
    "example": 3000000001
  }
}
```

A new item is refused in CI unless `--new` is given, as for a script.

### Workshop logs and JSON

Each item has a folder of its own in steamship's home, named by `--name`, or for a script by its
`publishedfileid`, or `new` when it has none. The script steamcmd is given and steamcmd's console
go there. Uploading several items in a row leaves each one's console to read.

With `--json`, either form prints what came of the upload as one line of JSON on standard output,
once steamcmd has run. Everything meant for a person goes to standard error. Here the line is
spread out:

```json
{
  "app": 1000070,
  "item": 3000000001,
  "log": "/home/you/.local/share/steamship/workshop/1000070/example/steamcmd.log",
  "name": "example",
  "outcome": "made"
}
```

- `outcome` is `made`, `updated`, `failed` or `not-logged-in`.
- `item` is the item's ID, when there is one.
- `log` is where steamcmd's console was saved.
- `name` is the item's name in the `--items` file, or `null` for a script.

The exit code is the same as without `--json`. A run refused before steamcmd starts prints no
JSON.

## Builds and branches

```sh
steamship builds 480
steamship promote 480 --build 12345678 --branch testing
steamship branch 480 testing --description "0.3.0, the season update"
```

`builds` lists each branch with the build live on it. It also lists the last builds uploaded,
with the day each was uploaded and where it is live.

`promote` sets an uploaded build live on a beta branch without uploading it again. While the app
is unreleased, it also sets one live on the default branch, as below.

`branch` sets a beta branch's description, such as the version live on it. Players see it beside
the branch when they choose it in the game's Betas properties. Nothing is sent when the
description already reads so. The default branch's description is set in Steamworks only.

An app is named by its ID or by its app build script.

### Going back to the previous build

```sh
steamship rollback 480 testing
```

`rollback` sets a branch back to its previous build. Uploads [from a game](games.md) record the
commit and build ID they set live on each branch, in steamship's own folder. So `rollback` works
only on the machine that uploaded. A branch with no earlier recorded build is refused. The
default branch can be rolled back only while the app is unreleased, as with `promote`.

### The first branch of a new app

A new app has only its default branch. Steamworks lets you create others only once a build is
live on it. The first upload of a script that sets a beta branch live therefore leaves the build on
Steam, live nowhere. Then:

1. Set that build live on the default branch with `promote`, as below.
2. Create the branch the script names in Steamworks under SteamPipe, Builds.
3. Upload again, or `promote` the build.

### The default branch

```sh
steamship upload steam/app_build.vdf --version 1.4.0 --default
steamship promote 480 --build 12345678 --branch default
```

Until a game is released, nobody but its developers plays its default branch. In that time, Valve
sets a build live there for the publisher key alone. `upload --default` sets the build live on
the default branch too, after the branch the script sets live. `promote --branch default` sets an
uploaded one live there.

steamcmd's `SetLive` cannot name the default branch. Both go through the Web API instead, with the
key as for `promote`.

Before anything is sent, steamship asks steamcmd, logged in as the build account, for the app's
release state. It goes on only while Steam shows it `unavailable` (no store page yet) or
`prerelease` (coming soon).

With `--json`, `default` says whether the build is live on the default branch. `outcome` is
`not-live` when the app is unreleased and Steam did not set it there.

### Once a game is released

Once the game is released, its default branch is the one players get. Valve asks for a Steam
Mobile confirmation to set a build live there. That is done in Steamworks.

Then `upload --default` says so and uploads to the script's branch alone. A game's tools can pass
`--default` on every upload, from before launch to after it. `promote --branch default` is
refused. steamship never sends the SteamID that confirmation needs. Without it, Valve refuses too.

### Builds as JSON

For a script that runs steamship, `builds --json` prints the branches and builds as one line of
JSON on standard output. Everything meant for a person goes to standard error. Here the line is
spread out:

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

`password` says whether players need the branch's password. A build's `version` and `commit` are
read back from the description `upload` gave it. They are null for a build described otherwise.
Steam does not say when a build was set live.

When Steam does not answer, the line is an `error` with a `code` and a `message`. The exit code is
then 1:

```json
{"app": 480, "error": {"code": "no-build-live", "message": "Steam answered HTTP 500; ..."}}
```

`no-build-live` is Steam's answer while no build is live on the app's default branch. The other
codes are `key-refused`, `http-error`, `unreachable`, `unreadable` and `steam-failed`. A run
refused before Steam is asked, such as one with no key, prints no JSON and exits 2. With
`--json`, steamship never asks for a key.

### The Web API key

`builds`, `promote` and `branch` use Steam's partner Web API, which steamcmd cannot reach. They
need the publisher key of a group that holds the app, from Steamworks under Users & Permissions,
Manage Groups. The key comes from `STEAMSHIP_WEB_API_KEY`, or else from the one steamship keeps.
At a terminal with neither, steamship asks for it. Once the command works, it offers to keep it.

`steamship login` offers to keep one too. `steamship login --web-api-key` does only that, after
Steam has checked the key.

A kept key is in the system's credential store: Windows Credential Manager, the macOS Keychain,
or GNOME Keyring or KWallet on Linux. It is sent in a request header, never in an address. The
key can do everything its group may, for every app the group holds. Give it a group of its own
with only the apps and permissions it needs.

### When the Web API does not answer

A Web API request that gets no answer at all is made once more 10 seconds later. When the second
try fails too, the message says why and how long that try ran. The reason may be a host name that
was not found, a refused connection or a timeout.

## Fetching a build

```sh
steamship fetch 480
steamship fetch 480 --branch testing --system linux
steamship fetch 480 --build 12345678 --into play/spacewar --json
```

`fetch` downloads the build live on a branch exactly as Steam serves it to players. It can then be
started and checked before anyone is told it is ready. It asks the Web API, with the key as for
`builds`, which build is live on the branch. That is the default branch unless `--branch` names
another.

With `--build`, that build is fetched from a branch it is live on, the default first. Steam serves
only a branch's live build. A build live nowhere is refused. `promote` sets it live.

steamcmd then downloads the build with the saved login, as the build account. An app the build
account holds no licence for is a failure that says so.

### Branches with a password

A branch that takes a password, as its Steamworks betas page shows, is opened with the first of
these it finds:

- The password in `STEAMSHIP_BRANCH_PASSWORD`.
- The one kept for that app and branch in the system's credential store.
- One typed at the prompt or piped in.

With `--json` there is no prompt. steamship types the password into steamcmd's console, where no
other program on the machine sees it. It hides the password in everything it prints.

That console would split a password holding a space or a quote. `--password-on-command-line`
gives such a password on steamcmd's command line instead. There any program on the machine can
read it while the fetch runs.

### Keeping a branch password

Once Steam has taken a typed password, Enter keeps it for the next fetch. `--keep-password` keeps
it, or the one from the variable, without asking. A kept password that Steam refuses is
forgotten. The next fetch asks for the one the branch takes now.

```sh
steamship fetch 480 --branch testing --keep-password
```

### Where a fetched build goes

The depots are the ones Steam installs on this system, or on the one `--system` names
(`windows`, `linux` or `macos`). Each app, branch and system has its own folder under the
steamship's home, `fetched/<app>/<branch>-<system>`, or the one `--into` names. Nothing goes into
a Steam library and no window opens.

Fetching again checks each of the build's files in the folder and downloads only those that
differ. A folder holding another branch's
build is refused. So is one holding files but no build of the app.

### Checking what was fetched

steamcmd records the build it put in the folder in `steamapps/appmanifest_<app>.acf`. `fetch`
fails unless that is the build the Web API showed live, as when a new one is set live during the
download. It then prints the folder and the build. With `--json`, it prints them as one line on
standard output, here spread out:

```json
{
  "app": 480,
  "branch": "testing",
  "build_id": 12345678,
  "system": "linux",
  "folder": "/home/you/.local/share/steamship/fetched/480/testing-linux"
}
```

## Settings in Steamworks

```sh
steamship settings 480
steamship settings 480 --save steam/settings.vdf
steamship settings 480 --check steam/settings.vdf
```

A build runs only as Steamworks is set up for it:

- The install folder.
- Each launch option's executable and system.
- The Linux runtime.
- Each depot's system.
- The Common Redistributables the app ticks.
- Where Steam Cloud keeps saves.

None of that is in the build scripts. None of it can be changed but by hand in Steamworks.
`settings` asks Steam for it through steamcmd with the saved login, as the build account sees the
app. That works before the app is released. It shows the settings in Valve's own format.

### Steam Cloud and Steam Input

`settings` also says whether Steam Cloud is set up. An app with none has no Cloud section at all.

Cloud kept to developers is named as such, since players then have none while every check passes.
Steamworks can keep an app's Cloud to developers by itself the first time it is saved.

From the app's Steam Input page, `settings` names the defaults chosen there, as the page names
them. It gives the path of the action manifest for a Custom Configuration the game bundles. It
names the Steam Deck's touchscreen mode and the controllers opted into Steam Input too.

It says so when there is no Default Controller Configuration. A new app has none. On a Steam
Deck, a demo without one has had no input from the Deck's controls.

### Keeping a snapshot

`--save` keeps the settings in a file beside your scripts. `--check` exits 2 naming every setting
that differs from that file, such as `config/launch/0/executable`.

Run the check before a release. A launch option changed in Steamworks, by mistake or by someone
else, then stops the release. When the change was meant, save the file again and commit it.

The file leaves out Steam's bookkeeping (each depot's manifests and the build live on each
branch) and a released app's store details. It then changes only when the app's settings do.
Nothing on Steam is changed.

## Store, library and client artwork

```sh
steamship assets steam/store
```

Steamworks takes its artwork by hand, one upload at a time. Only then does it say that a picture
is the wrong size. `assets` checks a folder of artwork first, without sending anything. It checks
every PNG, JPEG, icon and zip in the folder against Valve's sizes and formats. Each file is named
for the asset it is.

`assets` exits 2 naming each file that is wrong and each named for no asset. It does the same when
there are too few screenshots.

### Names and sizes

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
| `shortcut_icon` | 256x256, or 512x512 as a PNG | Windows icon or PNG |
| `app_icon` | 184x184 | JPEG |
| `mac_icon` | any | Apple icon (ICNS) |
| `linux_icons` | square PNGs, such as 16x16, 32x32 and 256x256 | zip |
| `event_cover` | 800x450 | PNG or JPEG |
| `event_header` | 1920x622 | PNG or JPEG |
| `workshop_header`, the Workshop's branding image | 948x203 | JPEG |

Names match regardless of case. Screenshots are any files whose names start with `screenshot`,
such as `screenshot_01.jpg`. Other files in the folder are left alone.

### Artwork in each language

Steamworks takes the store page's and the library's artwork in each language. It reads the
language from the end of the file's name, in Steam's own name for it (its API language code). For
example: `header_capsule_koreana.png`, `library_logo_schinese.png`, `screenshot_03_brazilian.jpg`.

`assets` reads names the same way and checks a language's artwork against the same sizes. The
five screenshots it counts are those without a language. A language's own screenshots come beside
them.

### A folder for each language

A language's artwork can also be kept in a folder of its own inside the one checked. The folder
takes Steam's name for the language or a locale's, such as `german/` or `de/`, `brazilian/` or
`pt_BR/`. `assets` checks every file in such a folder as that language's, whether or not its name
ends in the language. It refuses one whose name says another language. Every other folder is
listed as skipped.

Steamworks reads the language only from the name. A file without it is uploaded with its language
chosen by hand. Steamworks reads no language from the names of the icons, event images, bundle
header and Workshop branding image. `assets` refuses one of those named for a language.

### Client icons

The shortcut, app, Mac and Linux icons go up under Installation, Client Images. A Windows icon is
as large as its largest image. A shortcut icon must therefore hold one of 256x256.

The Linux icons are one zip, `linux_icons.zip`, of a PNG for each size the game's icon comes in.
Their names and folders inside it are the game's own. `assets` names every file in it that is not
a square PNG. It passes over what macOS adds when it makes a zip, such as the `__MACOSX` folder.

## Steam DRM

```sh
steamship drm-wrap 480 build/windows/game.exe
steamship drm-wrap 480 build/windows/game.exe --output wrapped/game.exe --compatibility
```

`drm-wrap` has Valve's servers add the app's Steam DRM to a Windows executable before the build is
uploaded. It goes through steamcmd with the saved login. `--compatibility` asks for Valve's
compatibility mode, for an executable the default wrap breaks.

The executable is replaced by the wrapped one, or the wrapped one is written to `--output`. Either
way, nothing is replaced unless the wrap succeeded. The wrapped file keeps the original's
permissions.

Valve's tool adds Steam DRM only to an executable that already imports `GetModuleHandleA` from
`kernel32.dll`, as it says when it refuses one. Executables made by Godot and by Rust do not import
it and cannot be wrapped. steamship passes the refusal on with that explanation.

## Achievements and stats

```sh
steamship achievements 480
steamship achievements 480 --check steam/achievements.json
```

Achievements and stats are made and published in Steamworks. steamship changes nothing there.
`achievements` lists those Steam holds for the app, in English, with the same Web API key as
`builds`. Each achievement comes with the share of players who have it. Each stat comes with its
total over all players.

Steam gives those figures once players have the game. It gives a total only for a stat marked as
aggregated in Steamworks.

### Checking achievements

With `--check`, `achievements` compares them with a file kept beside your scripts. It exits 2
naming each of these:

- An achievement that is in the file but not on Steam, or the other way round.
- A display name, description or hidden flag that differs.
- An icon Steam has none of.
- With an import receipt, an icon changed since that import, as
  [Which icons Steam has](#which-icons-steam-has) describes.

It ends with how many of the file's achievements match, such as
`14 of 15 achievements match: 1 differs`. It counts the stats too when the file lists any. Run it
before a release. Any difference then stops the release.

### Achievements as JSON

`achievements --json` prints what Steam holds as one line of JSON on standard output. The
achievements and stats are in the keys the file below uses. Each achievement's `icon` and
`icon_locked` are added as addresses, or null where Steam has none.

Each achievement's `percent` is the share of players who have it. Each stat's `total` is its total
over all players. Either is null where Steam gives none. Steam failing is an `error` as
`builds --json` gives it. `--json` does not go with `--check`.

### The achievements file

The file lists each achievement by the API name the game unlocks it with and each stat by the one
the game sets it by:

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

`description` and `hidden` may be left out, for none and `false`. `app`, when there, must be the
app checked. Other keys, such as your own tools', are left alone.

Achievements and stats count on Steam only once published in Steamworks. One entered or imported
but not yet published is missing from Steam's, or shows as it was before. A check that fails says
so.

### Stats in the file

The stats are checked only when the file has a `stats` list. The check then names each stat
missing on either side and each display name or default that differs. A stat's `name` and
`default` may be left out, for none and 0.

A stat may also say whether it is `aggregated`, true or false. That is checked against the totals
Steam keeps of it over all players. Where Steam answers neither a total nor that it keeps none,
the check says so and leaves it.

A stat's minimum, maximum, maximum change, whether it only goes up and who may set it are not in
Steam's Web API. Only Steamworks shows them.

### games-kit records

A game made with games-kit can be checked against its own achievement records instead. They are a
list in which records typed `achievement` and `stat` are read as above, by the API name in
`steam`:

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

Records of other types are left alone. The records hold all of the game's stats, which are
therefore always checked. A stat's `name` is checked only when its record sets it. Its `default`
and `aggregated` are checked as games-kit gives them, 0 and `false` where the record says nothing.

### Several files

`--check` takes several files, after it or each after its own `--check`. It reads them together.
Achievements and stats kept in more than one file are checked as one list.

An achievement or stat in two of them is refused, with both files named. So is an achievement
and a stat of one API name, in one file or two. Steamworks saves such an import but refuses to
publish it. It says the achievement "has API name conflict with stat".

An achievement and a stat whose API names differ only in case are refused too. Steam publishes
them. It
then finds a stat by its API name in any case and answers for that stat as for the achievement.
Whether the stat is aggregated cannot be read.

### What the check cannot see

Steam's Web API does not give a stat's type, bounds, whether it only goes up or who may set it.
Nor does it give an achievement's progress stat and maximum, its set, whether it is archived or who
may unlock it.

A stat's `kind`, `min`, `max`, `max_change`, `increment_only` and `permission` therefore go
unchecked. So do an achievement's `stat`, `at`, `set`, `archived` and `permission`. The check says
so. They are still checked against each other, as
[What the editor refuses](#what-the-editor-refuses) describes.

### Every language

```sh
steamship achievements 480 --check steam/achievements.json steam/achievements/localisation.csv
```

A `.csv` file among them is a localisation in the columns Steamworks' achievement editor exports
under Bulk Import/Export. They are the API name, then `name` or `description`, then a column for
each language. Each language's column is headed with Steam's name for it, such as `german` or
`schinese`. The editor's headers for the first two columns are longer. steamship reads only that
they begin with `api_name` and `field`.

```text
api_name,field,english,german
FIRST_ANCHOR,name,Down by the Wheel,Hinab zum Rad
FIRST_ANCHOR,description,Drain a mill's first sluice.,Gib die erste Schleuse eines Mahlwerks frei.
```

### Languages on Steam

`--check` then asks Steam for the achievements in each of its languages too. It names each name or
description that differs, with its language. Steam answers a text it has no translation of in
English. A text the localisation has and Steam lacks is therefore said to be missing. A language
Steam holds nothing in is said once.

The editor's import can apply only part of a localisation the first time. This check catches it.
Every achievement the other files list must be in the localisation. Nothing else may be.

### Gettext catalogues

A game that keeps its translations in gettext catalogues gives those instead. Each `.po` file is in
the language its header's `Language` names, or else its file name, such as `de.po` or `pt_BR.po`.
steamship turns that into Steam's name for the language with Valve's table.

A games-kit record's name and description are under the contexts `achievement.name` and
`achievement.description`. A set's name is under `achievement_set.name`. Each is found by the
record's `id`, as games-kit keeps a record's translations.

In a file of the kind above, with no `id`, each text is under the context `steam achievements`, with
its English as the msgid. `steamship text --template` gives these entries for the game's template.

Every text a catalogue lacks, leaves empty or marks fuzzy is refused. Each message names the
catalogue, the context and the msgid.

```sh
steamship achievements 480 --check mods/base/data/achievements.json game/locale/*.po
```

### The achievement editor's import

```sh
steamship achievements 480 --write mods/base/data/achievements.json game/locale/*.po --out steam/achievements
steamship achievements 480 --write mods/base/data/achievements.json game/locale/*.po --out steam/achievements --unchanged
```

Steamworks' achievement editor takes a game's achievements as files dropped on it under Bulk
Import/Export. No Web API sets them. `--write` writes those files into the `--out` folder, from the
same files `--check` reads, without asking Steam anything:

- `480-achievements-definitions.csv`: each achievement's set, who may unlock it, and whether it is
  a spoiler (hidden) or archived. It also holds the stat its progress bar counts, from 0 up to its
  `at`.
- `480-achievements-localization.csv`: each name and description in English and in each language
  of the catalogues or localisation given.
- `480-achievement-sets-localization.csv`: each set's name in the same languages, when there are
  sets.
- `<API name>_ACHIEVED.png` and `<API name>_UNACHIEVED.png`: each achievement's `icon` and
  `icon_locked`, which the editor puts on the achievement they are named for.

### Progress bars

An achievement that unlocks at the first step above an `int` stat's `default` gets no bar. A bar
of one step only ever shows empty. An example is the first duel won, where another achievement
counts the same stat to a hundred.

### What else an achievement gives

Besides what the check reads, an achievement may give these:

- Its `set`.
- `archived` as true, for one that can no longer be earned. Valve asks for this in place of
  deleting one players hold.
- Its `permission`: `Client`, the default, `GameServer` or `OfficialGameServer`. A games-kit record
  words these as `client`, `game_server` or `official_game_server`.

Its icons are paths inside the games-kit mod its records are in, the folder holding the mod's
`manifest.json`. For a file of the kind above, they are paths beside that file. Each has to be a
square PNG of 64 pixels or more, 256 at best.

### Achievement sets

A set is listed with its set ID and English name, as `sets` in steamship's own file:

```json
{ "setid": "1", "name": "The Cellar" }
```

or as a games-kit record, which an achievement's `set` names by its `id`:

```json
{ "type": "achievement_set", "id": "base:cellar", "steam": 1, "name": "The Cellar" }
```

A set is made in the editor, which gives it its ID, before an import can put achievements in it.
The ID may be given as text or as a whole number.

### What the editor refuses

Before anything is written, and before `--check` asks Steam anything, steamship refuses the files
where the editor would. It exits 2 on any of these:

- An achievement that counts a `stat` with no `at`.
- An achievement that unlocks `at` a value with no stat, at 0 or below, or at a fraction of a stat
  whose `kind` is `int`.
- An achievement that counts a stat, or is in a set, that no file lists.
- A set with no English name.
- More than 40 sets with the base set.
- More than 5,000 achievements, archived ones included.

Over 100 achievements, steamship says that Steam takes them only once Valve has vetted the app.

### Keeping the folder in step

`--write` removes a file of the kinds written for the app that it would no longer write, such as
the icon of an achievement since removed. Other files are left alone.

With `--unchanged`, nothing is written. It exits 2 naming each file that is missing, differs or is
left over. In a game's own checks, that keeps the folder in step with the records.

### Importing in Steamworks

Once the files are written:

1. Drop every CSV and PNG file on the editor and Save imported data.
2. Drop the `achievements-localization.csv` again on its own. The editor applies only part of a
   localisation in the import that makes the achievements it names.

Each time, stay on the page until Pending Changes is gone. A page left sooner has applied only
some of the icons. Once published, `--check` reads them back from Steam in every language.

### What to look at after an import

Steam's Web API does not give an achievement's progress bar. `--check` cannot see it. When
`--write` changes a bar the folder's definitions gave before, it names the achievement with both
ranges. Look at each in the editor once the import is saved. Before publishing, set by hand any
bar that has not moved.

`--write` also names each achievement whose English name, English description or counted stat
changes while its icons stay as they were. Check that an icon drawn for what the achievement used
to ask still fits before it goes up.

### Which icons Steam has

```sh
steamship achievements 480 --write mods/base/data/achievements.json game/locale/*.po --out steam/achievements --imported
steamship achievements 480 --check mods/base/data/achievements.json --out steam/achievements
```

Once the import is saved and published, `--imported` fetches each icon Steam serves, for an
unreleased app too. It compares each with its file pixel by pixel, the file scaled to the size
Steam serves.

Steam serves a JPEG, which leaves every pixel of an icon it took within 48 of the file in each
channel, out of 255. An icon is not the file's when more than one pixel in a thousand is more than
64 off. That catches a changed detail ten pixels square. Pixels the file has see-through are left
out, since what Steam puts behind them is not known.

When every icon is alike, `--imported` writes `steam-icons.json` into the `--out` folder and
nothing else. This receipt records each icon's file by its SHA-256 and the image Steam made of it
by the name Steam serves it under.

### When the icons do not match

`--imported` is refused, exiting 2, in any of these cases:

- The folder is not as `--write` would write it.
- Steam does not have an icon's achievement, or the icon itself, yet.
- An icon's file changed since the last receipt, but Steam still has the image the last import
  made.
- Steam has an icon for an achievement the files give none, as one uploaded in the editor by hand.
- An icon Steam serves is not its file. The message says how many pixels are off and by how much
  on average.

A locked icon the files lack is left to Steam, which makes one when the import is told to. Steam
answers with what is published, the Web API and the images alike. An import saved in the editor
but not yet published still shows the icons from before it.

### Checking against the receipt

`--check` with that folder as `--out` then reads the receipt as well, without fetching the images.
It names each of these icons:

- One whose file changed since the import the receipt records.
- One whose image on Steam changed though its file did not.
- One the receipt has no import of.

A folder with no receipt is refused. `--write` and `--unchanged` leave `steam-icons.json` alone.
A game's own checks can therefore run `--unchanged` without Steam. Keep the receipt with the
records.

## Leaderboards

```sh
steamship leaderboards 480
steamship leaderboards 480 --check steam/leaderboards.json
steamship leaderboards 480 --check steam/leaderboards.json --create
steamship leaderboards 480 --check steam/leaderboards.json --create --delete-missing
```

`leaderboards` lists those Steam holds for the app, with the same Web API key as `builds`. With
`--check`, it compares them with a file, by name. It exits 2 naming each leaderboard that is in
the file but not on Steam, or the other way round. It also names each setting that differs.

With `--create` too, the file's leaderboards that Steam lacks are made first, as the file has them.
A release can then bring in a new one before the build that uses it.

### Deleting and changing leaderboards

With `--delete-missing`, the leaderboards on Steam that the file does not hold are deleted first,
with their scores. A renamed leaderboard then leaves no old one behind. With nothing to delete,
steamcmd is not asked.

steamship deletes one only while steamcmd's app info shows the build account the app unreleased,
as for setting the default branch live. Once players have the game, it exits 2 naming the
leaderboards. Delete them in Steamworks, under Stats & Achievements, Leaderboards, or put them back
in the file.

Steam keeps an existing leaderboard's settings. Asked to make one that is already there, it
answers with the one it has. steamship therefore never changes or remakes a leaderboard. A setting
that differs is changed in Steamworks, under Stats & Achievements, Leaderboards.

### The leaderboards file

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

`sort` is `ascending` (the lowest score first) or `descending`. `display` is `numeric`, `seconds`
or `milliseconds`. Both are needed, since steamship makes no leaderboard with one guessed.

`trusted_writes` (only the Web API may set scores, never the game) and `friends_only` (players see
only their friends' scores) may be left out, for `false`. `app`, when there, must be the app
checked. Other keys are left alone.

### Leaderboards in games-kit records

A game built on games-kit keeps its leaderboards as records, beside its achievements and stats.
`--check` reads them as it reads those. In a list of records, each one typed `leaderboard` is read
the same way, by its Steamworks name in `steam`. A record's `upload` and `details` are what the
game sends with each score, not settings Steam keeps. They are left alone.

`--check` takes several files, after it or each after its own `--check`. The files given to
`achievements --check` can be given here too. A leaderboard in two of them is refused, with both
files named.

```sh
steamship leaderboards 1000050 --check mods/base/data/achievements.json mods/base/data/boards.json
```

### Dated leaderboards

A leaderboard with a `period` of `day`, `week` or `month` is a template. The game makes one board
each period on first use, with the template's sort and display. Each board is named after the
template and the UTC date, as `DAILY_20261025`, `WEEKLY_2026W43` (the ISO week) or
`MONTHLY_202610`.

No board has the template's own name. `--create` therefore makes none. The check does not report
it missing from Steam. Each dated board on Steam is compared with its template. It is never
reported as missing from the file, nor deleted by `--delete-missing`.

steamship refuses a template in any of these cases:

- It has `trusted_writes` or `friends_only`, which Steamworks sets board by board.
- Its name and the date would pass Steam's 128 bytes.
- Another leaderboard has a name it would make.

`period` is `none` when left out. steamship's own file takes it too.

## Rich presence

```sh
steamship rich-presence 480 steam/rich_presence_english.vdf steam/rich_presence_german.vdf
steamship rich-presence 480 steam/rich_presence_*.vdf --preview
```

`rich-presence` uploads the localisation files Steamworks takes under Community, Rich Presence,
with the same Web API key as `builds`. They are in Valve's own format, one per language:

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

Each file replaces all of its language's tokens on Steam. A token taken out of a file is gone from
Steam after the next upload. Languages with no file are left as they are. `--preview` checks the
files and sends nothing.

### What a rich presence file may not hold

Every file is checked before anything is sent. A file is refused with exit 2 when it has any of
these:

- No language named.
- No tokens.
- A token that does not start with `#`, or is there twice.
- A text that refers to tokens, as `{#season_%season%}` does, when no token starts `#season_`.
- A text holding a slot Steam cannot fill, which Steam would show as written.

Two files for the same language are refused too. Steam fills only `%field%` and `{#Token}` or
`{#Token_%field%}`. A `{guild}`, `{0}` or lone `}` is therefore refused. So is a `%` that starts no
field, as in `50%`. Write "per cent" instead.

A game's `check` reads its files the same way.
