//! Wildcard masks (`*` and `?`), as used in bans, ignores and highlights.

use crate::casemap::CaseMapping;

/// Matches `text` against `mask` under `mapping`. `*` matches any run of characters
/// (including none) and `?` matches exactly one. No other characters are special.
pub fn matches(mask: &str, text: &str, mapping: CaseMapping) -> bool {
    let mask: Vec<char> = mask.chars().map(|c| mapping.fold_for_match(c)).collect();
    let text: Vec<char> = text.chars().map(|c| mapping.fold_for_match(c)).collect();

    // Iterative matcher with single-star backtracking: linear in practice and
    // never recursive, so hostile masks cannot blow the stack.
    let (mut m, mut t) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while t < text.len() {
        match mask.get(m) {
            Some('*') => {
                star = Some((m, t));
                m += 1;
            }
            Some(&c) if c == '?' || Some(&c) == text.get(t) => {
                m += 1;
                t += 1;
            }
            _ => match star {
                Some((star_m, star_t)) => {
                    m = star_m + 1;
                    t = star_t + 1;
                    star = Some((star_m, star_t + 1));
                }
                None => return false,
            },
        }
    }
    mask.get(m..)
        .is_some_and(|rest| rest.iter().all(|&c| c == '*'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wildcards() {
        let m = CaseMapping::Rfc1459;
        assert!(matches("*", "", m));
        assert!(matches("", "", m));
        assert!(!matches("", "a", m));
        assert!(matches("a*b?c", "aXXbYc", m));
        assert!(!matches("a*b?c", "aXXbc", m));
        assert!(matches(
            "*!*@*.EXAMPLE.com",
            "nick!user@host.example.COM",
            m
        ));
        assert!(matches("nick[a]*", "NICK{A}!x@y", m));
        assert!(matches("**a**", "bab", m));
    }
}
