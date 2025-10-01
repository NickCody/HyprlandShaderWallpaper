use std::env;
use std::path::PathBuf;

use anyhow::{anyhow, Context, Result};
use directories_next::BaseDirs;

#[derive(Debug, Clone)]
pub struct PathResolver {
    cwd: PathBuf,
}

impl PathResolver {
    pub fn new() -> Result<Self> {
        let cwd = env::current_dir().context("failed to resolve current working directory")?;
        Ok(Self { cwd })
    }

    pub fn with_cwd<P: Into<PathBuf>>(cwd: P) -> Self {
        Self { cwd: cwd.into() }
    }

    pub fn expand_path(&self, input: &str) -> Result<PathBuf> {
        let expanded_env = expand_env_vars(input)?;
        let expanded = expand_home(&expanded_env)?;
        Ok(PathBuf::from(expanded))
    }

    pub fn normalize_local_path(&self, input: &str) -> Result<PathBuf> {
        let expanded = self.expand_path(input)?;
        if expanded.is_absolute() {
            return Ok(expanded);
        }

        if expanded.as_os_str().is_empty() {
            anyhow::bail!("local path must not be empty");
        }

        let candidate = self.cwd.join(&expanded);
        if candidate.exists() {
            return Ok(candidate);
        }

        Ok(expanded)
    }
}

fn expand_home(input: &str) -> Result<String> {
    if !input.starts_with('~') {
        return Ok(input.to_string());
    }

    let base_dirs = BaseDirs::new()
        .ok_or_else(|| anyhow!("unable to determine home directory for '~' expansion"))?;
    let home_dir = base_dirs.home_dir();

    if input == "~" {
        return Ok(home_dir.to_string_lossy().into_owned());
    }

    if let Some(rest) = input.strip_prefix("~/") {
        let mut path = home_dir.to_path_buf();
        path.push(rest);
        return Ok(path.to_string_lossy().into_owned());
    }

    anyhow::bail!("user-specific home expansion ('{input}') is not supported")
}

fn expand_env_vars(input: &str) -> Result<String> {
    let mut output = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '\\' {
            if let Some(next) = chars.next() {
                output.push(next);
            } else {
                output.push('\\');
            }
            continue;
        }

        if ch != '$' {
            output.push(ch);
            continue;
        }

        match chars.peek() {
            None => {
                output.push('$');
                break;
            }
            Some('{') => {
                chars.next();
                let mut name = String::new();
                let mut closed = false;
                while let Some(c) = chars.next() {
                    if c == '}' {
                        closed = true;
                        break;
                    }
                    name.push(c);
                }

                if !closed {
                    anyhow::bail!("missing closing '}}' in environment variable reference");
                }

                if name.is_empty() {
                    anyhow::bail!("environment variable name must not be empty");
                }

                let value = env::var(&name)
                    .map_err(|_| anyhow!("environment variable '{name}' is not set"))?;
                output.push_str(&value);
            }
            Some(&c) if is_env_name_char(c) => {
                let mut name = String::new();
                while let Some(&c) = chars.peek() {
                    if is_env_name_char(c) {
                        name.push(c);
                        chars.next();
                    } else {
                        break;
                    }
                }
                let value = env::var(&name)
                    .map_err(|_| anyhow!("environment variable '{name}' is not set"))?;
                output.push_str(&value);
            }
            _ => {
                output.push('$');
            }
        }
    }

    Ok(output)
}

fn is_env_name_char(ch: char) -> bool {
    ch == '_' || ch.is_ascii_alphanumeric()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expands_environment_variables() {
        env::set_var("SHADERTOY_PATH_TEST", "value");
        let resolver = PathResolver::with_cwd("/");
        let path = resolver.expand_path("/tmp/$SHADERTOY_PATH_TEST").unwrap();
        assert_eq!(path, PathBuf::from("/tmp/value"));
        env::remove_var("SHADERTOY_PATH_TEST");
    }

    #[test]
    fn expand_env_fails_on_missing_variable() {
        let resolver = PathResolver::with_cwd("/");
        let err = resolver
            .expand_path("/tmp/$UNSET_ENV_FOR_TEST")
            .unwrap_err();
        assert!(err.to_string().contains("UNSET_ENV_FOR_TEST"));
    }

    #[test]
    fn expands_home_prefix() {
        let resolver = PathResolver::with_cwd("/");
        let expanded = resolver.expand_path("~").unwrap();
        assert!(expanded.is_absolute());
    }

    #[test]
    fn normalize_local_path_prefers_cwd_when_exists() {
        let temp = tempfile::tempdir().unwrap();
        let file_path = temp.path().join("pack");
        std::fs::create_dir_all(&file_path).unwrap();

        let resolver = PathResolver::with_cwd(temp.path());
        let normalized = resolver.normalize_local_path("pack").unwrap();
        assert_eq!(normalized, file_path);
    }
}
