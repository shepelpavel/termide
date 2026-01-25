//! SSH config parser for ~/.ssh/config
//!
//! Parses OpenSSH config files to extract host-specific settings
//! like IdentityFile, User, Port, etc.

use std::fs;
use std::path::{Path, PathBuf};

/// Settings for a specific host from SSH config.
#[derive(Debug, Clone, Default)]
pub struct SshHostConfig {
    /// Actual hostname to connect to (HostName directive).
    pub hostname: Option<String>,
    /// Username for authentication.
    pub user: Option<String>,
    /// Port number.
    pub port: Option<u16>,
    /// Identity files (private keys) to use.
    pub identity_files: Vec<PathBuf>,
    /// Whether to only use identities from agent that match IdentityFile.
    pub identities_only: bool,
}

/// Parsed SSH config with host entries.
#[derive(Debug, Default)]
pub struct SshConfig {
    /// Host-specific configurations in order of appearance.
    /// Each entry is (patterns, config).
    hosts: Vec<(Vec<String>, SshHostConfig)>,
}

impl SshConfig {
    /// Parse SSH config from the default location (~/.ssh/config).
    pub fn from_default_path() -> Option<Self> {
        let home = dirs::home_dir()?;
        let config_path = home.join(".ssh").join("config");
        Self::from_file(&config_path)
    }

    /// Parse SSH config from a specific file.
    pub fn from_file(path: &Path) -> Option<Self> {
        let content = fs::read_to_string(path).ok()?;
        Some(Self::parse(&content))
    }

    /// Parse SSH config from string content.
    pub fn parse(content: &str) -> Self {
        let mut config = SshConfig::default();
        let mut current_patterns: Vec<String> = Vec::new();
        let mut current_config = SshHostConfig::default();
        let mut in_host_block = false;

        for line in content.lines() {
            let line = line.trim();

            // Skip empty lines and comments
            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            // Parse key-value pair
            let (key, value) = match Self::parse_line(line) {
                Some(kv) => kv,
                None => continue,
            };

            if key.eq_ignore_ascii_case("Host") {
                // Save previous host block
                if in_host_block && !current_patterns.is_empty() {
                    config.hosts.push((current_patterns, current_config));
                }

                // Start new host block
                current_patterns = value.split_whitespace().map(String::from).collect();
                current_config = SshHostConfig::default();
                in_host_block = true;
            } else if key.eq_ignore_ascii_case("Match") {
                // Match blocks are more complex, skip for now
                // Save current and start fresh without pattern
                if in_host_block && !current_patterns.is_empty() {
                    config.hosts.push((current_patterns, current_config));
                }
                current_patterns = Vec::new();
                current_config = SshHostConfig::default();
                in_host_block = false;
            } else {
                // Parse option
                Self::apply_option(&mut current_config, &key, value);
            }
        }

        // Save last host block
        if in_host_block && !current_patterns.is_empty() {
            config.hosts.push((current_patterns, current_config));
        }

        config
    }

    /// Parse a single line into key-value pair.
    fn parse_line(line: &str) -> Option<(String, &str)> {
        // SSH config supports both "Key Value" and "Key=Value" formats
        let (key, value) = if let Some(eq_pos) = line.find('=') {
            let (k, v) = line.split_at(eq_pos);
            (k.trim(), v[1..].trim())
        } else {
            let mut parts = line.splitn(2, char::is_whitespace);
            let key = parts.next()?.trim();
            let value = parts.next().unwrap_or("").trim();
            (key, value)
        };

        if key.is_empty() {
            return None;
        }

        Some((key.to_string(), value))
    }

    /// Apply a config option to the host config.
    fn apply_option(config: &mut SshHostConfig, key: &str, value: &str) {
        match key.to_lowercase().as_str() {
            "hostname" => {
                config.hostname = Some(value.to_string());
            }
            "user" => {
                config.user = Some(value.to_string());
            }
            "port" => {
                if let Ok(port) = value.parse() {
                    config.port = Some(port);
                }
            }
            "identityfile" => {
                let path = Self::expand_path(value);
                config.identity_files.push(path);
            }
            "identitiesonly" => {
                config.identities_only = value.eq_ignore_ascii_case("yes");
            }
            _ => {
                // Ignore other options for now
            }
        }
    }

    /// Expand ~ in path.
    fn expand_path(path: &str) -> PathBuf {
        if let Some(rest) = path.strip_prefix("~/") {
            if let Some(home) = dirs::home_dir() {
                return home.join(rest);
            }
        }
        PathBuf::from(path)
    }

    /// Merge source config into target (only if target field is not set).
    /// Note: IdentityFile is additive in OpenSSH - each match adds to the list.
    fn merge_config(target: &mut SshHostConfig, source: &SshHostConfig) {
        if target.hostname.is_none() {
            target.hostname = source.hostname.clone();
        }
        if target.user.is_none() {
            target.user = source.user.clone();
        }
        if target.port.is_none() {
            target.port = source.port;
        }
        // IdentityFile is additive - append new keys (avoiding duplicates)
        for key in &source.identity_files {
            if !target.identity_files.contains(key) {
                target.identity_files.push(key.clone());
            }
        }
        // identities_only: take source if true
        if source.identities_only {
            target.identities_only = true;
        }
    }

    /// Get configuration for a specific host.
    ///
    /// Returns merged config from all matching Host entries.
    /// SSH config is processed in order; first match wins for each option.
    pub fn get_host_config(&self, host: &str) -> SshHostConfig {
        let mut result = SshHostConfig::default();

        // Apply matching host configs in order (first match wins for each option)
        for (patterns, config) in &self.hosts {
            if Self::matches_any_pattern(host, patterns) {
                Self::merge_config(&mut result, config);
            }
        }

        result
    }

    /// Check if host matches any of the patterns.
    fn matches_any_pattern(host: &str, patterns: &[String]) -> bool {
        patterns.iter().any(|p| Self::matches_pattern(host, p))
    }

    /// Check if host matches a single pattern.
    ///
    /// Supports * and ? wildcards.
    fn matches_pattern(host: &str, pattern: &str) -> bool {
        if pattern == "*" {
            return true;
        }

        // Simple glob matching
        let mut host_chars = host.chars().peekable();
        let mut pattern_chars = pattern.chars().peekable();

        while let Some(pc) = pattern_chars.next() {
            match pc {
                '*' => {
                    // Match zero or more characters
                    if pattern_chars.peek().is_none() {
                        return true; // * at end matches everything
                    }
                    // Try matching rest of pattern at each position
                    let rest_pattern: String = pattern_chars.collect();
                    let mut rest_host: String = host_chars.collect();
                    while !rest_host.is_empty() {
                        if Self::matches_pattern(&rest_host, &rest_pattern) {
                            return true;
                        }
                        rest_host = rest_host[1..].to_string();
                    }
                    return Self::matches_pattern("", &rest_pattern);
                }
                '?' => {
                    // Match single character
                    if host_chars.next().is_none() {
                        return false;
                    }
                }
                c => {
                    // Match literal character (case-insensitive for hostnames)
                    match host_chars.next() {
                        Some(hc) if hc.eq_ignore_ascii_case(&c) => {}
                        _ => return false,
                    }
                }
            }
        }

        host_chars.peek().is_none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_config() {
        let config = SshConfig::parse(
            r#"
Host example.com
    User admin
    Port 2222
    IdentityFile ~/.ssh/example_key

Host *.internal
    User developer
    IdentitiesOnly yes
"#,
        );

        let host_config = config.get_host_config("example.com");
        assert_eq!(host_config.user, Some("admin".to_string()));
        assert_eq!(host_config.port, Some(2222));
        assert_eq!(host_config.identity_files.len(), 1);

        let internal_config = config.get_host_config("server.internal");
        assert_eq!(internal_config.user, Some("developer".to_string()));
        assert!(internal_config.identities_only);
    }

    #[test]
    fn test_pattern_matching() {
        assert!(SshConfig::matches_pattern("example.com", "*"));
        assert!(SshConfig::matches_pattern("example.com", "example.com"));
        assert!(SshConfig::matches_pattern("example.com", "*.com"));
        assert!(SshConfig::matches_pattern("server.internal", "*.internal"));
        assert!(!SshConfig::matches_pattern("example.com", "*.org"));
        assert!(SshConfig::matches_pattern("a", "?"));
        assert!(!SshConfig::matches_pattern("ab", "?"));
    }

    #[test]
    fn test_global_config() {
        // In SSH config, order matters: first match wins for each option.
        // Specific hosts should come BEFORE wildcards (typical convention).
        let config = SshConfig::parse(
            r#"
Host special
    Port 2222

Host *
    User defaultuser
    Port 22
"#,
        );

        let default_config = config.get_host_config("random.host");
        assert_eq!(default_config.user, Some("defaultuser".to_string()));
        assert_eq!(default_config.port, Some(22));

        let special_config = config.get_host_config("special");
        assert_eq!(special_config.user, Some("defaultuser".to_string()));
        assert_eq!(special_config.port, Some(2222));
    }
}
