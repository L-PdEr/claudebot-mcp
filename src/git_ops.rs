//! Native Git Operations
//!
//! Provides Git operations using git2-rs for branch management,
//! commits, and push without shelling out to git CLI.

use git2::{
    BranchType, Commit, Cred, Error as Git2Error, FetchOptions,
    PushOptions, RemoteCallbacks, Repository, Signature, StatusOptions,
};
use std::path::Path;
use thiserror::Error;
use tracing::{debug, info, warn};

/// Git operation errors
#[derive(Error, Debug)]
pub enum GitError {
    #[error("Git error: {0}")]
    Git2(#[from] Git2Error),
    #[error("Repository not found at {0}")]
    NotFound(String),
    #[error("Not a git repository")]
    NotRepo,
    #[error("Branch not found: {0}")]
    BranchNotFound(String),
    #[error("Remote not found: {0}")]
    RemoteNotFound(String),
    #[error("Authentication failed: {0}")]
    AuthFailed(String),
    #[error("No changes to commit")]
    NothingToCommit,
    #[error("Merge conflict")]
    MergeConflict,
    #[error("Uncommitted changes")]
    UncommittedChanges,
}

/// Result type for git operations
pub type GitResult<T> = Result<T, GitError>;

/// Git repository wrapper with convenience operations
pub struct GitRepo {
    repo: Repository,
}

/// Status of a file in the working directory
#[derive(Debug, Clone)]
pub struct FileStatus {
    pub path: String,
    pub staged: bool,
    pub modified: bool,
    pub deleted: bool,
    pub new: bool,
    pub conflicted: bool,
}

/// Commit information
#[derive(Debug, Clone)]
pub struct CommitInfo {
    pub hash: String,
    pub short_hash: String,
    pub message: String,
    pub author: String,
    pub timestamp: i64,
}

/// Branch information
#[derive(Debug, Clone)]
pub struct BranchInfo {
    pub name: String,
    pub is_remote: bool,
    pub is_head: bool,
    pub commit: String,
}

impl GitRepo {
    /// Open an existing repository
    pub fn open<P: AsRef<Path>>(path: P) -> GitResult<Self> {
        let path = path.as_ref();
        let repo = Repository::discover(path)
            .map_err(|_| GitError::NotFound(path.display().to_string()))?;
        Ok(Self { repo })
    }

    /// Initialize a new repository
    pub fn init<P: AsRef<Path>>(path: P) -> GitResult<Self> {
        let repo = Repository::init(path)?;
        Ok(Self { repo })
    }

    /// Clone a repository
    pub fn clone_repo(url: &str, path: &Path, ssh_key_path: Option<&Path>) -> GitResult<Self> {
        let mut callbacks = RemoteCallbacks::new();

        if let Some(key_path) = ssh_key_path {
            callbacks.credentials(move |_url, username_from_url, _allowed_types| {
                let username = username_from_url.unwrap_or("git");
                Cred::ssh_key(username, None, key_path, None)
            });
        }

        let mut fetch_opts = FetchOptions::new();
        fetch_opts.remote_callbacks(callbacks);

        let mut builder = git2::build::RepoBuilder::new();
        builder.fetch_options(fetch_opts);

        let repo = builder.clone(url, path)?;
        Ok(Self { repo })
    }

    /// Get the repository path
    pub fn path(&self) -> &Path {
        self.repo.path()
    }

    /// Get the working directory path
    pub fn workdir(&self) -> Option<&Path> {
        self.repo.workdir()
    }

    // ========== Status Operations ==========

    /// Get repository status
    pub fn status(&self) -> GitResult<Vec<FileStatus>> {
        let mut opts = StatusOptions::new();
        opts.include_untracked(true)
            .recurse_untracked_dirs(true)
            .include_ignored(false);

        let statuses = self.repo.statuses(Some(&mut opts))?;
        let mut files = Vec::new();

        for entry in statuses.iter() {
            let status = entry.status();
            if let Some(path) = entry.path() {
                files.push(FileStatus {
                    path: path.to_string(),
                    staged: status.is_index_new() || status.is_index_modified() || status.is_index_deleted(),
                    modified: status.is_wt_modified() || status.is_index_modified(),
                    deleted: status.is_wt_deleted() || status.is_index_deleted(),
                    new: status.is_wt_new() || status.is_index_new(),
                    conflicted: status.is_conflicted(),
                });
            }
        }

        Ok(files)
    }

    /// Check if working directory is clean
    pub fn is_clean(&self) -> GitResult<bool> {
        let status = self.status()?;
        Ok(status.is_empty())
    }

    /// Check for uncommitted changes
    pub fn has_changes(&self) -> GitResult<bool> {
        Ok(!self.is_clean()?)
    }

    // ========== Branch Operations ==========

    /// Get current branch name
    pub fn current_branch(&self) -> GitResult<String> {
        let head = self.repo.head()?;
        if let Some(name) = head.shorthand() {
            Ok(name.to_string())
        } else {
            Ok("HEAD".to_string())
        }
    }

    /// List all branches
    pub fn list_branches(&self, remote: bool) -> GitResult<Vec<BranchInfo>> {
        let branch_type = if remote { BranchType::Remote } else { BranchType::Local };
        let branches = self.repo.branches(Some(branch_type))?;

        let head = self.repo.head().ok();
        let head_name = head.as_ref().and_then(|h| h.shorthand()).unwrap_or("");

        let mut result = Vec::new();
        for branch in branches {
            let (branch, _) = branch?;
            if let Some(name) = branch.name()? {
                let commit = branch.get()
                    .peel_to_commit()
                    .map(|c| c.id().to_string()[..7].to_string())
                    .unwrap_or_default();

                result.push(BranchInfo {
                    name: name.to_string(),
                    is_remote: remote,
                    is_head: name == head_name,
                    commit,
                });
            }
        }

        Ok(result)
    }

    /// Create a new branch
    pub fn create_branch(&self, name: &str, from: Option<&str>) -> GitResult<()> {
        let target = if let Some(ref_name) = from {
            self.repo.revparse_single(ref_name)?
                .peel_to_commit()?
        } else {
            self.repo.head()?.peel_to_commit()?
        };

        self.repo.branch(name, &target, false)?;
        info!("Created branch: {}", name);
        Ok(())
    }

    /// Switch to a branch
    pub fn checkout_branch(&self, name: &str) -> GitResult<()> {
        // Find the branch
        let branch = self.repo.find_branch(name, BranchType::Local)?;
        let reference = branch.into_reference();

        // Set HEAD to the branch
        self.repo.set_head(reference.name().unwrap())?;

        // Checkout the working directory
        self.repo.checkout_head(Some(
            git2::build::CheckoutBuilder::new()
                .force()
        ))?;

        info!("Switched to branch: {}", name);
        Ok(())
    }

    /// Delete a branch
    pub fn delete_branch(&self, name: &str) -> GitResult<()> {
        let mut branch = self.repo.find_branch(name, BranchType::Local)?;
        branch.delete()?;
        info!("Deleted branch: {}", name);
        Ok(())
    }

    // ========== Staging Operations ==========

    /// Stage a file
    pub fn add(&self, path: &str) -> GitResult<()> {
        let mut index = self.repo.index()?;
        index.add_path(Path::new(path))?;
        index.write()?;
        debug!("Staged: {}", path);
        Ok(())
    }

    /// Stage multiple files
    pub fn add_all(&self, paths: &[&str]) -> GitResult<()> {
        let mut index = self.repo.index()?;
        for path in paths {
            index.add_path(Path::new(path))?;
        }
        index.write()?;
        debug!("Staged {} files", paths.len());
        Ok(())
    }

    /// Stage all changes
    pub fn add_all_changes(&self) -> GitResult<()> {
        let mut index = self.repo.index()?;
        index.add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)?;
        index.write()?;
        debug!("Staged all changes");
        Ok(())
    }

    /// Unstage a file
    pub fn reset_file(&self, path: &str) -> GitResult<()> {
        let head = self.repo.head()?.peel_to_commit()?;
        self.repo.reset_default(Some(&head.into_object()), [path].iter())?;
        debug!("Unstaged: {}", path);
        Ok(())
    }

    // ========== Commit Operations ==========

    /// Create a commit
    pub fn commit(&self, message: &str, author_name: &str, author_email: &str) -> GitResult<CommitInfo> {
        let mut index = self.repo.index()?;

        // Check if there's anything to commit
        if index.is_empty() {
            return Err(GitError::NothingToCommit);
        }

        let tree_id = index.write_tree()?;
        let tree = self.repo.find_tree(tree_id)?;

        let signature = Signature::now(author_name, author_email)?;

        // Get parent commit (if any)
        let parent = self.repo.head()
            .and_then(|h| h.peel_to_commit())
            .ok();

        let parents: Vec<&Commit> = parent.iter().collect();

        let oid = self.repo.commit(
            Some("HEAD"),
            &signature,
            &signature,
            message,
            &tree,
            &parents,
        )?;

        let commit = self.repo.find_commit(oid)?;
        let info = self.commit_to_info(&commit);

        info!("Created commit: {} - {}", info.short_hash, message.lines().next().unwrap_or(message));
        Ok(info)
    }

    /// Get recent commits
    pub fn log(&self, limit: usize) -> GitResult<Vec<CommitInfo>> {
        let mut revwalk = self.repo.revwalk()?;
        revwalk.push_head()?;
        revwalk.set_sorting(git2::Sort::TIME)?;

        let mut commits = Vec::new();
        for (i, oid) in revwalk.enumerate() {
            if i >= limit {
                break;
            }
            let oid = oid?;
            let commit = self.repo.find_commit(oid)?;
            commits.push(self.commit_to_info(&commit));
        }

        Ok(commits)
    }

    /// Get a specific commit
    pub fn get_commit(&self, hash: &str) -> GitResult<CommitInfo> {
        let obj = self.repo.revparse_single(hash)?;
        let commit = obj.peel_to_commit()?;
        Ok(self.commit_to_info(&commit))
    }

    fn commit_to_info(&self, commit: &Commit) -> CommitInfo {
        let hash = commit.id().to_string();
        CommitInfo {
            short_hash: hash[..7.min(hash.len())].to_string(),
            hash,
            message: commit.message().unwrap_or("").to_string(),
            author: format!("{} <{}>",
                commit.author().name().unwrap_or(""),
                commit.author().email().unwrap_or("")
            ),
            timestamp: commit.time().seconds(),
        }
    }

    // ========== Remote Operations ==========

    /// Push to remote
    pub fn push(
        &self,
        remote_name: &str,
        branch: &str,
        ssh_key_path: Option<&Path>,
    ) -> GitResult<()> {
        let mut remote = self.repo.find_remote(remote_name)
            .map_err(|_| GitError::RemoteNotFound(remote_name.to_string()))?;

        let mut callbacks = RemoteCallbacks::new();

        if let Some(key_path) = ssh_key_path {
            let key_path = key_path.to_path_buf();
            callbacks.credentials(move |_url, username_from_url, _allowed_types| {
                let username = username_from_url.unwrap_or("git");
                Cred::ssh_key(username, None, &key_path, None)
            });
        } else {
            // Try SSH agent
            callbacks.credentials(|_url, username_from_url, _allowed_types| {
                let username = username_from_url.unwrap_or("git");
                Cred::ssh_key_from_agent(username)
            });
        }

        callbacks.push_update_reference(|refname, status| {
            if let Some(msg) = status {
                warn!("Push rejected for {}: {}", refname, msg);
            }
            Ok(())
        });

        let mut push_opts = PushOptions::new();
        push_opts.remote_callbacks(callbacks);

        let refspec = format!("refs/heads/{}:refs/heads/{}", branch, branch);
        remote.push(&[&refspec], Some(&mut push_opts))?;

        info!("Pushed {} to {}", branch, remote_name);
        Ok(())
    }

    /// Fetch from remote
    pub fn fetch(
        &self,
        remote_name: &str,
        ssh_key_path: Option<&Path>,
    ) -> GitResult<()> {
        let mut remote = self.repo.find_remote(remote_name)
            .map_err(|_| GitError::RemoteNotFound(remote_name.to_string()))?;

        let mut callbacks = RemoteCallbacks::new();

        if let Some(key_path) = ssh_key_path {
            let key_path = key_path.to_path_buf();
            callbacks.credentials(move |_url, username_from_url, _allowed_types| {
                let username = username_from_url.unwrap_or("git");
                Cred::ssh_key(username, None, &key_path, None)
            });
        } else {
            callbacks.credentials(|_url, username_from_url, _allowed_types| {
                let username = username_from_url.unwrap_or("git");
                Cred::ssh_key_from_agent(username)
            });
        }

        let mut fetch_opts = FetchOptions::new();
        fetch_opts.remote_callbacks(callbacks);

        remote.fetch(&[] as &[&str], Some(&mut fetch_opts), None)?;

        info!("Fetched from {}", remote_name);
        Ok(())
    }

    /// Get diff between HEAD and working directory
    pub fn diff_head(&self) -> GitResult<String> {
        let head = self.repo.head()?.peel_to_tree()?;
        let diff = self.repo.diff_tree_to_workdir_with_index(Some(&head), None)?;

        let mut diff_str = String::new();
        diff.print(git2::DiffFormat::Patch, |_delta, _hunk, line| {
            let prefix = match line.origin() {
                '+' => "+",
                '-' => "-",
                ' ' => " ",
                _ => "",
            };
            if let Ok(content) = std::str::from_utf8(line.content()) {
                diff_str.push_str(prefix);
                diff_str.push_str(content);
            }
            true
        })?;

        Ok(diff_str)
    }

    /// Check if remote is configured
    pub fn has_remote(&self, name: &str) -> bool {
        self.repo.find_remote(name).is_ok()
    }

    /// Add a remote
    pub fn add_remote(&self, name: &str, url: &str) -> GitResult<()> {
        self.repo.remote(name, url)?;
        info!("Added remote {}: {}", name, url);
        Ok(())
    }

    /// Get remote URL
    pub fn remote_url(&self, name: &str) -> GitResult<String> {
        let remote = self.repo.find_remote(name)
            .map_err(|_| GitError::RemoteNotFound(name.to_string()))?;
        Ok(remote.url().unwrap_or("").to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_init_and_commit() {
        let dir = tempdir().unwrap();
        let repo = GitRepo::init(dir.path()).unwrap();

        // Create a test file
        std::fs::write(dir.path().join("test.txt"), "Hello, World!").unwrap();

        // Stage and commit
        repo.add("test.txt").unwrap();
        let commit = repo.commit("Initial commit", "Test User", "test@example.com").unwrap();

        assert!(!commit.hash.is_empty());
        assert!(commit.message.contains("Initial commit"));
    }

    #[test]
    fn test_branch_operations() {
        let dir = tempdir().unwrap();
        let repo = GitRepo::init(dir.path()).unwrap();

        // Create initial commit
        std::fs::write(dir.path().join("test.txt"), "Content").unwrap();
        repo.add("test.txt").unwrap();
        repo.commit("Initial", "Test", "test@test.com").unwrap();

        // Create and switch branch
        repo.create_branch("feature", None).unwrap();
        repo.checkout_branch("feature").unwrap();

        assert_eq!(repo.current_branch().unwrap(), "feature");
    }

    #[test]
    fn test_status() {
        let dir = tempdir().unwrap();
        let repo = GitRepo::init(dir.path()).unwrap();

        // Initially clean (no commits yet is also clean)
        assert!(repo.status().unwrap().is_empty() || repo.is_clean().unwrap());

        // Create untracked file
        std::fs::write(dir.path().join("new.txt"), "New file").unwrap();

        let status = repo.status().unwrap();
        assert!(!status.is_empty());
        assert!(status.iter().any(|f| f.path == "new.txt" && f.new));
    }
}
