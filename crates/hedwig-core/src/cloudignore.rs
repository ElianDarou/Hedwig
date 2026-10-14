use ignore::gitignore::{Gitignore, GitignoreBuilder};
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuleOrigin {
    Shared,
    Local,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IgnoreDecision {
    pub ignored: bool,
    pub origin: Option<RuleOrigin>,
    pub rule: Option<String>,
}

#[derive(Debug)]
pub struct IgnoreRules {
    shared: Gitignore,
    local: Gitignore,
}

#[derive(Debug, Error)]
pub enum IgnoreError {
    #[error("ungültige Ignore-Regel: {0}")]
    Invalid(String),
}

impl IgnoreRules {
    pub fn parse(shared: &str, local: &str) -> Result<Self, IgnoreError> {
        Ok(Self {
            shared: build_rules(".hedwigignore", shared)?,
            local: build_rules("local.hedwigignore", local)?,
        })
    }

    pub fn evaluate(&self, path: &Path, is_dir: bool) -> IgnoreDecision {
        if let Some(decision) = evaluate_one(&self.local, path, is_dir, RuleOrigin::Local) {
            return decision;
        }
        evaluate_one(&self.shared, path, is_dir, RuleOrigin::Shared).unwrap_or(IgnoreDecision {
            ignored: false,
            origin: None,
            rule: None,
        })
    }
}

fn build_rules(name: &str, contents: &str) -> Result<Gitignore, IgnoreError> {
    let mut builder = GitignoreBuilder::new(PathBuf::new());
    for line in contents.lines() {
        if let Some(error) = builder.add_line(Some(PathBuf::from(name)), line).err() {
            return Err(IgnoreError::Invalid(error.to_string()));
        }
    }
    builder
        .build()
        .map_err(|error| IgnoreError::Invalid(error.to_string()))
}

fn evaluate_one(
    rules: &Gitignore,
    path: &Path,
    is_dir: bool,
    origin: RuleOrigin,
) -> Option<IgnoreDecision> {
    let matched = rules.matched_path_or_any_parents(path, is_dir);
    if matched.is_none() {
        return None;
    }
    let glob = matched.inner()?;
    Some(IgnoreDecision {
        ignored: matched.is_ignore(),
        origin: Some(origin),
        rule: Some(glob.original().to_owned()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exposes_rule_and_origin() {
        let rules = IgnoreRules::parse("node_modules/\n*.log\n!important.log", "tmp/").unwrap();
        let ignored = rules.evaluate(Path::new("src/debug.log"), false);
        assert!(ignored.ignored);
        assert_eq!(ignored.origin, Some(RuleOrigin::Shared));
        assert_eq!(ignored.rule.as_deref(), Some("*.log"));

        let exception = rules.evaluate(Path::new("important.log"), false);
        assert!(!exception.ignored);
        assert_eq!(exception.rule.as_deref(), Some("!important.log"));

        let local = rules.evaluate(Path::new("tmp/cache.bin"), false);
        assert!(local.ignored);
        assert_eq!(local.origin, Some(RuleOrigin::Local));
    }
}
