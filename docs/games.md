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
   again if it changed it. So is every file the depots map that the build did not write, such as
   an export left over from an earlier build: its change time, which every write, copy and
   rename sets, is older than the build.
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
