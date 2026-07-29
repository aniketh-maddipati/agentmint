//! Git-backed checkpoint capture and worktree materialization services.
//! Used by: future checkpoint APIs and local worktree prototyping.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};

pub trait CheckpointService {
    fn capture_checkpoint(&self, repo_root: &Path) -> CheckpointResult<CheckpointCapture>;
    fn materialize_checkpoint(
        &self,
        repo_root: &Path,
        checkpoint: &CheckpointCapture,
    ) -> CheckpointResult<MaterializedCheckpoint>;
}

pub type CheckpointResult<T> = Result<T, CheckpointError>;

#[derive(Debug, thiserror::Error)]
pub enum CheckpointError {
    #[error("git command failed: {command}: {stderr}")]
    GitCommand { command: String, stderr: String },
    #[error("unsafe or non-restorable checkpoint state: {reasons:?}")]
    UnsafeState {
        reasons: Vec<CheckpointRejection>,
        snapshot: Option<Box<CheckpointCapture>>,
    },
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CheckpointCapture {
    pub checkpoint_id: String,
    pub head_commit: String,
    pub branch_state: BranchState,
    pub staged_binary_patch: String,
    pub unstaged_binary_patch: String,
    pub untracked_files: Vec<UntrackedFileMetadata>,
    pub changed_paths: Vec<String>,
    pub commands: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BranchState {
    pub branch_name: Option<String>,
    pub detached_head: bool,
    pub upstream_branch: Option<String>,
    pub ahead_count: u32,
    pub behind_count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UntrackedFileMetadata {
    pub path: String,
    pub kind: UntrackedFileKind,
    pub size_bytes: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UntrackedFileKind {
    File,
    Directory,
    Symlink,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MaterializedCheckpoint {
    pub worktree_path: PathBuf,
    pub head_commit: String,
    pub applied_commands: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "code", content = "detail", rename_all = "snake_case")]
pub enum CheckpointRejection {
    BareRepository,
    MergeInProgress,
    RebaseInProgress,
    CherryPickInProgress,
    RevertInProgress,
    BisectInProgress,
    UnresolvedConflicts(Vec<String>),
    UntrackedFilesPresent(Vec<String>),
    ExistingMaterializationPath(String),
    MissingHeadCommit,
}

pub struct GitCliCheckpointService<R = SystemGitRunner> {
    runner: R,
}

impl GitCliCheckpointService<SystemGitRunner> {
    pub fn new() -> Self {
        Self {
            runner: SystemGitRunner,
        }
    }
}

impl Default for GitCliCheckpointService<SystemGitRunner> {
    fn default() -> Self {
        Self::new()
    }
}

impl<R> GitCliCheckpointService<R> {
    pub fn with_runner(runner: R) -> Self {
        Self { runner }
    }
}

impl<R> CheckpointService for GitCliCheckpointService<R>
where
    R: GitRunner,
{
    fn capture_checkpoint(&self, repo_root: &Path) -> CheckpointResult<CheckpointCapture> {
        let head_commit = self.runner.run(repo_root, &["rev-parse", "HEAD"])?;
        let branch_name = self
            .runner
            .run_optional(repo_root, &["symbolic-ref", "--quiet", "--short", "HEAD"])?;
        let detached_head = branch_name.is_none();
        let upstream_branch = self
            .runner
            .run_optional(repo_root, &["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{upstream}"])?;
        let (ahead_count, behind_count) = ahead_behind_counts(&self.runner, repo_root, upstream_branch.as_deref())?;
        let staged_binary_patch = String::from_utf8_lossy(&self.runner.run_bytes(
            repo_root,
            &["diff", "--binary", "--cached", "--no-ext-diff", "--", "."],
        )?)
        .to_string();
        let unstaged_binary_patch = String::from_utf8_lossy(&self.runner.run_bytes(
            repo_root,
            &["diff", "--binary", "--no-ext-diff", "--", "."],
        )?)
        .to_string();
        let staged_paths = nul_split(self.runner.run_bytes(
            repo_root,
            &["diff", "--name-only", "--cached", "-z", "--", "."],
        )?);
        let unstaged_paths = nul_split(self.runner.run_bytes(
            repo_root,
            &["diff", "--name-only", "-z", "--", "."],
        )?);
        let unresolved_conflicts = nul_split(self.runner.run_bytes(
            repo_root,
            &["diff", "--name-only", "--diff-filter=U", "-z", "--", "."],
        )?);
        let untracked_paths =
            nul_split(self.runner.run_bytes(repo_root, &["ls-files", "--others", "--exclude-standard", "-z"])?);
        let untracked_files = build_untracked_metadata(repo_root, &untracked_paths)?;
        let changed_paths = merged_paths(&staged_paths, &unstaged_paths, &untracked_paths);
        let branch_state = BranchState {
            branch_name,
            detached_head,
            upstream_branch,
            ahead_count,
            behind_count,
        };
        let checkpoint = CheckpointCapture {
            checkpoint_id: checkpoint_id(&head_commit),
            head_commit: head_commit.trim().to_owned(),
            branch_state,
            staged_binary_patch,
            unstaged_binary_patch,
            untracked_files,
            changed_paths,
            commands: vec![
                "git worktree add --detach {worktree_path} <head_commit>".to_owned(),
                "git apply --binary --index {staged_patch}".to_owned(),
                "git apply --binary {unstaged_patch}".to_owned(),
            ],
        };
        let reasons = capture_rejections(repo_root, &self.runner, &checkpoint, unresolved_conflicts)?;
        if reasons.is_empty() {
            return Ok(checkpoint);
        }

        Err(CheckpointError::UnsafeState {
            reasons,
            snapshot: Some(Box::new(checkpoint)),
        })
    }

    fn materialize_checkpoint(
        &self,
        repo_root: &Path,
        checkpoint: &CheckpointCapture,
    ) -> CheckpointResult<MaterializedCheckpoint> {
        let reasons = materialization_rejections(repo_root, checkpoint)?;
        if !reasons.is_empty() {
            return Err(CheckpointError::UnsafeState {
                reasons,
                snapshot: None,
            });
        }

        let worktree_path = worktree_path(repo_root, &checkpoint.checkpoint_id);
        fs::create_dir_all(
            worktree_path
                .parent()
                .ok_or_else(|| std::io::Error::other("missing worktree parent"))?,
        )?;

        let mut commands = Vec::new();
        self.runner.run_no_output(
            repo_root,
            &[
                "worktree",
                "add",
                "--detach",
                worktree_path.to_string_lossy().as_ref(),
                checkpoint.head_commit.as_str(),
            ],
        )?;
        commands.push(format!(
            "git worktree add --detach {} {}",
            worktree_path.display(),
            checkpoint.head_commit
        ));

        if !checkpoint.staged_binary_patch.is_empty() {
            self.runner.run_with_stdin(
                &worktree_path,
                &["apply", "--binary", "--index", "-"],
                checkpoint.staged_binary_patch.as_bytes(),
            )?;
            commands.push("git apply --binary --index <staged_binary_patch>".to_owned());
        }

        if !checkpoint.unstaged_binary_patch.is_empty() {
            self.runner.run_with_stdin(
                &worktree_path,
                &["apply", "--binary", "-"],
                checkpoint.unstaged_binary_patch.as_bytes(),
            )?;
            commands.push("git apply --binary <unstaged_binary_patch>".to_owned());
        }

        Ok(MaterializedCheckpoint {
            worktree_path,
            head_commit: checkpoint.head_commit.clone(),
            applied_commands: commands,
        })
    }
}

pub trait GitRunner {
    fn run(&self, repo_root: &Path, args: &[&str]) -> CheckpointResult<String>;
    fn run_bytes(&self, repo_root: &Path, args: &[&str]) -> CheckpointResult<Vec<u8>>;
    fn run_optional(&self, repo_root: &Path, args: &[&str]) -> CheckpointResult<Option<String>>;
    fn run_no_output(&self, repo_root: &Path, args: &[&str]) -> CheckpointResult<()>;
    fn run_with_stdin(
        &self,
        repo_root: &Path,
        args: &[&str],
        stdin: &[u8],
    ) -> CheckpointResult<()>;
}

pub struct SystemGitRunner;

impl GitRunner for SystemGitRunner {
    fn run(&self, repo_root: &Path, args: &[&str]) -> CheckpointResult<String> {
        let bytes = self.run_bytes(repo_root, args)?;
        Ok(String::from_utf8_lossy(&bytes).trim().to_owned())
    }

    fn run_bytes(&self, repo_root: &Path, args: &[&str]) -> CheckpointResult<Vec<u8>> {
        let output = Command::new("git").args(args).current_dir(repo_root).output()?;
        if output.status.success() {
            return Ok(output.stdout);
        }
        Err(CheckpointError::GitCommand {
            command: display_command(args),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        })
    }

    fn run_optional(&self, repo_root: &Path, args: &[&str]) -> CheckpointResult<Option<String>> {
        let output = Command::new("git").args(args).current_dir(repo_root).output()?;
        if output.status.success() {
            return Ok(Some(String::from_utf8_lossy(&output.stdout).trim().to_owned()));
        }
        if matches!(output.status.code(), Some(1) | Some(128)) {
            return Ok(None);
        }
        Err(CheckpointError::GitCommand {
            command: display_command(args),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        })
    }

    fn run_no_output(&self, repo_root: &Path, args: &[&str]) -> CheckpointResult<()> {
        let output = Command::new("git").args(args).current_dir(repo_root).output()?;
        if output.status.success() {
            return Ok(());
        }
        Err(CheckpointError::GitCommand {
            command: display_command(args),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        })
    }

    fn run_with_stdin(
        &self,
        repo_root: &Path,
        args: &[&str],
        stdin: &[u8],
    ) -> CheckpointResult<()> {
        let mut child = Command::new("git")
            .args(args)
            .current_dir(repo_root)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()?;
        if let Some(mut pipe) = child.stdin.take() {
            use std::io::Write;
            pipe.write_all(stdin)?;
        }
        let output = child.wait_with_output()?;
        if output.status.success() {
            return Ok(());
        }
        Err(CheckpointError::GitCommand {
            command: display_command(args),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        })
    }
}

fn ahead_behind_counts<R: GitRunner>(
    runner: &R,
    repo_root: &Path,
    upstream_branch: Option<&str>,
) -> CheckpointResult<(u32, u32)> {
    let Some(upstream) = upstream_branch else {
        return Ok((0, 0));
    };
    let counts = runner.run(repo_root, &["rev-list", "--left-right", "--count", &format!("HEAD...{upstream}")])?;
    let mut parts = counts.split_whitespace();
    let ahead = parts.next().and_then(|part| part.parse::<u32>().ok()).unwrap_or(0);
    let behind = parts.next().and_then(|part| part.parse::<u32>().ok()).unwrap_or(0);
    Ok((ahead, behind))
}

fn capture_rejections<R: GitRunner>(
    repo_root: &Path,
    runner: &R,
    checkpoint: &CheckpointCapture,
    unresolved_conflicts: Vec<String>,
) -> CheckpointResult<Vec<CheckpointRejection>> {
    let mut reasons = Vec::new();

    if runner.run(repo_root, &["rev-parse", "--is-bare-repository"])? == "true" {
        reasons.push(CheckpointRejection::BareRepository);
    }
    if checkpoint.head_commit.is_empty() {
        reasons.push(CheckpointRejection::MissingHeadCommit);
    }
    if git_path_exists(runner, repo_root, "MERGE_HEAD")? {
        reasons.push(CheckpointRejection::MergeInProgress);
    }
    if git_path_exists(runner, repo_root, "rebase-merge")?
        || git_path_exists(runner, repo_root, "rebase-apply")?
    {
        reasons.push(CheckpointRejection::RebaseInProgress);
    }
    if git_path_exists(runner, repo_root, "CHERRY_PICK_HEAD")? {
        reasons.push(CheckpointRejection::CherryPickInProgress);
    }
    if git_path_exists(runner, repo_root, "REVERT_HEAD")? {
        reasons.push(CheckpointRejection::RevertInProgress);
    }
    if git_path_exists(runner, repo_root, "BISECT_LOG")? {
        reasons.push(CheckpointRejection::BisectInProgress);
    }
    if !unresolved_conflicts.is_empty() {
        reasons.push(CheckpointRejection::UnresolvedConflicts(unresolved_conflicts));
    }
    if !checkpoint.untracked_files.is_empty() {
        reasons.push(CheckpointRejection::UntrackedFilesPresent(
            checkpoint
                .untracked_files
                .iter()
                .map(|item| item.path.clone())
                .collect(),
        ));
    }

    Ok(reasons)
}

fn materialization_rejections(
    repo_root: &Path,
    checkpoint: &CheckpointCapture,
) -> CheckpointResult<Vec<CheckpointRejection>> {
    let mut reasons = Vec::new();
    if checkpoint.head_commit.is_empty() {
        reasons.push(CheckpointRejection::MissingHeadCommit);
    }
    if !checkpoint.untracked_files.is_empty() {
        reasons.push(CheckpointRejection::UntrackedFilesPresent(
            checkpoint
                .untracked_files
                .iter()
                .map(|item| item.path.clone())
                .collect(),
        ));
    }
    let worktree_path = worktree_path(repo_root, &checkpoint.checkpoint_id);
    if worktree_path.exists() {
        reasons.push(CheckpointRejection::ExistingMaterializationPath(
            worktree_path.display().to_string(),
        ));
    }
    Ok(reasons)
}

fn git_path_exists<R: GitRunner>(runner: &R, repo_root: &Path, name: &str) -> CheckpointResult<bool> {
    let path = runner.run(repo_root, &["rev-parse", "--git-path", name])?;
    Ok(Path::new(&path).exists())
}

fn build_untracked_metadata(
    repo_root: &Path,
    paths: &[String],
) -> CheckpointResult<Vec<UntrackedFileMetadata>> {
    paths.iter().map(|path| untracked_metadata(repo_root, path)).collect()
}

fn untracked_metadata(repo_root: &Path, path: &str) -> CheckpointResult<UntrackedFileMetadata> {
    let full_path = repo_root.join(path);
    let metadata = fs::symlink_metadata(&full_path)?;
    let file_type = metadata.file_type();
    let kind = if file_type.is_file() {
        UntrackedFileKind::File
    } else if file_type.is_dir() {
        UntrackedFileKind::Directory
    } else if file_type.is_symlink() {
        UntrackedFileKind::Symlink
    } else {
        UntrackedFileKind::Other
    };
    let size_bytes = if file_type.is_file() {
        Some(metadata.len())
    } else {
        None
    };

    Ok(UntrackedFileMetadata {
        path: path.to_owned(),
        kind,
        size_bytes,
    })
}

fn merged_paths(staged: &[String], unstaged: &[String], untracked: &[String]) -> Vec<String> {
    let mut merged = BTreeSet::new();
    merged.extend(staged.iter().cloned());
    merged.extend(unstaged.iter().cloned());
    merged.extend(untracked.iter().cloned());
    merged.into_iter().collect()
}

fn checkpoint_id(head_commit: &str) -> String {
    let trimmed = head_commit.trim();
    let short = trimmed.chars().take(12).collect::<String>();
    format!("checkpoint-{short}")
}

fn worktree_path(repo_root: &Path, checkpoint_id: &str) -> PathBuf {
    repo_root
        .join(".agentmint")
        .join("worktrees")
        .join(checkpoint_id)
}

fn nul_split(bytes: Vec<u8>) -> Vec<String> {
    bytes.split(|byte| *byte == 0)
        .filter(|entry| !entry.is_empty())
        .map(|entry| String::from_utf8_lossy(entry).to_string())
        .collect()
}

fn display_command(args: &[&str]) -> String {
    let joined = args
        .iter()
        .map(|arg| shell_escape(arg))
        .collect::<Vec<_>>()
        .join(" ");
    format!("git {joined}")
}

fn shell_escape(arg: &str) -> String {
    if arg.chars().all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '/' | '-' | '_' | '.' | '=' | ':' | '@' | '{' | '}')) {
        return arg.to_owned();
    }
    format!("{arg:?}")
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::process::Command;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::{
        CheckpointError, CheckpointRejection, CheckpointService, GitCliCheckpointService,
        UntrackedFileKind,
    };

    #[test]
    fn capture_rejects_untracked_files_with_metadata() {
        let repo = temp_repo_path("checkpoint-untracked");
        fs::create_dir_all(&repo).expect("repo dir");
        git(&repo, &["init"]).expect("init");
        git(&repo, &["config", "user.email", "agentmint@example.com"]).expect("config email");
        git(&repo, &["config", "user.name", "AgentMint"]).expect("config name");
        fs::write(repo.join("tracked.txt"), "base\n").expect("write tracked");
        git(&repo, &["add", "tracked.txt"]).expect("add tracked");
        git(&repo, &["commit", "-m", "init"]).expect("commit");
        fs::write(repo.join("notes.bin"), [0_u8, 1, 2, 3]).expect("write untracked");

        let service = GitCliCheckpointService::new();
        let err = service.capture_checkpoint(&repo).expect_err("should reject");

        match err {
            CheckpointError::UnsafeState {
                reasons,
                snapshot: Some(snapshot),
            } => {
                assert!(reasons.contains(&CheckpointRejection::UntrackedFilesPresent(vec![
                    "notes.bin".to_owned()
                ])));
                assert_eq!(snapshot.untracked_files.len(), 1);
                assert_eq!(snapshot.untracked_files[0].path, "notes.bin");
                assert_eq!(snapshot.untracked_files[0].kind, UntrackedFileKind::File);
                assert_eq!(snapshot.untracked_files[0].size_bytes, Some(4));
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn captures_and_materializes_staged_and_unstaged_changes() {
        let repo = temp_repo_path("checkpoint-materialize");
        fs::create_dir_all(&repo).expect("repo dir");
        git(&repo, &["init"]).expect("init");
        git(&repo, &["config", "user.email", "agentmint@example.com"]).expect("config email");
        git(&repo, &["config", "user.name", "AgentMint"]).expect("config name");
        fs::write(repo.join("demo.txt"), "one\n").expect("write base");
        git(&repo, &["add", "demo.txt"]).expect("add");
        git(&repo, &["commit", "-m", "init"]).expect("commit");
        fs::write(repo.join("demo.txt"), "two\n").expect("write staged");
        git(&repo, &["add", "demo.txt"]).expect("restage");
        fs::write(repo.join("demo.txt"), "three\n").expect("write unstaged");

        let service = GitCliCheckpointService::new();
        let checkpoint = service.capture_checkpoint(&repo).expect("checkpoint");
        let materialized = service
            .materialize_checkpoint(&repo, &checkpoint)
            .expect("materialize");

        let materialized_contents =
            fs::read_to_string(materialized.worktree_path.join("demo.txt")).expect("read materialized");
        assert_eq!(materialized_contents, "three\n");
        assert!(checkpoint.changed_paths.contains(&"demo.txt".to_owned()));
        assert!(!checkpoint.staged_binary_patch.is_empty());
        assert!(!checkpoint.unstaged_binary_patch.is_empty());
        assert_eq!(
            git(&materialized.worktree_path, &["diff", "--cached", "--name-only"]).expect("cached diff"),
            "demo.txt"
        );
        assert_eq!(
            git(&materialized.worktree_path, &["diff", "--name-only"]).expect("unstaged diff"),
            "demo.txt"
        );
    }

    #[test]
    fn materialize_rejects_existing_agentmint_worktree_path() {
        let repo = temp_repo_path("checkpoint-existing-worktree");
        fs::create_dir_all(&repo).expect("repo dir");
        git(&repo, &["init"]).expect("init");
        git(&repo, &["config", "user.email", "agentmint@example.com"]).expect("config email");
        git(&repo, &["config", "user.name", "AgentMint"]).expect("config name");
        fs::write(repo.join("demo.txt"), "one\n").expect("write base");
        git(&repo, &["add", "demo.txt"]).expect("add");
        git(&repo, &["commit", "-m", "init"]).expect("commit");

        let service = GitCliCheckpointService::new();
        let checkpoint = service.capture_checkpoint(&repo).expect("checkpoint");
        fs::create_dir_all(
            repo.join(".agentmint")
                .join("worktrees")
                .join(&checkpoint.checkpoint_id),
        )
        .expect("precreate worktree");

        let err = service
            .materialize_checkpoint(&repo, &checkpoint)
            .expect_err("should reject existing path");

        match err {
            CheckpointError::UnsafeState { reasons, .. } => {
                assert!(reasons.iter().any(|reason| matches!(
                    reason,
                    CheckpointRejection::ExistingMaterializationPath(_)
                )));
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    fn git(repo_root: &PathBuf, args: &[&str]) -> Result<String, String> {
        let output = Command::new("git")
            .args(args)
            .current_dir(repo_root)
            .output()
            .map_err(|err| err.to_string())?;
        if output.status.success() {
            return Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned());
        }
        Err(String::from_utf8_lossy(&output.stderr).trim().to_owned())
    }

    fn temp_repo_path(label: &str) -> PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time")
            .as_nanos();
        std::env::temp_dir().join(format!("agentmint-{label}-{stamp}"))
    }
}
