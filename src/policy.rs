use serde::Deserialize;
use std::collections::HashMap;

const DEFAULT_PATH: &str = "policies.json";

#[derive(Debug, Clone, Deserialize)]
pub struct PolicyLimit {
    pub max_amount: u64,
}

#[derive(Debug)]
pub struct Violation<'a> {
    pub action_type: &'a str,
    pub limit: u64,
    pub requested: u64,
}

#[derive(Debug, Clone, Default)]
pub struct PolicyEngine {
    limits: HashMap<Box<str>, PolicyLimit>,
}

impl PolicyEngine {
    pub fn new(limits: HashMap<Box<str>, PolicyLimit>) -> Self {
        Self { limits }
    }

    pub fn from_file(path: &str) -> Result<Self, Error> {
        let content = std::fs::read_to_string(path)?;
        let raw: HashMap<String, PolicyLimit> = serde_json::from_str(&content)?;
        let limits = raw.into_iter().map(|(k, v)| (k.into_boxed_str(), v)).collect();
        Ok(Self { limits })
    }

    pub fn from_default_file() -> Self {
        Self::from_file(DEFAULT_PATH).unwrap_or_default()
    }

    #[inline]
    pub fn check<'a>(&self, action: &'a str) -> Result<(), Violation<'a>> {
        let action_type = parse_action_type(action);

        let limit = match self.limits.get(action_type) {
            Some(l) => l,
            None => return Ok(()),
        };

        let amount = match parse_amount(action) {
            Some(a) => a,
            None => return Ok(()),
        };

        if amount > limit.max_amount {
            return Err(Violation {
                action_type,
                limit: limit.max_amount,
                requested: amount,
            });
        }

        Ok(())
    }
}

#[inline]
fn parse_action_type(action: &str) -> &str {
    match action.find(':') {
        Some(i) => &action[..i],
        None => action,
    }
}

#[inline]
fn parse_amount(action: &str) -> Option<u64> {
    let mut parts = action.split(':').peekable();
    
    while let Some(part) = parts.next() {
        if part == "amount" {
            return parts.next().and_then(|v| v.parse().ok());
        }
    }
    
    None
}

#[derive(Debug)]
pub enum Error {
    Io(std::io::Error),
    Parse(serde_json::Error),
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Self::Parse(e)
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "io error: {}", e),
            Self::Parse(e) => write!(f, "parse error: {}", e),
        }
    }
}

impl std::error::Error for Error {}

#[cfg(test)]
mod tests {
    use super::*;

    fn engine(policies: &[(&str, u64)]) -> PolicyEngine {
        let limits = policies
            .iter()
            .map(|(k, v)| (Box::from(*k), PolicyLimit { max_amount: *v }))
            .collect();
        PolicyEngine::new(limits)
    }

    mod action_type {
        use super::*;

        #[test]
        fn simple() {
            assert_eq!(parse_action_type("deploy"), "deploy");
        }

        #[test]
        fn with_segments() {
            assert_eq!(parse_action_type("refund:order:123"), "refund");
        }

        #[test]
        fn empty() {
            assert_eq!(parse_action_type(""), "");
        }
    }

    mod amount {
        use super::*;

        #[test]
        fn at_end() {
            assert_eq!(parse_amount("refund:amount:50"), Some(50));
        }

        #[test]
        fn in_middle() {
            assert_eq!(parse_amount("refund:amount:50:order:1"), Some(50));
        }

        #[test]
        fn missing() {
            assert_eq!(parse_amount("refund:order:123"), None);
        }

        #[test]
        fn invalid_number() {
            assert_eq!(parse_amount("refund:amount:abc"), None);
        }

        #[test]
        fn zero() {
            assert_eq!(parse_amount("refund:amount:0"), Some(0));
        }
    }

    mod check {
        use super::*;

        #[test]
        fn under_limit_passes() {
            let e = engine(&[("refund", 50)]);
            assert!(e.check("refund:amount:49").is_ok());
            assert!(e.check("refund:amount:50").is_ok());
        }

        #[test]
        fn over_limit_fails() {
            let e = engine(&[("refund", 50)]);
            let err = e.check("refund:amount:51").unwrap_err();
            assert_eq!(err.action_type, "refund");
            assert_eq!(err.limit, 50);
            assert_eq!(err.requested, 51);
        }

        #[test]
        fn no_amount_passes() {
            let e = engine(&[("refund", 50)]);
            assert!(e.check("refund:order:123").is_ok());
        }

        #[test]
        fn unknown_action_passes() {
            let e = engine(&[("refund", 50)]);
            assert!(e.check("deploy:amount:9999").is_ok());
        }

        #[test]
        fn empty_engine_passes() {
            let e = PolicyEngine::default();
            assert!(e.check("refund:amount:9999").is_ok());
        }

        #[test]
        fn multiple_policies() {
            let e = engine(&[("refund", 50), ("compute", 200)]);
            assert!(e.check("refund:amount:50").is_ok());
            assert!(e.check("compute:amount:200").is_ok());
            assert!(e.check("refund:amount:51").is_err());
            assert!(e.check("compute:amount:201").is_err());
        }
    }
}