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

- `SetLive` naming `default` or `public`, which Valve only allows from the Steamworks site, or a
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

`--preview` is Valve's dry run: the whole build is computed and logged, nothing is uploaded and
nothing is set live.

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

## Branches and promoting

```sh
steamship builds 480
steamship promote 480 --build 12345678 --branch testing
```

`builds` lists each branch with the build live on it, and the last builds uploaded, with the day
each was uploaded and where it is live. `promote` sets an uploaded build live on a beta branch
without uploading it again; the default branch is set live in Steamworks only, so `promote`
refuses it. An app is named by its ID or by its app build script.

A new app has only its default branch, and Steamworks lets you create others only once a build
is live on it. So the first upload of a script that sets a beta branch live leaves the build on
Steam, live nowhere. In Steamworks, under SteamPipe, Builds, set that build live on the default
branch, create the branch the script names, then upload again or `promote` the build.

Both use Steam's partner Web API, which steamcmd cannot reach, with the publisher Web API key of
a group that holds the app, from Steamworks under Users & Permissions, Manage Groups. The key
comes from `STEAMSHIP_WEB_API_KEY`, or else from the one steamship keeps. At a terminal with
neither, it is asked for, and once the command works, offered to be kept. `steamship login`
offers to keep one too, and `steamship login --web-api-key` does only that, after Steam has
checked it.

A kept key is in the system's credential store: Windows Credential Manager, the macOS Keychain,
or GNOME Keyring or KWallet on Linux. It is sent in a request header, never in an address. The
key can do everything its group may, for every app the group holds, so give it a group of its
own with only the apps and permissions it needs.

## Achievements

```sh
steamship achievements 480
steamship achievements 480 --check steam/achievements.json
```

Steamworks is where achievements are made and published, and steamship changes nothing there.
`achievements` lists those Steam holds for the app, in English, with the same Web API key as
`builds`. With `--check`, it compares them with a file kept beside your scripts, and exits 2
naming each achievement that is in the file but not on Steam or the other way round, each
display name, description or hidden flag that differs, and each icon Steam has none of. Run it
before a release, and the release waits while the two have drifted apart.

The file lists each achievement by the API name the game unlocks it with:

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
  ]
}
```

`description` and `hidden` may be left out, for none and `false`. `app`, when there, must be
the app checked, and other keys, such as your own tools', are left alone. Achievements count on
Steam only once published in Steamworks, so one entered but not yet published is missing from
Steam's.

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
        "#running"          "Running the %guild% guild in {#season_%season%}"
        "#season_autumn"    "Autumn"
    }
}
```

Each file replaces all of its language's tokens on Steam, so the files are the whole truth: a
token taken out of a file is gone from Steam after the next upload, and languages with no file
are left as they are. Every file is checked before anything is sent, and one that names no
language, holds no tokens, has a token that does not start with `#` or is there twice, or a
text that refers to tokens (`{#season_%season%}`) that none starts as, is refused with exit 2;
so are two files for the same language. `--preview` checks the files and sends nothing.
