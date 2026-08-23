use chrono::Local;

/// Substitute `{token}` placeholders in `pattern`. Built-in tokens
/// (`{date}`, `{YYYY}`, `{MM}`, `{DD}`) resolve automatically; everything else
/// comes from `vars`. Unrecognized tokens are left literal.
///
/// This is the raw form, used for file contents where `__` sequences (e.g.
/// Python's `__init__`, `__version__`) must be preserved exactly.
pub fn interpolate(
    pattern: &str,
    vars: &std::collections::HashMap<String, String>,
    date_format: &str,
) -> String {
    let now = Local::now();
    let date_str = now.format(date_format).to_string();
    let yyyy = now.format("%Y").to_string();
    let mm = now.format("%m").to_string();
    let dd = now.format("%d").to_string();

    let mut result = pattern.to_string();

    // Built-in tokens
    result = result.replace("{date}", &date_str);
    result = result.replace("{YYYY}", &yyyy);
    result = result.replace("{MM}", &mm);
    result = result.replace("{DD}", &dd);

    // Variable tokens
    for (key, value) in vars {
        result = result.replace(&format!("{{{}}}", key), value);
    }

    result
}

/// Characters that act as separators in a name.
///
/// An empty variable leaves the separators on *both* sides of it stranded, so
/// these are the characters a collapse has to consider. Kept deliberately small:
/// a dot is not included, because collapsing one would eat file extensions.
const NAME_SEPARATORS: [char; 2] = ['_', '-'];

fn is_name_separator(c: char) -> bool {
    NAME_SEPARATORS.contains(&c)
}

/// Interpolate a *name* — identical to [`interpolate`], then tidy the separators
/// an empty variable leaves behind. Use this for folder and file *names*, never
/// for file contents.
///
/// A run of two or more separators collapses to the **last** one, and runs at
/// either end are dropped entirely. "Last wins" is what makes a mixed run come
/// out right: in `{user}_{artist}-{title}` with no artist, the `_` belonged to
/// the variable that vanished and the `-` is the one the author meant to sit
/// between the surviving parts, so `french_-Seeping` becomes `french-Seeping`.
///
/// This used to collapse only `__`, which meant a pattern separated by anything
/// other than underscores kept the orphaned separator.
///
/// Single separators are never touched, so a date like `2026-07-28` passes
/// through unchanged.
pub fn interpolate_name(
    pattern: &str,
    vars: &std::collections::HashMap<String, String>,
    date_format: &str,
) -> String {
    let raw = interpolate(pattern, vars, date_format);

    let mut out = String::with_capacity(raw.len());
    let mut pending: Option<char> = None;
    for c in raw.chars() {
        if is_name_separator(c) {
            // Remember only the most recent separator of the run.
            pending = Some(c);
        } else {
            if let Some(sep) = pending.take() {
                // A run before any real content is a leading run — drop it.
                if !out.is_empty() {
                    out.push(sep);
                }
            }
            out.push(c);
        }
    }
    // `pending` still set here means the name ended in separators; dropping it
    // trims the trailing run.
    out
}

/// Recover a numeric ID from a folder name by locating a `<prefix><digits>`
/// token (e.g. `parse_id_token("2026-04-19_Foo_ID0030", "ID")` → `Some(30)`).
///
/// Zero-padding is ignored — the value is what matters, so `ID007` and `ID0030`
/// yield `7` and `30`. Scans left-to-right and returns the first occurrence of
/// the prefix that is immediately followed by at least one ASCII digit. Returns
/// `None` when the prefix is empty or no digit-bearing token is found.
///
/// This is the **only** place folder names still influence identity — used by
/// `fastf register` to seed a metadata-less folder's ID. Discovery never calls it.
pub fn parse_id_token(name: &str, prefix: &str) -> Option<u64> {
    if prefix.is_empty() {
        return None;
    }
    let mut search_start = 0;
    while let Some(rel) = name[search_start..].find(prefix) {
        let after = search_start + rel + prefix.len();
        let digits: String = name[after..]
            .chars()
            .take_while(|c| c.is_ascii_digit())
            .collect();
        if !digits.is_empty() {
            return digits.parse::<u64>().ok();
        }
        // Prefix without trailing digits — keep scanning past this occurrence.
        search_start = after;
        if search_start >= name.len() {
            break;
        }
    }
    None
}

/// Extract the numeric value from a formatted ID string by reading its trailing
/// run of ASCII digits (e.g. `id_value("ID0030")` → `Some(30)`). Prefix-agnostic
/// so it works across templates with different ID prefixes — used to compute the
/// counter self-heal floor (`library::max_id`). Returns `None` when the string
/// has no trailing digits.
///
/// **Ids containing a hyphen are rejected outright.** A sequential id never has
/// one, but a UUID (`019fa635-876f-7f41-8831-74a0bcb20044`) and a word handle
/// (`simple-panda-fennec`) both do — and reading the trailing digits of that
/// UUID would yield `20044` and shove the counter to `ID20045`. An interim build
/// wrote ids in both of those shapes, so this guard is not hypothetical: it is
/// what lets such a project sit in a base harmlessly instead of poisoning every
/// ID minted afterwards.
pub fn id_value(id: &str) -> Option<u64> {
    if id.contains('-') {
        return None;
    }
    let bytes = id.as_bytes();
    let mut i = bytes.len();
    while i > 0 && bytes[i - 1].is_ascii_digit() {
        i -= 1;
    }
    if i == bytes.len() {
        return None;
    }
    id[i..].parse::<u64>().ok()
}

/// MS-DOS device names, still reserved by Win32 today. A file or folder called
/// any of these — with or without an extension — cannot be created, and the
/// error Windows returns says nothing useful about why.
const RESERVED_DEVICE_NAMES: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9", "CONIN$",
    "CONOUT$",
];

fn is_reserved_device_name(stem: &str) -> bool {
    RESERVED_DEVICE_NAMES
        .iter()
        .any(|reserved| stem.eq_ignore_ascii_case(reserved))
}

/// Sanitize a string for use as a folder/file name component.
///
/// Beyond the outright-illegal characters, this covers the two Windows
/// behaviours that corrupt a name silently rather than failing loudly:
///
/// - **Trailing dots and spaces** are stripped by the filesystem itself, so
///   `"Draft ."` lands on disk as `"Draft"`. fastf would then have recorded a
///   folder name that does not match reality, and every later lookup, rename or
///   move against it would miss.
/// - **Reserved device names** (`CON`, `NUL`, `COM1`, …) cannot be used at all,
///   extension or not, so an underscore is appended.
///
/// Applied on every platform, not only Windows: a project created on Linux and
/// later opened on Windows needs a name that works in both places, and a
/// template should not produce different folder names depending on where it runs.
pub fn sanitize_name(s: &str) -> String {
    let replaced: String = s
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            // Illegal in names on Windows, unreadable everywhere else.
            c if (c as u32) < 0x20 => '_',
            c => c,
        })
        .collect();

    // Doing the trim here keeps the recorded name and the name on disk
    // identical. It also reduces ".." to the empty string, which callers such as
    // `rename_project_inner` already reject.
    let trimmed = replaced.trim_end_matches([' ', '.']);

    let stem = trimmed.split('.').next().unwrap_or(trimmed);
    if is_reserved_device_name(stem) {
        // `CON` → `CON_`, `CON.txt` → `CON_.txt`.
        return format!("{}_{}", stem, &trimmed[stem.len()..]);
    }
    trimmed.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a var map from `(slug, value)` pairs.
    fn vars_of(pairs: &[(&str, &str)]) -> std::collections::HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn test_empty_token_collapses_underscores() {
        let vars = vars_of(&[("name", "Project"), ("title", ""), ("id", "001")]);
        let result = interpolate_name("{name}_{title}_{id}", &vars, "%Y-%m-%d");
        assert_eq!(result, "Project_001");
    }

    /// The reported bug: a pattern whose separators are not all underscores.
    /// `{artist}` is empty, so the `_` in front of it is orphaned and the `-`
    /// the author put between artist and title is the one that should survive.
    #[test]
    fn empty_variable_collapses_a_mixed_separator_run() {
        let vars = vars_of(&[("username", "french"), ("artist", ""), ("title", "Seeping")]);
        assert_eq!(
            interpolate_name("{date}_{username}_{artist}-{title}", &vars, "%Y-%m-%d"),
            format!("{}_french-Seeping", chrono::Local::now().format("%Y-%m-%d"))
        );
    }

    /// A date is full of single hyphens and must survive untouched — only runs
    /// of two or more separators are collapsed.
    #[test]
    fn single_separators_are_never_collapsed() {
        let vars = vars_of(&[("a", "one"), ("b", "two")]);
        assert_eq!(interpolate_name("{a}-{b}", &vars, "%Y-%m-%d"), "one-two");
        assert_eq!(interpolate_name("{a}_{b}", &vars, "%Y-%m-%d"), "one_two");
        assert_eq!(
            interpolate_name("2026-07-28_{a}", &vars, "%Y-%m-%d"),
            "2026-07-28_one"
        );
    }

    #[test]
    fn empty_variables_at_either_end_leave_no_stray_separator() {
        let vars = vars_of(&[("lead", ""), ("mid", "Body"), ("tail", "")]);
        // Leading and trailing runs are dropped whatever the separator is.
        assert_eq!(
            interpolate_name("{lead}_{mid}_{tail}", &vars, "%Y-%m-%d"),
            "Body"
        );
        assert_eq!(
            interpolate_name("{lead}-{mid}-{tail}", &vars, "%Y-%m-%d"),
            "Body"
        );
        assert_eq!(
            interpolate_name("-_{mid}_-", &vars, "%Y-%m-%d"),
            "Body",
            "a leading dash would be actively hostile in a shell"
        );
    }

    #[test]
    fn several_empty_variables_in_a_row_collapse_to_one_separator() {
        let vars = vars_of(&[("a", "One"), ("b", ""), ("c", ""), ("d", "Two")]);
        assert_eq!(
            interpolate_name("{a}_{b}_{c}-{d}", &vars, "%Y-%m-%d"),
            "One-Two"
        );
    }

    #[test]
    fn test_interpolate_preserves_double_underscores() {
        // File content must preserve `__` sequences so Python's __version__,
        // __init__ etc. don't get mangled.
        use std::collections::HashMap;
        let vars = HashMap::new();
        let result = interpolate("__version__ = \"0.1.0\"", &vars, "%Y-%m-%d");
        assert_eq!(result, "__version__ = \"0.1.0\"");
    }

    #[test]
    fn parse_id_token_ignores_padding() {
        assert_eq!(parse_id_token("2026-04-19_Foo_ID0030", "ID"), Some(30));
        assert_eq!(parse_id_token("ID007_bar", "ID"), Some(7));
        assert_eq!(parse_id_token("proj_ID0068_final", "ID"), Some(68));
    }

    #[test]
    fn parse_id_token_none_when_absent() {
        assert_eq!(parse_id_token("no_id_here", "ID"), None);
        // Prefix present but no trailing digits.
        assert_eq!(parse_id_token("this_is_IDentity", "ID"), None);
        assert_eq!(parse_id_token("anything", ""), None);
    }

    #[test]
    fn parse_id_token_skips_prefix_without_digits() {
        // First "ID" has no digits; the second one does.
        assert_eq!(parse_id_token("IDeas_then_ID0042", "ID"), Some(42));
    }

    #[test]
    fn id_value_reads_trailing_digits() {
        assert_eq!(id_value("ID0030"), Some(30));
        assert_eq!(id_value("ID007"), Some(7));
        assert_eq!(id_value("T042"), Some(42));
        assert_eq!(id_value("no-digits"), None);
        assert_eq!(id_value(""), None);
    }

    /// An interim build wrote UUID and word-handle ids. Reading the trailing
    /// digits of a UUID would put the counter floor at 20044 and every project
    /// created afterwards would be ID20045+. Such an id must contribute nothing.
    #[test]
    fn id_value_rejects_uuid_and_word_handles() {
        assert_eq!(id_value("019fa635-876f-7f41-8831-74a0bcb20044"), None);
        assert_eq!(id_value("simple-panda-fennec"), None);
        assert_eq!(id_value("compass-newt-mayfly"), None);
    }

    #[test]
    fn test_sanitize() {
        assert_eq!(sanitize_name("hello/world"), "hello_world");
        assert_eq!(sanitize_name("a:b*c"), "a_b_c");
        // Ordinary names are untouched — including interior dots.
        assert_eq!(sanitize_name("Release v1.2.3"), "Release v1.2.3");
        assert_eq!(sanitize_name("Ariana_Grande"), "Ariana_Grande");
    }

    #[test]
    fn sanitize_defuses_reserved_device_names() {
        // Windows cannot create any of these, with or without an extension.
        assert_eq!(sanitize_name("CON"), "CON_");
        assert_eq!(sanitize_name("nul"), "nul_");
        assert_eq!(sanitize_name("COM1"), "COM1_");
        assert_eq!(sanitize_name("LPT9"), "LPT9_");
        assert_eq!(sanitize_name("CON.txt"), "CON_.txt");
        assert_eq!(sanitize_name("aux.tar.gz"), "aux_.tar.gz");
        // Names that merely start with a reserved word are fine.
        assert_eq!(sanitize_name("CONTENT"), "CONTENT");
        assert_eq!(sanitize_name("COM10"), "COM10");
        assert_eq!(sanitize_name("console"), "console");
    }

    #[test]
    fn sanitize_strips_trailing_dots_and_spaces() {
        // Windows drops these silently, so the name fastf records would not
        // match the folder that actually appears on disk.
        assert_eq!(sanitize_name("Draft."), "Draft");
        assert_eq!(sanitize_name("Draft "), "Draft");
        assert_eq!(sanitize_name("Draft . . "), "Draft");
        // Leading whitespace is legal and left alone.
        assert_eq!(sanitize_name(" Draft"), " Draft");
        // Dot-only names collapse to empty; callers reject that explicitly.
        assert_eq!(sanitize_name(".."), "");
        assert_eq!(sanitize_name("..."), "");
    }

    #[test]
    fn sanitize_removes_control_characters() {
        assert_eq!(sanitize_name("bad\u{0}name"), "bad_name");
        assert_eq!(sanitize_name("tab\tname"), "tab_name");
        assert_eq!(sanitize_name("nl\nname"), "nl_name");
    }

    #[test]
    fn sanitize_is_idempotent() {
        // plan() sanitizes each variable and then the assembled folder name, so
        // applying it twice must not keep changing the result.
        for input in [
            "CON",
            "Draft.",
            "a:b*c",
            "..",
            "Release v1.2.3",
            "CON.txt",
            "tab\tname",
        ] {
            let once = sanitize_name(input);
            assert_eq!(sanitize_name(&once), once, "not idempotent for {input:?}");
        }
    }
}
