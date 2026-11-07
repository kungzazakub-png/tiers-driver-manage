//! Driver version numbers as Windows prints them (1.2, 24.80.0.2, 32.0.16.2114), compared like .NET's `System.Version`.
use std::cmp::Ordering;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ver([i64; 4]);

impl Ver {
    /// Same rule as the PowerShell version: cut at the first character that is not a digit or dot, then 2-4 numbers.
    pub fn parse(s: &str) -> Option<Ver> {
        let cut: String = s.chars().take_while(|c| c.is_ascii_digit() || *c == '.').collect();
        let parts: Vec<&str> = cut.split('.').collect();
        if parts.len() < 2 || parts.len() > 4 || parts.iter().any(|p| p.is_empty()) { return None; }
        let mut v = [-1i64; 4];
        for (i, p) in parts.iter().enumerate() { v[i] = p.parse().ok()?; }
        Some(Ver(v))
    }
}

impl PartialOrd for Ver {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> { Some(self.cmp(other)) }
}
impl Ord for Ver {
    fn cmp(&self, other: &Self) -> Ordering { self.0.cmp(&other.0) }
}

/// Windows reports NVIDIA drivers as 32.0.16.2114; NVIDIA calls that 621.14 (last digit of the 3rd part + the 4th part).
pub fn nvidia_version(win_ver: &str) -> Option<f64> {
    let p: Vec<&str> = win_ver.split('.').collect();
    if p.len() < 4 || p[2].is_empty() || p[3].len() != 4 { return None; }
    let s = format!("{}{}", &p[2][p[2].len() - 1..], p[3]);
    format!("{}.{}", &s[..3], &s[3..]).parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_like_dotnet() {
        assert_eq!(Ver::parse("24.80.0.2"), Some(Ver([24, 80, 0, 2])));
        assert_eq!(Ver::parse("10.1.46.3 (beta)"), Some(Ver([10, 1, 46, 3])));
        assert!(Ver::parse("7").is_none());
        assert!(Ver::parse("").is_none());
        assert!(Ver::parse("1.2.3.4.5").is_none());
        assert!(Ver::parse("1..2").is_none());
    }
    #[test]
    fn ordering_is_numeric_not_textual() {
        assert!(Ver::parse("10.0").unwrap() > Ver::parse("9.9").unwrap());
        assert!(Ver::parse("32.0.101.9175").unwrap() > Ver::parse("32.0.101.7092").unwrap());
        assert!(Ver::parse("1.0.0").unwrap() > Ver::parse("1.0").unwrap());
        assert_eq!(Ver::parse("2.0.0.0").unwrap(), Ver::parse("2.0.0.0").unwrap());
    }
    #[test]
    fn nvidia_mapping() {
        assert_eq!(nvidia_version("32.0.16.2114"), Some(621.14));
        assert_eq!(nvidia_version("32.0.15.7640"), Some(576.4));
        assert_eq!(nvidia_version("10.1"), None);
        assert_eq!(nvidia_version("32.0.16.21"), None);
    }
}
