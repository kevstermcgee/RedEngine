# What the Idea Forge game agent can reach, and the smallest change that removes it (2026-10-18, proposal, not built)

Player reviews (anonymous, from a public relay) are quoted in every Idea Forge brief (`player_reviews.players_section`). So text anyone can write
reaches an agent that runs unattended at 06:00. This note says what that agent can do beyond making a game, measured on the owner's machine,
and proposes the smallest change that takes it away. Nothing here is built yet.

## How the agent runs today

`idea_forge.agent_argv` starts:

    claude -p <brief> --allowedTools "Bash Read Edit Write Glob Grep"
           --disallowedTools "Bash(git push:*) Bash(gh:*) Bash(curl:*) Bash(wget:*)" --permission-mode acceptEdits

It runs as the owner, in the run's worktree, with the nightly's whole environment (`os.environ` plus `IDEA_FORGE_RUN`, `RED_TRACE`,
`IDEA_FORGE_HOME`). The deny-list matches command prefixes, so it stops `git push ...` but not `git -C . push`, `/usr/bin/git push`,
`env git push`, `sh -c 'git push'`, or `python3 -c "import subprocess; ..."`. `Bash` is allowed as a whole.

What a Bash command in that agent can use (checked 2026-10-18; no secret was printed):

| What | Where | What it allows |
| --- | --- | --- |
| GitHub login `kevstermcgee` | `~/.config/gh/hosts.yml`, and `~/.gitconfig` names `gh auth git-credential` as git's credential helper | Push to RedEngine (`main` has no branch protection), push to RedEngineGames, the whole GitHub API with the token read from the file |
| SSH key | `~/.ssh/id_ed25519`, **no passphrase**; `SSH_AUTH_SOCK` is in the systemd user environment | `git push git@github.com:...` |
| Anthropic login | `~/.claude/.credentials.json` | Spending on the owner's Claude account (the `claude` process itself needs this file) |
| A writable home | everything under `/home/kevin` | Persistence: `~/.config/systemd/user` timers, crontab, `~/.local/bin/redengine-nightly` (the 23:00 job, which runs Claude with `--permission-mode bypassPermissions` and pushes to `main`), other worktrees |
| Network | unrestricted | Anything: Python's `urllib`, `git`, `ssh`, `pip`, `nc` |

The agent needs none of the first two. The tool, not the agent, pushes the branch, opens the PR and merges (`do_ship`, `settle`, publishing),
in the parent process.

## The smallest change

**Run the agent inside `bwrap` (bubblewrap, already installed at `/usr/bin/bwrap`) with the credentials hidden and the home read-only, and
with a scrubbed environment.** It is one prefix on `agent_argv` and a few `env` keys in `do_run` and `do_improve`, about 20 lines:

    bwrap --ro-bind / / --dev /dev --proc /proc --tmpfs /tmp
          --tmpfs ~/.ssh --tmpfs ~/.config/gh            # no SSH key, no GitHub token
          --ro-bind <run_dir>/gitconfig ~/.gitconfig     # user.name and user.email only: no credential helper
          --bind <worktree> <worktree>                   # the game, its build output (the seeded target/ is inside it)
          --bind <run_dir> <run_dir>                     # notes, trace, transcript
          --bind ~/.claude ~/.claude                     # claude's own session state
          --unsetenv SSH_AUTH_SOCK --unsetenv GH_TOKEN --unsetenv GITHUB_TOKEN
          --die-with-parent -- claude -p ...

Effect: no GitHub token and no SSH key in reach, so a pushed branch or a changed repository can come only from the tool's own `ship` step,
with its path limits (`allowed_path`) and CI. The home is read-only outside the worktree, the run directory and `~/.claude`, so the agent
cannot plant a timer or rewrite the 23:00 job. A game still builds and commits locally.

## What it leaves, and the next step

- **Network.** The `claude` process must reach the Anthropic API, so the network cannot be cut for the whole process. Its Bash commands could be
  cut off by Claude Code's own sandbox (bubblewrap plus a filtering proxy), which on Linux also needs `socat`, and `socat` is not installed. That is
  the second step: install it, turn the sandbox on for the agent, allow no domains, and keep `RED_NO_FETCH=1`, since the worktree is seeded
  and needs no download.
- **The Anthropic login** stays readable, because the agent cannot run without it. A leak costs money, not the repository.
- **Prompt injection still shapes what the agent writes**: the game and its feedback file. That output goes through the tool's checks
  (`allowed_path`, `score`) and CI, and an auto-merge happens only for the tool's own PRs within their path limits. The reviews themselves are
  now visible-text only and capped per game per day.

## How to check it once built

Add a canary run, with a stub brief and no model spend, whose agent tries each of these: `git -C . push`, `cat ~/.config/gh/hosts.yml`,
`ssh -T git@github.com`, writing `~/.config/systemd/user/x`, and `python3 -c "import urllib.request; urllib.request.urlopen('https://example.com')"`.
The first four must fail. The fifth fails only after the second step. Then run one real night and confirm that the game builds and that `ship`
still pushes and opens the PR.
