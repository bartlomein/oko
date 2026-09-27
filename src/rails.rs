//! Rails names a model in more ways than its constant. `belongs_to :upload`
//! and `has_many :uploads` refer to `Upload` by the rules ActiveRecord itself
//! applies (`derive_class_name`: singularize then camelize). A text scan for
//! the constant misses them, so a model's dependents list would stop at the
//! files that spell its name. (`class_name: "Upload"` spells it, so the plain
//! scan already finds it.)

use regex::Regex;
use std::sync::OnceLock;

/// One way a Ruby line can refer to a model without its constant.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AssociationRule {
    /// Shown beside the row: `belongs_to :upload`.
    pub label: &'static str,
}

/// The ActiveSupport default inflections that matter for model names, in
/// the order ActiveSupport applies them (last rule wins there; first match
/// here, so the list is reversed).
const PLURALS: &[(&str, &str)] = &[
    (r"(?i)(quiz)$", "${1}zes"),
    (r"(?i)^(oxen)$", "${1}"),
    (r"(?i)^(ox)$", "${1}en"),
    (r"(?i)(m|l)ice$", "${1}ice"),
    (r"(?i)(m|l)ouse$", "${1}ice"),
    (r"(?i)(pass|matr|vert|ind)(?:ix|ex)$", "${1}ices"),
    (r"(?i)(x|ch|ss|sh)$", "${1}es"),
    (r"(?i)([^aeiouy]|qu)y$", "${1}ies"),
    (r"(?i)(hive)$", "${1}s"),
    (r"(?i)([lr])f$", "${1}ves"),
    (r"(?i)([^f])fe$", "${1}ves"),
    (r"(?i)sis$", "ses"),
    (r"(?i)([ti])a$", "${1}a"),
    (r"(?i)([ti])um$", "${1}a"),
    (r"(?i)(buffal|tomat)o$", "${1}oes"),
    (r"(?i)(bu)s$", "${1}ses"),
    (r"(?i)(alias|status)$", "${1}es"),
    (r"(?i)(octop|vir)i$", "${1}i"),
    (r"(?i)(octop|vir)us$", "${1}i"),
    (r"(?i)^(ax|test)is$", "${1}es"),
    (r"(?i)s$", "s"),
    (r"$", "s"),
];
const SINGULARS: &[(&str, &str)] = &[
    (r"(?i)(database)s$", "${1}"),
    (r"(?i)(quiz)zes$", "${1}"),
    (r"(?i)(matr)ices$", "${1}ix"),
    (r"(?i)(vert|ind)ices$", "${1}ex"),
    (r"(?i)^(ox)en", "${1}"),
    (r"(?i)(alias|status)(es)?$", "${1}"),
    (r"(?i)(octop|vir)(us|i)$", "${1}us"),
    (r"(?i)^(a)x[ie]s$", "${1}xis"),
    (r"(?i)(cris|test)(is|es)$", "${1}is"),
    (r"(?i)(shoe)s$", "${1}"),
    (r"(?i)(o)es$", "${1}"),
    (r"(?i)(bus)(es)?$", "${1}"),
    (r"(?i)(m|l)ice$", "${1}ouse"),
    (r"(?i)(x|ch|ss|sh)es$", "${1}"),
    (r"(?i)^(m|l)ice$", "${1}ouse"),
    (r"(?i)(m)ovies$", "${1}ovie"),
    (r"(?i)(s)eries$", "${1}eries"),
    (r"(?i)([^aeiouy]|qu)ies$", "${1}y"),
    (r"(?i)([lr])ves$", "${1}f"),
    (r"(?i)(tive)s$", "${1}"),
    (r"(?i)(hive)s$", "${1}"),
    (r"(?i)([^f])ves$", "${1}fe"),
    (r"(?i)(^analy)(sis|ses)$", "${1}sis"),
    (
        r"(?i)((a)naly|(b)a|(d)iagno|(p)arenthe|(p)rogno|(s)ynop|(t)he)(sis|ses)$",
        "${1}sis",
    ),
    (r"(?i)([ti])a$", "${1}um"),
    (r"(?i)(n)ews$", "${1}ews"),
    (r"(?i)(ss)$", "${1}"),
    (r"(?i)s$", ""),
];
const IRREGULARS: &[(&str, &str)] = &[
    ("person", "people"),
    ("man", "men"),
    ("child", "children"),
    ("sex", "sexes"),
    ("move", "moves"),
    ("zombie", "zombies"),
];
const UNCOUNTABLES: &[&str] = &[
    "equipment",
    "information",
    "rice",
    "money",
    "species",
    "series",
    "fish",
    "sheep",
    "jeans",
    "police",
];

fn rules(table: &'static [(&str, &str)]) -> &'static [(Regex, &'static str)] {
    static PLURAL: OnceLock<Vec<(Regex, &'static str)>> = OnceLock::new();
    static SINGULAR: OnceLock<Vec<(Regex, &'static str)>> = OnceLock::new();
    let cell = if std::ptr::eq(table, PLURALS) {
        &PLURAL
    } else {
        &SINGULAR
    };
    cell.get_or_init(|| {
        table
            .iter()
            .map(|(pattern, replacement)| (Regex::new(pattern).unwrap(), *replacement))
            .collect()
    })
}

fn inflect(word: &str, table: &'static [(&str, &str)], irregular_from: usize) -> String {
    let lower = word.to_ascii_lowercase();
    if lower.is_empty() || UNCOUNTABLES.contains(&lower.as_str()) {
        return word.to_owned();
    }
    for pair in IRREGULARS {
        let (from, to) = if irregular_from == 0 {
            (pair.0, pair.1)
        } else {
            (pair.1, pair.0)
        };
        if let Some(prefix) = lower.strip_suffix(from)
            && prefix
                .chars()
                .next_back()
                .is_none_or(|c| !c.is_ascii_alphanumeric())
        {
            return format!("{}{to}", &word[..word.len() - from.len()]);
        }
    }
    for (pattern, replacement) in rules(table) {
        if pattern.is_match(word) {
            return pattern.replace(word, *replacement).into_owned();
        }
    }
    word.to_owned()
}

/// `upload` → `uploads`, `category` → `categories`, `person` → `people`.
pub fn pluralize(word: &str) -> String {
    inflect(word, PLURALS, 0)
}

/// `uploads` → `upload`, `categories` → `category`, `people` → `person`.
pub fn singularize(word: &str) -> String {
    inflect(word, SINGULARS, 1)
}

/// `OptimizedImage` → `optimized_image`, `S3Store` → `s3_store`.
pub fn underscore(name: &str) -> String {
    let mut out = String::with_capacity(name.len() + 4);
    let chars: Vec<char> = name.chars().collect();
    for (index, c) in chars.iter().enumerate() {
        if c.is_ascii_uppercase() {
            let previous = index.checked_sub(1).map(|i| chars[i]);
            let next = chars.get(index + 1);
            let boundary = match previous {
                None => false,
                Some('_') => false,
                Some(p) if p.is_ascii_lowercase() || p.is_ascii_digit() => true,
                Some(_) => next.is_some_and(|n| n.is_ascii_lowercase()),
            };
            if boundary {
                out.push('_');
            }
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(*c);
        }
    }
    out
}

/// `optimized_image` → `OptimizedImage`, `uploads` → `Uploads`.
pub fn camelize(word: &str) -> String {
    word.split('_')
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => first.to_ascii_uppercase().to_string() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect()
}

/// The Ruby tokens that refer to the model `name` (a constant's leaf, such
/// as `Upload` or `OptimizedImage`) without spelling it: association macros
/// with the singular and plural names. The foreign key (`upload_id`) is left
/// out: it names a column in serializers, params and jobs far more often
/// than a dependency.
pub struct Associations {
    /// The underscored name every matching line contains, for the cheap
    /// prefilter over chunk text.
    pub needle: String,
    pub patterns: Vec<(AssociationRule, Regex)>,
}

pub fn associations(name: &str) -> Option<Associations> {
    if !name.starts_with(|c: char| c.is_ascii_uppercase()) || name.len() < 3 {
        return None;
    }
    let singular = underscore(name);
    let plural = pluralize(&singular);
    let escaped_singular = regex::escape(&singular);
    let escaped_plural = regex::escape(&plural);
    let patterns = vec![
        (
            AssociationRule {
                label: "belongs_to / has_one",
            },
            Regex::new(&format!(
                r"\b(?:belongs_to|has_one)\s*\(?\s*:{escaped_singular}\b"
            ))
            .ok()?,
        ),
        (
            AssociationRule {
                label: "has_many / habtm",
            },
            Regex::new(&format!(
                r"\b(?:has_many|has_and_belongs_to_many)\s*\(?\s*:{escaped_plural}\b"
            ))
            .ok()?,
        ),
    ];
    Some(Associations {
        needle: singular,
        patterns,
    })
}

impl Associations {
    /// The first rule a line satisfies.
    pub fn matches(&self, line: &str) -> Option<AssociationRule> {
        if !line.contains(&self.needle) {
            return None;
        }
        self.patterns
            .iter()
            .find(|(_, pattern)| pattern.is_match(line))
            .map(|(rule, _)| *rule)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inflections_follow_active_support() {
        for (singular, plural) in [
            ("upload", "uploads"),
            ("category", "categories"),
            ("person", "people"),
            ("optimized_image", "optimized_images"),
            ("status", "statuses"),
            ("box", "boxes"),
            ("series", "series"),
            ("child", "children"),
            ("analysis", "analyses"),
        ] {
            assert_eq!(pluralize(singular), plural, "pluralize {singular}");
            assert_eq!(singularize(plural), singular, "singularize {plural}");
        }
        assert_eq!(underscore("OptimizedImage"), "optimized_image");
        assert_eq!(underscore("S3Store"), "s3_store");
        assert_eq!(underscore("HTMLParser"), "html_parser");
        assert_eq!(underscore("Upload"), "upload");
        assert_eq!(camelize("optimized_image"), "OptimizedImage");
        assert_eq!(camelize(&singularize("uploads")), "Upload");
    }

    #[test]
    fn association_lines_name_the_model() {
        let upload = associations("Upload").unwrap();
        for (line, label) in [
            ("  belongs_to :upload", "belongs_to / has_one"),
            (
                "has_one :upload, dependent: :destroy",
                "belongs_to / has_one",
            ),
            ("  has_many :uploads", "has_many / habtm"),
        ] {
            assert_eq!(upload.matches(line).map(|r| r.label), Some(label), "{line}");
        }
        for line in [
            "belongs_to :uploader",
            "has_many :upload_references",
            "class_name: \"UploadReference\"",
            "def uploads; end",
            "Upload.find(1)",
            "  upload_id = params[:upload_id]",
        ] {
            assert_eq!(upload.matches(line), None, "{line}");
        }
        let image = associations("OptimizedImage").unwrap();
        assert!(image.matches("has_many :optimized_images").is_some());
        assert!(image.matches("belongs_to :optimized_image").is_some());
        assert!(associations("x").is_none());
        assert!(associations("upload").is_none());
    }
}
