# 2026-10-04. Games ship as versioned installers that update in place
Status: accepted
Summary: Each game is released on its own as a per-user installer and a portable ZIP, updates itself from the site, keeps its saves in Saved Games, and every version stays downloadable.

## Context
Games were published as one GitHub Release per RedEngine commit holding a ZIP per game (28 games, a full engine copy in each). That had three problems the user named: a game could not be updated (a new ZIP is a new folder, and settings were keyed by the *folder's path*, so progress such as Marcel's days lived was lost with it); a ZIP is not an install (no Start-menu entry, shortcut or uninstaller), and unsigned downloads trip SmartScreen; and the website offered only the latest release, while the releases themselves carried no per-game version, so "an older version of a game" was not even defined.

## Decision
- **A game has its own version**, a whole number that grows each time its *content* changes (the git object ids of its files, its name and command line, fingerprinted by `release_tool.py plan`). The history is read back from the GitHub Releases (`<slug>-v<N>`, with a `redengine-release` line in the notes), so there is no version file to keep in step. A RedEngine change alone does not re-release every game; a manual run with `force` or `only` does.
- **Each release is an installer and a portable ZIP.** The installer (Inno Setup) is per-user, needs no administrator rights, offers a desktop shortcut and registers an uninstaller; its identity is a fixed GUID per game, so a newer installer is an update in place. The installed `Play-<slug>.exe` asks the site's `games/<slug>/latest.json` at start, shows what changed, and on *Yes* downloads the installer, checks its SHA-256 and runs it silently. A smoke test installs, updates (twice), starts and uninstalls on the runner before anything is published.
- **Saves follow the game, not the folder.** `RE2_SAVE_DIR` (set by the launcher to `Saved Games\<game>`) holds a game's settings and saved variables (`settings::state_path`); what was saved the old keyed way is read once.
- **The website lists every version** of every game (what changed, installer, ZIP, SHA-256), defaults to the latest, and shows earlier ZIP-only builds as "Earlier builds".
- **Code signing is a hook, not a promise.** The workflow signs with a certificate when the repository has one (`SIGN_PFX_BASE64`); without it SmartScreen warns, and the site says how to continue. Removing the warning needs a certificate from a public CA, which only the owner can obtain.

## Consequences
Updating needs no reinstall and loses nothing; old versions stay one click away. Each release is about 7.5 MB twice (installer and ZIP) because every game carries its own engine copy; only changed games are rebuilt, and nothing at all runs on a Windows runner when nothing changed. The old desktop launcher is no longer fed. Undo: restore the bundle workflow in RedEngineGames; installed games keep working either way.
