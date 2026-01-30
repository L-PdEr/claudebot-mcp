//! Pre-flight Check System
//!
//! Verifies tool availability and credentials BEFORE executing Claude Code.
//! Prevents silent failures from missing `gh`, expired tokens, etc.

use std::collections::HashMap;
use std::process::Stdio;
use tokio::process::Command;
use tracing::debug;

/// Result of pre-flight checks
#[derive(Debug, Default)]
pub struct PreflightResult {
    /// Whether execution can proceed
    pub ready: bool,
    /// Missing tools with install hints
    pub missing_tools: Vec<String>,
    /// Missing or invalid credentials
    pub missing_creds: Vec<String>,
    /// Non-blocking warnings
    pub warnings: Vec<String>,
}

impl PreflightResult {
    pub fn ok() -> Self {
        Self {
            ready: true,
            ..Default::default()
        }
    }

    /// Format as user-friendly error message
    pub fn format_error(&self) -> String {
        let mut msg = String::from("Cannot execute - missing requirements:\n\n");

        if !self.missing_tools.is_empty() {
            msg.push_str("Missing tools:\n");
            for tool in &self.missing_tools {
                msg.push_str(&format!("  - {}\n", tool));
            }
            msg.push('\n');
        }

        if !self.missing_creds.is_empty() {
            msg.push_str("Missing credentials:\n");
            for cred in &self.missing_creds {
                msg.push_str(&format!("  - {}\n", cred));
            }
        }

        msg
    }

    /// Format warnings for display
    pub fn format_warnings(&self) -> String {
        if self.warnings.is_empty() {
            return String::new();
        }
        let mut msg = String::from("Warnings:\n");
        for warn in &self.warnings {
            msg.push_str(&format!("  - {}\n", warn));
        }
        msg
    }
}

/// Tool check configuration
struct ToolCheck {
    command: &'static str,
    args: &'static [&'static str],
    install_hint: &'static str,
}

/// Credential check configuration
struct CredentialCheck {
    name: &'static str,
    env_var: Option<&'static str>,
    file_path: Option<&'static str>,
    test_command: Option<(&'static str, &'static [&'static str])>,
}

/// Pre-flight checker for tool and credential verification
pub struct PreflightChecker {
    required_tools: HashMap<String, ToolCheck>,
    credential_checks: Vec<CredentialCheck>,
}

impl Default for PreflightChecker {
    fn default() -> Self {
        Self::new()
    }
}

impl PreflightChecker {
    pub fn new() -> Self {
        let mut required_tools = HashMap::new();

        // Core tools
        required_tools.insert(
            "git".into(),
            ToolCheck {
                command: "git",
                args: &["--version"],
                install_hint: "apt install git",
            },
        );
        required_tools.insert(
            "gh".into(),
            ToolCheck {
                command: "gh",
                args: &["--version"],
                install_hint: "apt install gh  # Then: gh auth login",
            },
        );
        required_tools.insert(
            "claude".into(),
            ToolCheck {
                command: "claude",
                args: &["--version"],
                install_hint: "npm install -g @anthropic-ai/claude-code",
            },
        );
        required_tools.insert(
            "cargo".into(),
            ToolCheck {
                command: "cargo",
                args: &["--version"],
                install_hint: "curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh",
            },
        );
        required_tools.insert(
            "node".into(),
            ToolCheck {
                command: "node",
                args: &["--version"],
                install_hint: "apt install nodejs  # or use nvm",
            },
        );
        required_tools.insert(
            "npm".into(),
            ToolCheck {
                command: "npm",
                args: &["--version"],
                install_hint: "apt install npm",
            },
        );

        let credential_checks = vec![
            CredentialCheck {
                name: "GitHub CLI auth",
                env_var: None,
                file_path: None,
                test_command: Some(("gh", &["auth", "status"])),
            },
            CredentialCheck {
                name: "GitHub token",
                env_var: Some("GITHUB_TOKEN"),
                file_path: None,
                test_command: None,
            },
            CredentialCheck {
                name: "SSH key",
                env_var: None,
                file_path: Some("~/.ssh/id_ed25519"),
                test_command: None,
            },
            CredentialCheck {
                name: "Anthropic API key",
                env_var: Some("ANTHROPIC_API_KEY"),
                file_path: None,
                test_command: None,
            },
        ];

        Self {
            required_tools,
            credential_checks,
        }
    }

    /// Check all tools and credentials
    pub async fn check_all(&self) -> PreflightResult {
        let mut result = PreflightResult {
            ready: true,
            ..Default::default()
        };

        // Check all tools
        for (name, check) in &self.required_tools {
            if !self.tool_exists(check).await {
                result
                    .missing_tools
                    .push(format!("{} ({})", name, check.install_hint));
                result.ready = false;
            }
        }

        // Check credentials
        for cred in &self.credential_checks {
            match self.check_credential(cred).await {
                CredStatus::Ok => {}
                CredStatus::Missing(hint) => {
                    result
                        .missing_creds
                        .push(format!("{}: {}", cred.name, hint));
                    // Only fail for critical creds
                    if cred.name == "GitHub CLI auth" || cred.name == "Anthropic API key" {
                        result.ready = false;
                    }
                }
                CredStatus::Warning(msg) => {
                    result.warnings.push(format!("{}: {}", cred.name, msg));
                }
            }
        }

        result
    }

    /// Check only tools/creds needed for a specific command
    /// Only claude is mandatory - everything else is a warning
    pub async fn check_for_command(&self, command: &str) -> PreflightResult {
        let mut result = PreflightResult {
            ready: true,
            ..Default::default()
        };

        // Only claude CLI is mandatory
        if let Some(check) = self.required_tools.get("claude") {
            if !self.tool_exists(check).await {
                result
                    .missing_tools
                    .push(format!("claude ({})", check.install_hint));
                result.ready = false;
            }
        }

        // Other tools are optional - just warn if missing
        let needed_tools = self.detect_required_tools(command);
        debug!("Command may use tools: {:?}", needed_tools);

        for tool_name in &needed_tools {
            if tool_name == "claude" {
                continue; // Already checked above
            }
            if let Some(check) = self.required_tools.get(tool_name) {
                if !self.tool_exists(check).await {
                    result.warnings.push(
                        format!("{} not found - some operations may fail ({})", tool_name, check.install_hint)
                    );
                }
            }
        }

        result
    }

    /// Detect which tools are needed for a command
    fn detect_required_tools(&self, command: &str) -> Vec<String> {
        let mut needed = vec!["claude".to_string()];
        let cmd_lower = command.to_lowercase();

        // GitHub CLI
        if cmd_lower.contains("github")
            || cmd_lower.contains("gh ")
            || cmd_lower.contains("repo")
            || cmd_lower.contains("pull request")
            || cmd_lower.contains("pr ")
            || cmd_lower.contains("issue")
        {
            needed.push("gh".to_string());
            needed.push("git".to_string());
        }

        // Git operations
        if cmd_lower.contains("git")
            || cmd_lower.contains("commit")
            || cmd_lower.contains("push")
            || cmd_lower.contains("pull")
            || cmd_lower.contains("branch")
            || cmd_lower.contains("merge")
        {
            needed.push("git".to_string());
        }

        // Rust/Cargo
        if cmd_lower.contains("cargo")
            || cmd_lower.contains("rust")
            || cmd_lower.contains("build")
            || cmd_lower.contains("test")
            || cmd_lower.contains("clippy")
        {
            needed.push("cargo".to_string());
        }

        // Node/npm
        if cmd_lower.contains("npm")
            || cmd_lower.contains("node")
            || cmd_lower.contains("yarn")
            || cmd_lower.contains("package.json")
        {
            needed.push("node".to_string());
            needed.push("npm".to_string());
        }

        // Deduplicate
        needed.sort();
        needed.dedup();
        needed
    }

    /// Check if a tool exists and works
    async fn tool_exists(&self, check: &ToolCheck) -> bool {
        match Command::new(check.command)
            .args(check.args)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await
        {
            Ok(status) => status.success(),
            Err(_) => false,
        }
    }

    /// Check a credential
    async fn check_credential(&self, cred: &CredentialCheck) -> CredStatus {
        // Check env var first
        if let Some(var) = cred.env_var {
            if std::env::var(var).is_ok() {
                return CredStatus::Ok;
            }
        }

        // Check file existence
        if let Some(path) = cred.file_path {
            let expanded = shellexpand::tilde(path);
            if std::path::Path::new(expanded.as_ref()).exists() {
                return CredStatus::Ok;
            }
        }

        // Run test command if available
        if let Some((cmd, args)) = cred.test_command {
            return match Command::new(cmd)
                .args(args)
                .stdout(Stdio::null())
                .stderr(Stdio::piped())
                .output()
                .await
            {
                Ok(output) if output.status.success() => CredStatus::Ok,
                Ok(output) => {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    if stderr.contains("not logged in") || stderr.contains("authentication") {
                        CredStatus::Missing("not authenticated".into())
                    } else {
                        CredStatus::Warning(stderr.trim().to_string())
                    }
                }
                Err(e) => CredStatus::Missing(format!("command failed: {}", e)),
            };
        }

        CredStatus::Missing("not found".into())
    }

    /// Check GitHub CLI authentication
    async fn check_gh_auth(&self) -> Result<(), String> {
        let output = Command::new("gh")
            .args(["auth", "status"])
            .output()
            .await
            .map_err(|e| format!("failed to run gh: {}", e))?;

        if output.status.success() {
            Ok(())
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            if stderr.contains("not logged in") {
                Err("not authenticated - run 'gh auth login'".into())
            } else {
                Err(stderr.trim().to_string())
            }
        }
    }

    /// Check if SSH key exists
    async fn has_ssh_key(&self) -> bool {
        let paths = [
            "~/.ssh/id_ed25519",
            "~/.ssh/id_rsa",
            "~/.ssh/id_ecdsa",
        ];

        for path in paths {
            let expanded = shellexpand::tilde(path);
            if std::path::Path::new(expanded.as_ref()).exists() {
                return true;
            }
        }
        false
    }

    /// Quick check for claude CLI only
    pub async fn check_claude_cli(&self) -> bool {
        if let Some(check) = self.required_tools.get("claude") {
            self.tool_exists(check).await
        } else {
            false
        }
    }
}

#[derive(Debug)]
enum CredStatus {
    Ok,
    Missing(String),
    Warning(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_detect_required_tools() {
        let checker = PreflightChecker::new();

        let tools = checker.detect_required_tools("set up github repo");
        assert!(tools.contains(&"gh".to_string()));
        assert!(tools.contains(&"git".to_string()));

        let tools = checker.detect_required_tools("cargo build");
        assert!(tools.contains(&"cargo".to_string()));

        let tools = checker.detect_required_tools("simple question");
        assert!(tools.contains(&"claude".to_string()));
        assert!(!tools.contains(&"gh".to_string()));
    }

    #[tokio::test]
    async fn test_preflight_format() {
        let result = PreflightResult {
            ready: false,
            missing_tools: vec!["gh (apt install gh)".into()],
            missing_creds: vec!["GitHub CLI: not authenticated".into()],
            warnings: vec![],
        };

        let msg = result.format_error();
        assert!(msg.contains("gh"));
        assert!(msg.contains("GitHub CLI"));
    }
}
