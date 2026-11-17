//! SigmaOS-apt-sortpkgs — Debian Packages-file stanza sorter.
//!
//! Inspired by Debian's `apt-sortpkgs`: reads a `Packages`/`Sources`-style
//! deb822 index (records separated by blank lines, `Key: value` fields,
//! continuation lines start with a space) and emits the records sorted by
//! package name then version. Sorting keeps indexes deterministic and makes
//! diffs between regenerated indexes minimal — the same reason Debian sorts
//! its shipped package lists.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use std::fmt;

/// Errors raised by this component.
#[derive(Debug, thiserror::Error)]
pub enum ComponentError {
    /// A line could not be parsed as a `Key: value` field.
    #[error("malformed line: {0}")]
    Malformed(String),
    /// A continuation line appeared before any field.
    #[error("continuation line without a field")]
    OrphanContinuation,
}

/// Result alias for component operations.
pub type ComponentResult<T> = Result<T, ComponentError>;

/// One deb822 record: an ordered map of field → value.
///
/// Field order is preserved (the Debian control format is order-sensitive for
/// readability), while lookup is by field name.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Stanza {
    fields: Vec<(String, String)>,
}

impl Stanza {
    /// Create an empty stanza.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set a field, replacing any existing value for that field.
    pub fn set(&mut self, key: &str, value: &str) {
        if let Some(slot) = self.fields.iter_mut().find(|(k, _)| k == key) {
            slot.1 = value.to_string();
        } else {
            self.fields.push((key.to_string(), value.to_string()));
        }
    }

    /// Get a field value by name.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.fields
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    /// Package name (`Package` field), if present.
    pub fn package(&self) -> Option<&str> {
        self.get("Package")
    }

    /// Version (`Version` field), if present.
    pub fn version(&self) -> Option<&str> {
        self.get("Version")
    }

    /// Iterate fields in order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.fields.iter().map(|(k, v)| (k.as_str(), v.as_str()))
    }
}

impl fmt::Display for Stanza {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (k, v) in &self.fields {
            writeln!(f, "{}: {}", k, v)?;
        }
        Ok(())
    }
}

/// Parse deb822 text into stanzas. Records are separated by blank lines;
/// a line beginning with whitespace continues the previous field's value.
pub fn parse(input: &str) -> ComponentResult<Vec<Stanza>> {
    let mut stanzas = Vec::new();
    let mut cur = Stanza::new();
    let mut last_key: Option<String> = None;

    for line in input.lines() {
        if line.trim().is_empty() {
            if !cur.fields.is_empty() {
                stanzas.push(std::mem::take(&mut cur));
            }
            last_key = None;
            continue;
        }
        if line.starts_with(' ') || line.starts_with('\t') {
            // Continuation: append to the previous field's value.
            let key = last_key.as_ref().ok_or(ComponentError::OrphanContinuation)?;
            let add = line.trim();
            if let Some(slot) = cur.fields.iter_mut().find(|(k, _)| *k == *key) {
                if !slot.1.is_empty() {
                    slot.1.push(' ');
                }
                slot.1.push_str(add);
            }
            continue;
        }
        let (key, value) = line
            .split_once(':')
            .ok_or_else(|| ComponentError::Malformed(format!("bad line: {}", line)))?;
        let key = key.trim().to_string();
        cur.set(&key, value.trim());
        last_key = Some(key);
    }
    if !cur.fields.is_empty() {
        stanzas.push(cur);
    }
    Ok(stanzas)
}

/// Sort stanzas by (Package, Version). Stanzas without a `Package` field sort
/// last; ties fall back to full field comparison for determinism.
pub fn sort_stanzas(stanzas: &mut [Stanza]) {
    stanzas.sort_by(|a, b| {
        let ka = a.package().unwrap_or("\u{10FFFF}");
        let kb = b.package().unwrap_or("\u{10FFFF}");
        ka.cmp(kb)
            .then_with(|| a.version().unwrap_or("").cmp(b.version().unwrap_or("")))
            .then_with(|| a.fields.cmp(&b.fields))
    });
}

/// Serialize stanzas back to deb822 text (blank line between records).
pub fn render(stanzas: &[Stanza]) -> String {
    let mut out = String::new();
    for (i, s) in stanzas.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        out.push_str(&s.to_string());
    }
    out
}

/// Convenience: parse → sort → render in one call.
pub fn sort_packages(input: &str) -> ComponentResult<String> {
    let mut stanzas = parse(input)?;
    sort_stanzas(&mut stanzas);
    Ok(render(&stanzas))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "Package: zlib1g\nVersion: 1.2.13\nDescription: compression\n\nPackage: bash\nVersion: 5.2\nDescription: shell\n\nPackage: apt\nVersion: 2.6\nDescription: pkg mgr\n";

    #[test]
    fn parse_extracts_fields() {
        let s = parse(SAMPLE).unwrap();
        assert_eq!(s.len(), 3);
        assert_eq!(s[0].package(), Some("zlib1g"));
        assert_eq!(s[0].get("Version"), Some("1.2.13"));
    }

    #[test]
    fn sort_orders_by_name() {
        let out = sort_packages(SAMPLE).unwrap();
        let names: Vec<&str> = out
            .lines()
            .filter(|l| l.starts_with("Package:"))
            .map(|l| l[9..].trim())
            .collect();
        assert_eq!(names, vec!["apt", "bash", "zlib1g"]);
    }

    #[test]
    fn continuation_lines_join() {
        let input = "Package: foo\nDescription: line one\n more text\nVersion: 1.0\n";
        let s = parse(input).unwrap();
        assert_eq!(s[0].get("Description"), Some("line one more text"));
    }

    #[test]
    fn render_roundtrips() {
        let s = parse(SAMPLE).unwrap();
        let text = render(&s);
        let s2 = parse(&text).unwrap();
        assert_eq!(s, s2);
    }

    #[test]
    fn missing_package_sorts_last() {
        let input = "Version: 9.9\n\nPackage: aaa\nVersion: 1.0\n";
        let out = sort_packages(input).unwrap();
        assert!(out.find("Package: aaa").unwrap() < out.find("Version: 9.9").unwrap());
    }
}
