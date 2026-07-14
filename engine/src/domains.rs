use std::collections::HashSet;
use std::path::Path;

pub const DEFAULT_TEST_DOMAINS: &[&str] = &[
    "discord.com",
    "x.com",
    "media.discordapp.net",
    "wikipedia.org",
    "soundcloud.com",
];

pub const CONTROL_DOMAIN: &str = "example.com";

#[derive(Debug, Default, Clone)]
pub struct DomainList {
    exact: HashSet<String>,
    suffix: Vec<String>,
}

impl DomainList {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.exact.is_empty() && self.suffix.is_empty()
    }

    pub fn len(&self) -> usize {
        self.exact.len() + self.suffix.len()
    }

    pub fn parse(text: &str) -> Self {
        let mut list = Self::default();

        for line in text.lines() {
            let entry = line.split('#').next().unwrap_or("").trim().to_lowercase();
            if entry.is_empty() {
                continue;
            }

            if let Some(exact) = entry.strip_prefix('=') {
                if !exact.is_empty() {
                    list.exact.insert(exact.to_string());
                }
                continue;
            }

            let base = entry
                .strip_prefix("*.")
                .or_else(|| entry.strip_prefix('.'))
                .unwrap_or(&entry);

            if !base.is_empty() {
                list.suffix.push(base.to_string());
            }
        }

        list
    }

    pub fn load(path: impl AsRef<Path>) -> std::io::Result<Self> {
        let text = std::fs::read_to_string(path)?;
        Ok(Self::parse(&text))
    }

    pub fn matches(&self, host: &str) -> bool {
        let host = host.trim_end_matches('.').to_lowercase();

        if self.exact.contains(&host) {
            return true;
        }

        self.suffix.iter().any(|base| {
            host == *base || (host.len() > base.len() && host.ends_with(&format!(".{}", base)))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_list_matches_nothing() {
        let list = DomainList::new();

        assert!(list.is_empty());
        assert!(!list.matches("discord.com"));
    }

    #[test]
    fn test_domain_and_subdomains() {
        let list = DomainList::parse("discord.com");

        assert!(list.matches("discord.com"));
        assert!(list.matches("cdn.discord.com"));
        assert!(list.matches("a.b.discord.com"));
        assert!(!list.matches("notdiscord.com"));
        assert!(!list.matches("discord.com.evil.net"));
    }

    #[test]
    fn test_exact_prefix() {
        let list = DomainList::parse("=discord.com");

        assert!(list.matches("discord.com"));
        assert!(!list.matches("cdn.discord.com"));
    }

    #[test]
    fn test_wildcard_forms() {
        let list = DomainList::parse("*.example.org\n.example.net");

        assert!(list.matches("example.org"));
        assert!(list.matches("a.example.org"));
        assert!(list.matches("example.net"));
        assert!(list.matches("a.example.net"));
    }

    #[test]
    fn test_comments_and_blanks() {
        let list = DomainList::parse("# header\n\n  discord.com  # inline\n\n# tail\n");

        assert_eq!(list.len(), 1);
        assert!(list.matches("discord.com"));
    }

    #[test]
    fn test_case_and_trailing_dot() {
        let list = DomainList::parse("Discord.COM");

        assert!(list.matches("DISCORD.com"));
        assert!(list.matches("discord.com."));
    }
}
