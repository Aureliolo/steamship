# Uploading from CI

`steamship ci`, run once from your own machine, sets up uploads from GitHub Actions. The login
goes into a repository secret. A step in your workflow then uploads each build. No password ever
reaches the CI.

## GitHub Actions

```sh in your game's repository, after steamship login
steamship ci
```

`ci` takes the repository from `origin` and finds the app build script by its contents. It checks
the scripts but not the content they map, which need not be built yet. It logs in with the saved
login to be sure Steam takes it. Then it sets the login as the repository secret
`STEAMSHIP_LOGIN` through the GitHub command line, [`gh`](https://cli.github.com). Last, it shows
the step to add to your workflow, after the step that builds your content:

```yaml
- name: Upload to Steam
  uses: Aureliolo/steamship@<commit> # vX.Y.Z
  with:
    command: upload
    app: steam/app_build.vdf
    version: ${{ github.ref_name }}
    login: ${{ secrets.STEAMSHIP_LOGIN }}
```

The action is listed on the GitHub Marketplace as
[Upload to Steam with steamship](https://github.com/marketplace/actions/upload-to-steam-with-steamship).
It installs the steamship release it is pinned to. It refuses that release unless its
SHA-256 and its attestation check out. Then it runs the command, and gives the BuildID as its
`build-id` output. The outputs are set on a failed step too.

The action runs on Linux, Windows and macOS runners. Renovate raises its pin by itself.

### Inputs

`command` is `upload`, the default, or `workshop`. Most other inputs are a flag of that command
under the flag's name. `account`, `login`, `web-api-key` and `home` set an environment variable
instead, and `app` is the command's first argument. In a game's folder, leave `app` and `version`
out.

| Input           | Command    | Flag or use                                                   |
| --------------- | ---------- | ------------------------------------------------------------- |
| `command`       | both       | `upload`, the default, or `workshop`                          |
| `app`           | both       | The app, or for `workshop` an item script                     |
| `dry-run`       | both       | `--dry-run`                                                   |
| `account`       | both       | The build account, as `STEAMSHIP_ACCOUNT`                     |
| `login`         | both       | The login from `steamship ci`, required, as `STEAMSHIP_LOGIN` |
| `web-api-key`   | both       | The publisher key, as `STEAMSHIP_WEB_API_KEY`                 |
| `home`          | both       | steamship's home folder, as `STEAMSHIP_HOME`                  |
| `version`       | `upload`   | `--version`                                                   |
| `default`       | `upload`   | `--default`                                                   |
| `demo`          | `upload`   | `--demo`                                                      |
| `allow-ungreen` | `upload`   | `--allow-ungreen`                                             |
| `new`           | `workshop` | `--new`                                                       |
| `for-launch`    | `workshop` | `--for-launch`                                                |
| `visibility`    | `workshop` | `--visibility`                                                |
| `name`          | `workshop` | `--name`                                                      |
| `items`         | `workshop` | `--items`                                                     |
| `content`       | `workshop` | `--content`                                                   |
| `preview-image` | `workshop` | `--preview-image`                                             |
| `title`         | `workshop` | `--title`                                                     |
| `description`   | `workshop` | `--description`                                               |
| `change-note`   | `workshop` | `--change-note`                                               |

A boolean is `true` or `false`, and anything else is refused before steamship starts. So is an
input the other command takes, such as `new` with `command: upload`, so that no input is ignored
without a word. The step exits with steamship's own code.

### What `ci` asks for

`ci` asks for anything it cannot find. The app is its first word, by its ID or its app build
script's path. `--repo`, `--account` and `--secret` give the other answers up front.

The secret holds the token steamcmd saved, the account's name and the day `ci` saved it. It never
holds a password or a Steam Guard secret. `ci` shows none of it. It is a few kilobytes. It leaves
out steamcmd's list of the servers it last reached, which steamcmd finds again.

The secret starts with `steamship-login-` and its format's number. An older steamship that meets
a newer format asks you to upgrade steamship, or raise the action's pin.

### The default branch

Until the game is released, `default: true` sets each build live on the default branch too, as
[`upload --default`](uploading.md#the-default-branch) does. That goes through the partner Web
API, which needs the publisher key. Keep the key as a secret of its own and give it as
`web-api-key: ${{ secrets.STEAMSHIP_WEB_API_KEY }}`. With the key, steamship also confirms the
build live on the script's branch. Once Steam shows the game released, the step says so and
uploads to the script's branch alone.

### Content built in another job

When your content is built in another job, hand it over as a tar archive.
`actions/upload-artifact` does not keep file permissions. A Linux or macOS program would arrive
without its executable bit, which `check` refuses.

### A Workshop item

A Workshop item goes up with `command: workshop`, and its `workshopitem` script as `app`:

```yaml
- name: Upload the Workshop item
  uses: Aureliolo/steamship@<commit> # vX.Y.Z
  with:
    command: workshop
    app: workshop/item.vdf
    login: ${{ secrets.STEAMSHIP_LOGIN }}
```

The item's ID is its `item-id` output. A script with no `publishedfileid` is refused there, since
every run would make another item. Make the item once and add its ID to the script. Or set
`new: true` for the one run that makes it.

### Keeping the home

The action gives each job a home of its own. When the job ends, whatever the outcome,
`steamship logout` removes the login, the account, the key and an install's working folders from
it. A workflow that wants to keep the home's chunk cache and the record `rollback` reads sets
`home:` and caches that folder. The [home folder's table](install.md#the-home-folder) says what
each entry holds.

```yaml
- uses: actions/cache@<commit> # vX.Y.Z
  with:
    path: |
      ${{ runner.temp }}/steamship/apps
      !${{ runner.temp }}/steamship/apps/*/output/*.log
      ${{ runner.temp }}/steamship/recorded-builds
    key: steamship-${{ github.run_id }}
    restore-keys: steamship-
- name: Upload to Steam
  uses: Aureliolo/steamship@<commit> # vX.Y.Z
  with:
    app: steam/app_build.vdf
    version: ${{ github.ref_name }}
    login: ${{ secrets.STEAMSHIP_LOGIN }}
    home: ${{ runner.temp }}/steamship
```

A cache cannot be updated, so each run saves a new one under its own key and restores the newest.
The cache leaves out the build logs, which a restored job has no use for. `logout` has removed the
login, the account and an install's working folders by then, so the cache holds none of them.

### Steam's logs

When an upload or a Workshop item fails in CI, steamship prints Steam's own logs after the
reasons. Each is a collapsed group: steamcmd's console, the app and depot build logs, and what
steamcmd added to its own logs during the run. All of it is redacted as steamship's logs always
are. The account's name is masked too.

It prints them too when a job on Linux or macOS is cancelled or runs past its
`timeout-minutes`. At the first signal steamship stops steamcmd and prints the logs and why it
stopped. Then it ends as the signal would have. A second signal ends it at once. On Windows,
steamcmd ends with steamship and nothing of it is printed.

## When the token expires

An upload in CI exits 3 when the token has expired, and says the day `ci` saved the login.
`steamship status` in CI shows that day too. Run `steamship login` and `steamship ci` again, and
the next run uploads.

## When Steam limits logins

Exit 6 means Steam is limiting login attempts. Wait, then run again with the same login. A new
login only adds attempts.

## When a second run would duplicate an upload

Exit 5 means Steam holds a build or Workshop item that a second run would duplicate. The output
says what to do. Do not run the same upload again.

## Another CI

```sh
steamship ci --output login.txt
```

`--output` writes the login to a file in place of a secret. The file is readable by you alone.
Give its contents to your CI as the variable `STEAMSHIP_LOGIN`, then
delete the file. `steamship upload` reads the login from that variable. Nothing else about
uploading changes.

A job in a Docker container needs `--security-opt seccomp=unconfined`. Docker's default seccomp
filter refuses the socket call Valve's 32-bit steamcmd makes. steamcmd then never reaches Steam.
steamship says so when it happens. On Linux, the container also needs the 32-bit C libraries
[Install](install.md) names.

## Keeping steamship current

A tool that installs steamship from the release archives can pin it in any JSON file. The pin is
a `"steamship"` object that holds, for each target, the release's tag and the archive's SHA-256:

```json
"steamship": {
  "x86_64-pc-windows-msvc": { "tag": "v{{version}}", "sha256": "<the .sha256 beside it>" },
  "x86_64-linux-musl": { "tag": "v{{version}}", "sha256": "<the .sha256 beside it>" }
}
```

The archive is `steamship-<version>-<target>.tar.gz`, or `.zip` for Windows, from the tag's
release. With this in your Renovate configuration, Renovate raises the tag and the digest
together:

```json
"extends": ["github>Aureliolo/steamship//.github/renovate-preset"]
```
