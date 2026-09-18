//! Porter stemming ported from Titus Wormer's MIT-licensed `stemmer` package.
//! See THIRD_PARTY_NOTICES.md. Input from search tokenization is ASCII.
use regex::Regex;
use std::sync::OnceLock;

struct Patterns {
    gt0: Regex,
    eq1: Regex,
    gt1: Regex,
    vowel: Regex,
    consonant_like: Regex,
}
fn patterns() -> &'static Patterns {
    static P: OnceLock<Patterns> = OnceLock::new();
    P.get_or_init(|| Patterns {
        gt0: Regex::new("^([^aeiou][^aeiouy]*)?([aeiouy][aeiou]*)([^aeiou][^aeiouy]*)").unwrap(),
        eq1: Regex::new(
            "^([^aeiou][^aeiouy]*)?([aeiouy][aeiou]*)([^aeiou][^aeiouy]*)([aeiouy][aeiou]*)?$",
        )
        .unwrap(),
        gt1: Regex::new("^([^aeiou][^aeiouy]*)?(([aeiouy][aeiou]*)([^aeiou][^aeiouy]*)){2,}")
            .unwrap(),
        vowel: Regex::new("^([^aeiou][^aeiouy]*)?[aeiouy]").unwrap(),
        consonant_like: Regex::new("^([^aeiou][^aeiouy]*)[aeiouy][^aeiouwxy]$").unwrap(),
    })
}
// A lazy nonempty prefix matches the longest available suffix first.
fn suffix<'a>(word: &'a str, rules: &[(&str, &'static str)]) -> Option<(&'a str, &'static str)> {
    rules
        .iter()
        .filter_map(|(ending, replacement)| {
            word.strip_suffix(ending)
                .filter(|s| !s.is_empty())
                .map(|s| (s, *replacement))
        })
        .min_by_key(|(s, _)| s.len())
}
pub fn stemmer(value: &str) -> String {
    let mut result = value.to_lowercase();
    if result.len() < 3 {
        return result;
    }
    let first_y = result.starts_with('y');
    if first_y {
        result.replace_range(..1, "Y");
    }
    let p = patterns();
    if (result.ends_with("sses") && result.len() > 4)
        || (result.ends_with("ies") && result.len() > 3)
    {
        result.truncate(result.len() - 2);
    } else if result.len() >= 3 && result.ends_with('s') && !result.ends_with("ss") {
        result.pop();
    }
    if let Some(prefix) = result.strip_suffix("eed").filter(|s| !s.is_empty()) {
        if p.gt0.is_match(prefix) {
            result.pop();
        }
    } else if let Some((prefix, _)) = suffix(&result, &[("ed", ""), ("ing", "")])
        && p.vowel.is_match(prefix)
    {
        result = prefix.to_owned();
        if ["at", "bl", "iz"].iter().any(|s| result.ends_with(s)) {
            result.push('e');
        } else {
            let b = result.as_bytes();
            if b.len() >= 2
                && b[b.len() - 1] == b[b.len() - 2]
                && !b"aeiouylsz".contains(&b[b.len() - 1])
            {
                result.pop();
            } else if p.consonant_like.is_match(&result) {
                result.push('e');
            }
        }
    }
    if let Some(prefix) = result.strip_suffix('y').filter(|s| !s.is_empty())
        && p.vowel.is_match(prefix)
    {
        result.pop();
        result.push('i');
    }
    if let Some((prefix, replacement)) = suffix(
        &result,
        &[
            ("ational", "ate"),
            ("tional", "tion"),
            ("enci", "ence"),
            ("anci", "ance"),
            ("izer", "ize"),
            ("bli", "ble"),
            ("alli", "al"),
            ("entli", "ent"),
            ("eli", "e"),
            ("ousli", "ous"),
            ("ization", "ize"),
            ("ation", "ate"),
            ("ator", "ate"),
            ("alism", "al"),
            ("iveness", "ive"),
            ("fulness", "ful"),
            ("ousness", "ous"),
            ("aliti", "al"),
            ("iviti", "ive"),
            ("biliti", "ble"),
            ("logi", "log"),
        ],
    ) && p.gt0.is_match(prefix)
    {
        result = format!("{prefix}{replacement}");
    }
    if let Some((prefix, replacement)) = suffix(
        &result,
        &[
            ("icate", "ic"),
            ("ative", ""),
            ("alize", "al"),
            ("iciti", "ic"),
            ("ical", "ic"),
            ("ful", ""),
            ("ness", ""),
        ],
    ) && p.gt0.is_match(prefix)
    {
        result = format!("{prefix}{replacement}");
    }
    if let Some((prefix, _)) = suffix(
        &result,
        &[
            ("al", ""),
            ("ance", ""),
            ("ence", ""),
            ("er", ""),
            ("ic", ""),
            ("able", ""),
            ("ible", ""),
            ("ant", ""),
            ("ement", ""),
            ("ment", ""),
            ("ent", ""),
            ("ou", ""),
            ("ism", ""),
            ("ate", ""),
            ("iti", ""),
            ("ous", ""),
            ("ive", ""),
            ("ize", ""),
        ],
    ) {
        if p.gt1.is_match(prefix) {
            result = prefix.to_owned();
        }
    } else if let Some(prefix) = result
        .strip_suffix("ion")
        .filter(|s| s.len() >= 2 && (s.ends_with('s') || s.ends_with('t')))
        && p.gt1.is_match(prefix)
    {
        result = prefix.to_owned();
    }
    if let Some(prefix) = result.strip_suffix('e').filter(|s| !s.is_empty())
        && (p.gt1.is_match(prefix)
            || (p.eq1.is_match(prefix) && !p.consonant_like.is_match(prefix)))
    {
        result.pop();
    }
    if result.ends_with("ll") && p.gt1.is_match(&result) {
        result.pop();
    }
    if first_y {
        result.replace_range(..1, "y");
    }
    result
}
/// Compatibility name used by diagnostic comparisons.
pub fn stem(value: &str) -> String {
    stemmer(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn porter_examples() {
        for (word, expected) in [
            ("renaming", "renam"),
            ("rename", "renam"),
            ("atomically", "atom"),
            ("atomic", "atom"),
            ("ponies", "poni"),
            ("caresses", "caress"),
            ("relational", "relat"),
            ("yelling", "yell"),
            ("filing", "file"),
            ("sky", "sky"),
            ("ties", "ti"),
            ("ied", "i"),
        ] {
            assert_eq!(stemmer(word), expected, "{word}");
        }
    }
}
