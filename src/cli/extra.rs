//! Sorting clap's trailing bucket into flags and template variables.
//!
//! `new`, `apply` and `register` accept template variables as `--slug=value`
//! flags, which clap cannot declare: the set is whatever the template says.
//! They are therefore declared with `trailing_var_arg`, and from the first
//! token clap does not recognize, every remaining token lands in a `Vec<String>`
//! instead of being parsed.
//!
//! This module empties that bucket. The list of flags it recognizes is read
//! from **clap's own declarations** for that subcommand rather than typed out
//! here: the hand-written version knew five flags, of which `register` declares
//! none, so `--rename` after the path was reported "unrecognized" and dropped
//! while the identical flag before the path worked.
//!
//! Everything that is not a declared flag and not a `--key=value` pair is an
//! error. A bare `--word` used to become a warning followed by a successful
//! create; a flag typed on the line is a request, and the only honest answers
//! are to honour it or to refuse it.

use anyhow::{Result, bail};
use std::collections::HashMap;

/// One declared flag recovered from the trailing bucket, named by its clap
/// long (without dashes) so the per-command `apply_extra` can match on it.
#[derive(Debug, PartialEq)]
pub struct Recognized {
    pub name: String,
    pub value: Option<String>,
}

/// What came out of the trailing bucket.
#[derive(Default, Debug)]
pub struct ClassifiedExtra {
    /// Declared flags, in the order they were written.
    pub recognized: Vec<Recognized>,
    /// `--key=value` pairs that are not declared flags: template variables.
    /// Keys have hyphens normalised to underscores to match the slug shape
    /// templates use on disk.
    pub vars: HashMap<String, String>,
}

/// One flag as clap declares it.
struct Declared {
    long: String,
    short: Option<char>,
    takes_value: bool,
}

fn declared_flags(cmd: &clap::Command) -> Vec<Declared> {
    cmd.get_arguments()
        .filter_map(|arg| {
            let long = arg.get_long()?.to_string();
            Some(Declared {
                long,
                short: arg.get_short(),
                takes_value: arg.get_action().takes_values(),
            })
        })
        .collect()
}

/// Classify every token clap could not parse for `cmd`.
///
/// Recognized forms per declared flag: `--flag`, `--flag=value`, `--flag value`,
/// `-s`, `-s value`. Undeclared `--key=value` is a variable. Anything else is an
/// error that names the token and, where there is one, the syntax that works.
pub fn classify_extra(extra: Vec<String>, cmd: &clap::Command) -> Result<ClassifiedExtra> {
    let declared = declared_flags(cmd);
    let mut out = ClassifiedExtra::default();
    let mut tokens = extra.into_iter().peekable();

    while let Some(token) = tokens.next() {
        // --key=value: a declared flag with a value, or a template variable.
        if let Some(body) = token.strip_prefix("--")
            && let Some((key, value)) = body.split_once('=')
        {
            match declared.iter().find(|d| d.long == key) {
                Some(flag) if flag.takes_value => out.recognized.push(Recognized {
                    name: flag.long.clone(),
                    value: Some(value.to_string()),
                }),
                Some(flag) => bail!(
                    "`--{}` does not take a value — write it as `--{}`",
                    flag.long,
                    flag.long
                ),
                None => {
                    out.vars.insert(key.replace('-', "_"), value.to_string());
                }
            }
            continue;
        }

        // --flag, optionally followed by its value.
        if let Some(key) = token.strip_prefix("--") {
            let Some(flag) = declared.iter().find(|d| d.long == key) else {
                bail!("{}", unknown_flag(&token, tokens.peek()));
            };
            out.recognized.push(Recognized {
                name: flag.long.clone(),
                value: take_value(flag, &token, &mut tokens)?,
            });
            continue;
        }

        // -s, optionally followed by its value.
        if let Some(key) = token.strip_prefix('-')
            && !token.is_empty()
            && token != "-"
        {
            let short = (key.chars().count() == 1).then(|| key.chars().next().unwrap());
            let Some(flag) = short.and_then(|c| declared.iter().find(|d| d.short == Some(c)))
            else {
                bail!("{}", unknown_flag(&token, tokens.peek()));
            };
            out.recognized.push(Recognized {
                name: flag.long.clone(),
                value: take_value(flag, &token, &mut tokens)?,
            });
            continue;
        }

        bail!(
            "unexpected argument `{}`\n  \
             Template variables are passed as `--slug=value`.",
            token
        );
    }

    Ok(out)
}

/// Consume the next token as this flag's value, when it takes one.
fn take_value(
    flag: &Declared,
    written: &str,
    tokens: &mut std::iter::Peekable<std::vec::IntoIter<String>>,
) -> Result<Option<String>> {
    if !flag.takes_value {
        return Ok(None);
    }
    match tokens.next() {
        Some(value) => Ok(Some(value)),
        None => bail!(
            "`{}` needs a value — write it as `--{}=<value>`",
            written,
            flag.long
        ),
    }
}

/// The refusal for a flag nothing declares. When a plain word follows it, the
/// user almost certainly meant a variable and wrote it in space form, so show
/// the `=` form of exactly what they typed.
fn unknown_flag(token: &str, next: Option<&String>) -> String {
    let mut message = format!("unknown flag `{token}`");
    match next {
        Some(value) if !value.starts_with('-') => {
            message.push_str(&format!(
                "\n  Template variables are passed as `{token}={value}`."
            ));
        }
        _ => message.push_str("\n  Template variables are passed as `--slug=value`."),
    }
    message
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::{Arg, ArgAction, Command};

    /// A stand-in for a subcommand: two booleans (one with a short) and one
    /// value flag, which is every shape the real ones use.
    fn cmd() -> Command {
        Command::new("t")
            .arg(
                Arg::new("yes")
                    .long("yes")
                    .short('y')
                    .action(ArgAction::SetTrue),
            )
            .arg(
                Arg::new("dry_run")
                    .long("dry-run")
                    .action(ArgAction::SetTrue),
            )
            .arg(Arg::new("base_dir").long("base-dir").action(ArgAction::Set))
    }

    fn classify(args: &[&str]) -> Result<ClassifiedExtra> {
        classify_extra(args.iter().map(|s| s.to_string()).collect(), &cmd())
    }

    #[test]
    fn declared_flags_are_recognized_in_every_form() {
        let c = classify(&["--yes", "-y", "--dry-run"]).unwrap();
        // Named by clap's long, which is what `apply_extra` matches on.
        assert_eq!(
            c.recognized
                .iter()
                .map(|r| r.name.as_str())
                .collect::<Vec<_>>(),
            ["yes", "yes", "dry-run"]
        );
        assert!(c.vars.is_empty());
    }

    #[test]
    fn a_value_flag_takes_either_form() {
        let equals = classify(&["--base-dir=/tmp/x"]).unwrap();
        let spaced = classify(&["--base-dir", "/tmp/x"]).unwrap();
        assert_eq!(equals.recognized[0].value.as_deref(), Some("/tmp/x"));
        assert_eq!(spaced.recognized[0].value.as_deref(), Some("/tmp/x"));
        assert!(equals.vars.is_empty() && spaced.vars.is_empty());
    }

    #[test]
    fn undeclared_key_value_pairs_are_variables() {
        let c = classify(&["--artist=Bad Bunny", "--client-name=Acme"]).unwrap();
        assert_eq!(c.vars.get("artist"), Some(&"Bad Bunny".to_string()));
        assert_eq!(c.vars.get("client_name"), Some(&"Acme".to_string()));
        assert!(c.recognized.is_empty());
    }

    #[test]
    fn an_empty_value_is_a_value() {
        let c = classify(&["--artist="]).unwrap();
        assert_eq!(c.vars.get("artist"), Some(&String::new()));
    }

    #[test]
    fn an_unknown_flag_is_refused_and_names_itself() {
        let err = classify(&["--bogus"]).unwrap_err().to_string();
        assert!(err.contains("--bogus"), "{err}");
    }

    /// The shape that used to fail with "no terminal to prompt on": a variable
    /// written the way an ordinary flag is written.
    #[test]
    fn a_variable_in_space_form_shows_the_equals_form() {
        let err = classify(&["--artist", "Bad Bunny"])
            .unwrap_err()
            .to_string();
        assert!(err.contains("--artist=Bad Bunny"), "{err}");
    }

    #[test]
    fn a_boolean_given_a_value_is_refused() {
        let err = classify(&["--yes=true"]).unwrap_err().to_string();
        assert!(err.contains("--yes"), "{err}");
    }

    #[test]
    fn a_value_flag_with_nothing_after_it_is_refused() {
        let err = classify(&["--base-dir"]).unwrap_err().to_string();
        assert!(err.contains("needs a value"), "{err}");
    }

    #[test]
    fn a_stray_word_is_refused() {
        let err = classify(&["oops"]).unwrap_err().to_string();
        assert!(err.contains("oops"), "{err}");
    }

    #[test]
    fn an_unknown_short_flag_is_refused() {
        let err = classify(&["-q"]).unwrap_err().to_string();
        assert!(err.contains("-q"), "{err}");
    }
}
