//! A scoped Git commit: change one subtree of a repository and commit exactly that, whatever else is going on in the checkout.
//!
//! Publishing writes generated files into a repository that other people and other AIs are working in. A plain `git add DIR && git commit` commits the *whole index*, so anything
//! somebody else had already staged would ride along. This module never uses the checkout's index to build the commit:
//!
//! 1. the subtree is materialised from `HEAD` (not from the working tree) into a scratch directory, with a temporary index, and the caller's `edit` changes it there;
//! 2. the new tree is `HEAD`'s tree with only that subtree replaced (`write-tree` of the temporary index), and the commit is made with `commit-tree`: no hook, no editor, no staged work;
//! 3. the commit reaches the checkout the way a branch switch does (`read-tree -m -u HEAD NEW`): only the paths the publication changed are rewritten, staged and unstaged work
//!    elsewhere is not looked at, and git itself refuses (changing nothing) if a changed path holds somebody's modification;
//! 4. the branch moves with a compare-and-swap (`update-ref HEAD NEW OLD`), so a commit made in between is never overwritten;
//! 5. a failed push takes the commit back out again: a failed publication leaves the repository as it found it.
//!
//! Everything that could lose work stops *before* anything is written, with the command that fixes it (see [`commit_subtree`]). Nothing here resets, stashes, unstages or cleans.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// The author address of every commit made here (how a later `--push` tells a publication from somebody's work).
const PUBLISH_EMAIL: &str = "publish@redengine.invalid";

/// What a scoped commit did.
#[derive(Debug, Clone, Default)]
pub struct Committed {
    /// The new commit, or `None` when the subtree already had exactly this content (nothing was committed).
    pub commit: Option<String>,
    /// The commit `HEAD` pointed at before.
    pub parent: String,
    /// The paths the commit changed (added, modified or deleted), repository-relative.
    pub changed: Vec<String>,
    /// The commit was pushed to `origin`.
    pub pushed: bool,
}

/// How to commit.
pub struct Scope<'a> {
    /// The repository root (the directory holding `.git`).
    pub repo: &'a Path,
    /// The subtree this commit may change, repository-relative (`projects/coin-dash`).
    pub subtree: &'a str,
    /// The commit message.
    pub message: &'a str,
    /// Push `HEAD` to `origin` after committing (and take the commit back if that fails).
    pub push: bool,
}

struct Git {
    root: PathBuf,
}

impl Git {
    /// Runs git in the repository, without the `GIT_*` variables a hook or a parent git might have left in the environment.
    fn cmd(&self, args: &[&str]) -> Command {
        let mut c = Command::new("git");
        c.arg("-C").arg(&self.root);
        for v in ["GIT_DIR", "GIT_INDEX_FILE", "GIT_WORK_TREE", "GIT_OBJECT_DIRECTORY", "GIT_COMMON_DIR"] {
            c.env_remove(v);
        }
        c.args(args);
        c
    }
    fn run_bytes(&self, mut c: Command, stdin: Option<&[u8]>) -> Result<Vec<u8>, String> {
        let what = format!("{:?}", c.get_args().skip(2).collect::<Vec<_>>());
        c.stdout(Stdio::piped()).stderr(Stdio::piped()).stdin(if stdin.is_some() { Stdio::piped() } else { Stdio::null() });
        let mut child = c.spawn().map_err(|e| format!("cannot run git: {e} (is git installed and on PATH?)"))?;
        if let (Some(bytes), Some(mut si)) = (stdin, child.stdin.take()) {
            // A closed pipe (git exited early) is reported by the exit status below.
            let _ = si.write_all(bytes);
        }
        let o = child.wait_with_output().map_err(|e| format!("git: {e}"))?;
        if o.status.success() {
            Ok(o.stdout)
        } else {
            Err(format!("git {what}: {}", String::from_utf8_lossy(&o.stderr).trim()))
        }
    }
    fn out(&self, args: &[&str]) -> Result<String, String> {
        self.run_bytes(self.cmd(args), None).map(|b| String::from_utf8_lossy(&b).trim().to_string())
    }
    fn ok(&self, args: &[&str]) -> bool {
        self.cmd(args).stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|s| s.success())
    }
    /// A command that works on a private index and a private working directory (the scratch copy).
    fn scratch(&self, args: &[&str], index: &Path, work: &Path) -> Command {
        let mut c = self.cmd(args);
        c.env("GIT_INDEX_FILE", index).env("GIT_WORK_TREE", work);
        c
    }
}

/// A scratch directory inside the git directory (same disk, never shown by `git status`), removed on drop.
struct Scratch(PathBuf);

impl Scratch {
    fn new(git_dir: &Path) -> Result<Scratch, String> {
        // A crashed earlier run may have left its scratch behind: those are ours (the prefix), at least an hour old, and hold only copies.
        if let Ok(rd) = std::fs::read_dir(git_dir) {
            for e in rd.flatten() {
                let stale = e.file_name().to_string_lossy().starts_with("redengine-publish-")
                    && e.metadata().and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).is_some_and(|age| age.as_secs() > 3600);
                if stale {
                    let _ = std::fs::remove_dir_all(e.path());
                }
            }
        }
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
        let p = git_dir.join(format!("redengine-publish-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&p).map_err(|e| format!("cannot create a scratch directory in {}: {e}", git_dir.display()))?;
        Ok(Scratch(p))
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// One path the new commit changes.
#[derive(Debug, Clone)]
struct Change {
    path: String,
    /// The blob at `HEAD` (`None`: the commit adds the file).
    old: Option<String>,
    /// The blob in the new commit (`None`: the commit deletes the file).
    new: Option<String>,
}

fn parse_diff_tree(raw: &[u8]) -> Vec<Change> {
    let text = String::from_utf8_lossy(raw);
    let mut parts = text.split('\0').filter(|s| !s.is_empty());
    let mut out = Vec::new();
    while let (Some(meta), Some(path)) = (parts.next(), parts.next()) {
        let f: Vec<&str> = meta.trim_start_matches(':').split_whitespace().collect();
        if f.len() < 5 {
            continue;
        }
        let blob = |s: &str| if s.chars().all(|c| c == '0') { None } else { Some(s.to_string()) };
        out.push(Change { path: path.to_string(), old: blob(f[2]), new: blob(f[3]) });
    }
    out
}

/// Commits what `edit` does to `scope.subtree` and nothing else. `edit` receives the scratch copy of the subtree (`HEAD`'s version of it, or an empty directory if `HEAD` has none) and
/// changes it; it returns when the subtree is the way it should be committed.
///
/// Refuses, *before writing anything*, with the exact recovery step, when:
/// * the repository is not a normal checkout on a branch (no commit yet, detached `HEAD`, merge/rebase/cherry-pick/revert/bisect in progress),
/// * one of the files this publication replaces is staged, modified or in the way in the checkout (git would have to overwrite somebody's work: the files are named),
/// * `push` is asked and the branch holds commits `origin` does not have (they would be published along with the game).
///
/// Anything the checkout holds that the commit does not change (staged, unstaged, partially staged, untracked, other games' files) is neither read into the commit nor changed.
pub fn commit_subtree(scope: &Scope, edit: impl FnOnce(&Path) -> Result<(), String>) -> Result<Committed, String> {
    let repo = scope.repo;
    if !repo.join(".git").exists() {
        return Err(format!("{} is not a git checkout (clone https://github.com/kevstermcgee/RedEngineGames and pass its path with --repo)", repo.display()));
    }
    let git = Git { root: repo.to_path_buf() };
    let top = git.out(&["rev-parse", "--show-toplevel"])?;
    let same = |a: &Path, b: &Path| std::fs::canonicalize(a).ok().zip(std::fs::canonicalize(b).ok()).is_some_and(|(a, b)| a == b);
    if !same(Path::new(&top), repo) {
        return Err(format!("{} is inside the repository at {top}: pass that root as --repo", repo.display()));
    }
    let git_dir = {
        let d = git.out(&["rev-parse", "--absolute-git-dir"])?;
        PathBuf::from(d)
    };
    // ---- refuse states we do not want to commit into --------------------------------------------------------------------------------------------------
    for (marker, what) in [
        ("MERGE_HEAD", "a merge"),
        ("CHERRY_PICK_HEAD", "a cherry-pick"),
        ("REVERT_HEAD", "a revert"),
        ("BISECT_LOG", "a bisect"),
        ("rebase-merge", "a rebase"),
        ("rebase-apply", "a rebase or `git am`"),
    ] {
        if git_dir.join(marker).exists() {
            return Err(format!(
                "{} is in the middle of {what}: finish or abort it first (`git -C {} status` says how), then publish again. Nothing was changed.",
                repo.display(),
                repo.display()
            ));
        }
    }
    let parent = git.out(&["rev-parse", "--verify", "-q", "HEAD"]).map_err(|_| {
        format!(
            "{} has no commit yet: make the first commit (`git -C {} commit --allow-empty -m init`), then publish again. Nothing was changed.",
            repo.display(),
            repo.display()
        )
    })?;
    let branch = git.out(&["symbolic-ref", "-q", "--short", "HEAD"]).map_err(|_| {
        format!(
            "{} is on a detached HEAD: check out a branch (`git -C {} switch main`), then publish again. Nothing was changed.",
            repo.display(),
            repo.display()
        )
    })?;
    if scope.push {
        let upstream = git
            .out(&["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}"])
            .ok()
            .or_else(|| git.ok(&["rev-parse", "--verify", "-q", &format!("refs/remotes/origin/{branch}")]).then(|| format!("origin/{branch}")));
        if let Some(up) = upstream {
            // Earlier publications that were committed without --push are ours: pushing them is what this run is for. Anything else is somebody's work.
            let range = format!("{up}..HEAD");
            let log = git.out(&["log", "--format=%H %ae", &range]).unwrap_or_default();
            let mut foreign = 0;
            for line in log.lines() {
                let (sha, email) = line.split_once(' ').unwrap_or((line, ""));
                let files = git.out(&["diff-tree", "--no-commit-id", "--name-only", "-r", "-m", "--root", sha]).unwrap_or_default();
                let ours = email == PUBLISH_EMAIL && files.lines().all(|f| f.starts_with(&format!("{}/", scope.subtree)));
                if !ours {
                    foreign += 1;
                }
            }
            if foreign > 0 {
                return Err(format!(
                    "{branch} has {foreign} commit(s) that {up} does not have and that are not publications, and `--push` would publish them along with the game. Push or review them first (`git -C {} log {up}..HEAD`), or publish without --push. Nothing was changed.",
                    repo.display()
                ));
            }
        }
    }
    // ---- the new tree, built from HEAD in a scratch area ----------------------------------------------------------------------------------------------
    let scratch = Scratch::new(&git_dir)?;
    let (index, work) = (scratch.0.join("index"), scratch.0.join("work"));
    std::fs::create_dir_all(work.join(scope.subtree)).map_err(|e| format!("{}: {e}", work.display()))?;
    if git.ok(&["cat-file", "-e", &format!("{parent}:{}", scope.subtree)]) {
        git.run_bytes(git.scratch(&["read-tree", &format!("--prefix={}/", scope.subtree), &format!("{parent}:{}", scope.subtree)], &index, &work), None)?;
        git.run_bytes(git.scratch(&["-c", "core.autocrlf=false", "checkout-index", "-a", "-f"], &index, &work), None)?;
        std::fs::remove_file(&index).ok();
    }
    edit(&work.join(scope.subtree))?;
    git.run_bytes(git.scratch(&["read-tree", &parent], &index, &work), None)?;
    git.run_bytes(git.scratch(&["-c", "core.autocrlf=false", "add", "-A", "-f", "--", scope.subtree], &index, &work), None)?;
    let new_tree = String::from_utf8_lossy(&git.run_bytes(git.scratch(&["write-tree"], &index, &work), None)?).trim().to_string();
    let changes = parse_diff_tree(&git.run_bytes(git.cmd(&["diff-tree", "-r", "-z", "--no-renames", &parent, &new_tree]), None)?);
    if changes.is_empty() {
        return Ok(Committed { commit: None, parent, changed: vec![], pushed: false });
    }
    let paths: Vec<String> = changes.iter().map(|c| c.path.clone()).collect();
    // ---- what the checkout holds at exactly the paths we are about to change ---------------------------------------------------------------------------
    let conflicts = find_conflicts(&git, repo, &changes)?;
    if !conflicts.hard.is_empty() {
        let list = conflicts.hard.iter().map(|(p, why)| format!("  {p}  ({why})")).collect::<Vec<_>>().join("\n");
        let r = repo.display();
        return Err(format!(
            "publishing would overwrite work in the checkout; nothing was changed. These files belong to this game's publication but hold other content:\n{list}\n\
             Recover by keeping or removing them yourself, then publish again:\n  \
             keep them: git -C {r} stash push --include-untracked -- {}\n  \
             discard them (they are generated): git -C {r} restore --staged --worktree -- {} ; git -C {r} clean -fd -- {}",
            conflicts.dirs.join(" "),
            conflicts.dirs.join(" "),
            conflicts.dirs.join(" "),
        ));
    }
    // Leftovers that are byte-for-byte what we are about to write (a publication that stopped halfway) are the one thing we clear ourselves: nothing is lost by it.
    for (path, head_blob) in &conflicts.identical_to_new {
        let file = repo.join(path);
        match head_blob {
            None => {
                let _ = std::fs::remove_file(&file);
            }
            Some(blob) => {
                let content = git.run_bytes(git.cmd(&["cat-file", "blob", blob]), None)?;
                std::fs::write(&file, content).map_err(|e| format!("{}: {e}", file.display()))?;
            }
        }
    }
    // ---- the commit, and the move of HEAD --------------------------------------------------------------------------------------------------------------
    let mut ci = git.cmd(&["commit-tree", &new_tree, "-p", &parent, "-m", scope.message]);
    for (k, v) in [("NAME", "RedEngine publish"), ("EMAIL", PUBLISH_EMAIL)] {
        ci.env(format!("GIT_AUTHOR_{k}"), v).env(format!("GIT_COMMITTER_{k}"), v);
    }
    let commit = String::from_utf8_lossy(&git.run_bytes(ci, None)?).trim().to_string();
    // The checkout follows the commit the way a branch switch does: only the changed paths, and git refuses without changing anything if one of them is not clean.
    git.run_bytes(git.cmd(&["read-tree", "-m", "-u", &parent, &commit]), None).map_err(|e| {
        format!(
            "{e}\nThe checkout could not take the published files (another git command may be running, or `.git/index.lock` is left over: remove it only if no git process is running). \
             Nothing was committed and nothing was changed; publish again."
        )
    })?;
    let undo = |why: &str| -> String {
        let moved_back = git.ok(&["update-ref", "-m", "publish: undone", "HEAD", &parent, &commit]);
        let synced_back = git.ok(&["read-tree", "-m", "-u", &commit, &parent]);
        if moved_back && synced_back {
            format!("{why}\nThe publication was taken back out: the repository is as it was before.")
        } else {
            format!(
                "{why}\nCould not fully take the publication back out. To return to the previous state: git -C {r} update-ref HEAD {parent} && git -C {r} read-tree -m -u {commit} {parent}",
                r = repo.display()
            )
        }
    };
    if !git.ok(&["update-ref", "-m", &format!("publish: {}", scope.message.lines().next().unwrap_or("")), "HEAD", &commit, &parent]) {
        // Somebody committed between our read and our write: HEAD is not ours to move. Put the checkout back.
        let synced_back = git.ok(&["read-tree", "-m", "-u", &commit, &parent]);
        let rest = if synced_back {
            String::new()
        } else {
            format!("; the published files are still in the working tree: git -C {r} read-tree -m -u {commit} {parent}", r = repo.display())
        };
        return Err(format!("{branch} moved while publishing (someone committed): nothing was committed{rest}. Publish again."));
    }
    let mut pushed = false;
    if scope.push {
        if let Err(e) = git.run_bytes(git.cmd(&["push", "origin", "HEAD"]), None) {
            return Err(undo(&format!(
                "{e}\nThe push failed, so nothing was published. If origin has newer commits: `git -C {r} pull --rebase origin {branch}`, then publish again.",
                r = repo.display()
            )));
        }
        pushed = true;
    }
    drop(scratch);
    Ok(Committed { commit: Some(commit), parent, changed: paths, pushed })
}

struct Conflicts {
    /// `(path, why)`: the checkout holds something at this path that the commit would replace.
    hard: Vec<(String, String)>,
    /// `(path, blob at HEAD)`: the working file is exactly the new content, left by a publication that did not finish.
    identical_to_new: Vec<(String, Option<String>)>,
    /// The directories to name in recovery commands.
    dirs: Vec<String>,
}

fn find_conflicts(git: &Git, repo: &Path, changes: &[Change]) -> Result<Conflicts, String> {
    let paths: Vec<String> = changes.iter().map(|c| c.path.clone()).collect();
    // Index entries at those paths: `mode sha stage\tpath` (in chunks, so a big game never overruns the command line).
    let mut index: BTreeMap<String, (String, String)> = BTreeMap::new();
    for chunk in paths.chunks(100) {
        let mut args: Vec<String> = ["ls-files", "-s", "-z", "--"].iter().map(|s| s.to_string()).collect();
        args.extend(chunk.iter().map(|p| format!(":(literal){p}")));
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        let listed = git.run_bytes(git.cmd(&args), None)?;
        for rec in String::from_utf8_lossy(&listed).split('\0').filter(|s| !s.is_empty()) {
            if let Some((meta, path)) = rec.split_once('\t') {
                let f: Vec<&str> = meta.split_whitespace().collect();
                if f.len() == 3 {
                    index.insert(path.to_string(), (f[1].to_string(), f[2].to_string()));
                }
            }
        }
    }
    // What the working files would hash to as blobs (the same filters `git add` applies).
    let present: Vec<&String> = paths.iter().filter(|p| std::fs::symlink_metadata(repo.join(p)).is_ok()).collect();
    let mut hashed: BTreeMap<String, String> = BTreeMap::new();
    if !present.is_empty() {
        let stdin: String = present.iter().map(|p| format!("{p}\n")).collect();
        let out = git.run_bytes(git.cmd(&["hash-object", "--stdin-paths"]), Some(stdin.as_bytes()))?;
        for (p, h) in present.iter().zip(String::from_utf8_lossy(&out).lines()) {
            hashed.insert((*p).clone(), h.to_string());
        }
    }
    let mut c = Conflicts { hard: vec![], identical_to_new: vec![], dirs: vec![] };
    for ch in changes {
        let idx = index.get(&ch.path);
        let work = hashed.get(&ch.path);
        if idx.is_some_and(|(_, stage)| stage != "0") {
            c.hard.push((ch.path.clone(), "unmerged in the index".into()));
            continue;
        }
        if idx.map(|(sha, _)| sha) != ch.old.as_ref() {
            c.hard.push((ch.path.clone(), if idx.is_some() { "staged with other content".into() } else { "staged for deletion".into() }));
            continue;
        }
        match (work, &ch.old) {
            (w, old) if w == old.as_ref() => {}
            (Some(w), _) if ch.new.as_ref() == Some(w) => c.identical_to_new.push((ch.path.clone(), ch.old.clone())),
            (Some(_), Some(_)) => c.hard.push((ch.path.clone(), "modified in the working tree".into())),
            (Some(_), None) => c.hard.push((ch.path.clone(), "an untracked file with other content is in the way".into())),
            (None, Some(_)) => c.hard.push((ch.path.clone(), "deleted in the working tree".into())),
            (None, None) => {}
        }
    }
    // The directories those paths live in, two levels down at most (`projects/coin-dash/maps`, or the file itself at the top).
    let mut dirs: Vec<String> = c
        .hard
        .iter()
        .map(|(p, _)| {
            let parts: Vec<&str> = p.split('/').collect();
            if parts.len() > 3 {
                parts[..3].join("/")
            } else {
                p.clone()
            }
        })
        .collect();
    dirs.sort();
    dirs.dedup();
    c.dirs = dirs;
    Ok(c)
}
