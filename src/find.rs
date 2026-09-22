//! `idk find`: print the files whose tags match every condition.

use std::ffi::OsStr;
use std::io::{IsTerminal, Write};
use std::path::Path;
use std::process::ExitCode;
use std::sync::Arc;

use clap::builder::TypedValueParser;
use clap::error::ErrorKind;
use id3::Tag;
use regex::{Regex, RegexBuilder};

use crate::cli::FindArgs;
use crate::input;
use crate::runner::{self, Order};
use crate::tag_field::{TagField, TagFieldParser};

/// A parsed `--where` condition.
#[derive(Clone, Debug, PartialEq)]
pub enum Condition {
    /// `FIELD=VALUE` (or `FIELD!=VALUE` when `negate`).
    Compare {
        /// Field to read.
        field: TagField,
        /// What to compare it with.
        operand: Operand,
        /// `!=` instead of `=`.
        negate: bool,
    },
    /// `FIELD~REGEX`; the pattern is validated when parsed.
    Matches {
        /// Field to read.
        field: TagField,
        /// Regex source, compiled once `--ignore-case` is known.
        pattern: String,
    },
}

/// The right-hand side of a comparison.
#[derive(Clone, Debug, PartialEq)]
pub enum Operand {
    /// A literal value.
    Value(String),
    /// Another field's value, written `@FIELD`.
    Field(TagField),
}

/// clap value parser for `--where` conditions.
#[derive(Clone)]
pub struct ConditionParser;

impl TypedValueParser for ConditionParser {
    type Value = Condition;

    fn parse_ref(
        &self,
        cmd: &clap::Command,
        arg: Option<&clap::Arg>,
        value: &OsStr,
    ) -> Result<Condition, clap::Error> {
        let raw = value.to_string_lossy();
        let invalid = |message: &str| {
            clap::Error::raw(
                ErrorKind::ValueValidation,
                format!("invalid condition '{raw}': {message}\n"),
            )
            .with_cmd(cmd)
        };
        let Some(at) = raw.find(['=', '~']) else {
            return Err(invalid("expected FIELD=VALUE, FIELD!=VALUE or FIELD~REGEX"));
        };
        let field_name = |name: &str| TagFieldParser.parse_ref(cmd, arg, OsStr::new(name));
        let rest = &raw[at + 1..];
        if raw[at..].starts_with('~') {
            let field = field_name(&raw[..at])?;
            if let Err(err) = Regex::new(rest) {
                return Err(invalid(&format!("bad regex: {err}")));
            }
            return Ok(Condition::Matches {
                field,
                pattern: rest.to_owned(),
            });
        }
        let (name, negate) = match raw[..at].strip_suffix('!') {
            Some(name) => (name, true),
            None => (&raw[..at], false),
        };
        let field = field_name(name)?;
        let operand = match rest.strip_prefix('@') {
            Some(other) => Operand::Field(field_name(other)?),
            None => Operand::Value(rest.to_owned()),
        };
        Ok(Condition::Compare {
            field,
            operand,
            negate,
        })
    }
}

/// A compiled check against one file's tag.
enum Test {
    Compare {
        field: TagField,
        operand: Operand,
        negate: bool,
    },
    Matches {
        field: TagField,
        regex: Regex,
    },
    Missing(TagField),
    Present(TagField),
}

/// Every condition from the command line, compiled for matching.
pub struct Matcher {
    tests: Vec<Test>,
    ignore_case: bool,
}

impl Matcher {
    /// Compiles `--where`, `--missing` and `--present` conditions.
    pub fn new(
        conditions: &[Condition],
        missing: &[TagField],
        present: &[TagField],
        ignore_case: bool,
    ) -> Self {
        let compare = conditions.iter().map(|condition| match condition {
            Condition::Compare {
                field,
                operand,
                negate,
            } => Test::Compare {
                field: field.clone(),
                operand: operand.clone(),
                negate: *negate,
            },
            Condition::Matches { field, pattern } => Test::Matches {
                field: field.clone(),
                regex: RegexBuilder::new(pattern)
                    .case_insensitive(ignore_case)
                    .build()
                    .expect("validated when parsed"),
            },
        });
        let missing = missing.iter().cloned().map(Test::Missing);
        let present = present.iter().cloned().map(Test::Present);
        Matcher {
            tests: compare.chain(missing).chain(present).collect(),
            ignore_case,
        }
    }

    /// Whether `tag` (`None` for an untagged file) satisfies every condition.
    pub fn matches(&self, tag: Option<&Tag>) -> bool {
        let value = |field: &TagField| tag.and_then(|tag| field.read(tag)).unwrap_or_default();
        self.tests.iter().all(|test| match test {
            Test::Compare {
                field,
                operand,
                negate,
            } => {
                let expected = match operand {
                    Operand::Value(expected) => expected.clone(),
                    Operand::Field(other) => value(other),
                };
                self.equal(&value(field), &expected) != *negate
            }
            Test::Matches { field, regex } => regex.is_match(&value(field)),
            Test::Missing(field) => value(field).trim().is_empty(),
            Test::Present(field) => !value(field).trim().is_empty(),
        })
    }

    fn equal(&self, left: &str, right: &str) -> bool {
        if self.ignore_case {
            left.to_lowercase() == right.to_lowercase()
        } else {
            left == right
        }
    }
}

/// Whether the file at `path` matches.
pub fn find_file(path: &Path, matcher: &Matcher) -> id3::Result<bool> {
    let tag = id3::no_tag_ok(Tag::read_from_path(path))?;
    Ok(matcher.matches(tag.as_ref()))
}

/// Runs `idk find` over every input file and returns the process exit code.
///
/// Matching paths are printed in input order. Exit code 1 means a file failed,
/// not that nothing matched.
pub async fn run(args: FindArgs) -> ExitCode {
    let matcher = Arc::new(Matcher::new(
        &args.conditions,
        &args.missing,
        &args.present,
        args.ignore_case,
    ));
    let separator: &[u8] = if args.null { b"\0" } else { b"\n" };
    let show_progress = !args.run.quiet && std::io::stdout().is_terminal();
    let inputs = input::resolve(args.input.clone()).await;
    let mut failed = inputs.failures > 0;

    runner::process(
        inputs.files,
        args.run.jobs(),
        Order::Input,
        show_progress,
        move |path| find_file(path, &matcher),
        |progress, path, result| match result {
            Ok(true) => progress.suspend(|| {
                let mut out = std::io::stdout().lock();
                // Raw bytes, so unusual file names survive the round trip to --files-from.
                let _ = out.write_all(path.as_os_str().as_encoded_bytes());
                let _ = out.write_all(separator);
            }),
            Ok(false) => {}
            Err(err) => {
                failed = true;
                progress.suspend(|| eprintln!("error: {}: {err}", path.display()));
            }
        },
    )
    .await;

    if failed {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use id3::TagLike;

    fn parse(raw: &str) -> Result<Condition, String> {
        let cmd = clap::Command::new("idk");
        ConditionParser
            .parse_ref(&cmd, None, OsStr::new(raw))
            .map_err(|err| err.to_string())
    }

    fn compare(field: TagField, operand: Operand, negate: bool) -> Condition {
        Condition::Compare {
            field,
            operand,
            negate,
        }
    }

    #[test]
    fn parses_every_operator() {
        assert_eq!(
            parse("genre=Rock"),
            Ok(compare(
                TagField::Genre,
                Operand::Value("Rock".into()),
                false
            ))
        );
        assert_eq!(
            parse("genre!="),
            Ok(compare(TagField::Genre, Operand::Value("".into()), true))
        );
        assert_eq!(
            parse("albumartist!=@artist"),
            Ok(compare(
                TagField::AlbumArtist,
                Operand::Field(TagField::Artist),
                true
            ))
        );
        assert_eq!(
            parse("title~^Live"),
            Ok(Condition::Matches {
                field: TagField::Title,
                pattern: "^Live".into()
            })
        );
        assert_eq!(
            parse("comment=a=b"),
            Ok(compare(
                TagField::Comment,
                Operand::Value("a=b".into()),
                false
            ))
        );
    }

    #[test]
    fn rejects_invalid_conditions() {
        assert!(parse("genre").unwrap_err().contains("expected FIELD=VALUE"));
        assert!(parse("title~(").unwrap_err().contains("bad regex"));
        // Unknown field names are rejected by TagFieldParser; its wording is covered end to end.
        assert!(parse("genres=Rock").is_err());
        assert!(parse("artist=@nope").is_err());
    }

    fn tag(build: impl FnOnce(&mut Tag)) -> Tag {
        let mut tag = Tag::new();
        build(&mut tag);
        tag
    }

    fn matcher(conditions: &[&str], missing: &[TagField], ignore_case: bool) -> Matcher {
        let conditions: Vec<_> = conditions.iter().map(|c| parse(c).unwrap()).collect();
        Matcher::new(&conditions, missing, &[], ignore_case)
    }

    #[test]
    fn evaluates_all_conditions_together() {
        let tag = tag(|tag| {
            tag.set_artist("Artist");
            tag.set_album_artist("Various Artists");
            tag.set_genre("Rock");
        });

        assert!(matcher(&["genre=Rock", "albumartist!=@artist"], &[], false).matches(Some(&tag)));
        assert!(!matcher(&["genre=Rock", "albumartist=@artist"], &[], false).matches(Some(&tag)));
        assert!(!matcher(&["genre=rock"], &[], false).matches(Some(&tag)));
        assert!(matcher(&["genre=rock"], &[], true).matches(Some(&tag)));
        assert!(matcher(&["albumartist~^various"], &[], true).matches(Some(&tag)));
        assert!(!matcher(&["albumartist~^various"], &[], false).matches(Some(&tag)));
    }

    #[test]
    fn missing_and_empty_fields_compare_as_empty() {
        let tag = tag(|tag| tag.set_artist("Artist"));

        assert!(matcher(&["genre="], &[], false).matches(Some(&tag)));
        assert!(matcher(&[], &[TagField::Genre], false).matches(Some(&tag)));
        assert!(!matcher(&[], &[TagField::Artist], false).matches(Some(&tag)));
        assert!(matcher(&[], &[TagField::Artist], false).matches(None));
        let present = Matcher::new(&[], &[], &[TagField::Artist], false);
        assert!(present.matches(Some(&tag)));
        assert!(!present.matches(None));
    }

    #[test]
    fn no_conditions_match_everything() {
        assert!(Matcher::new(&[], &[], &[], false).matches(None));
    }
}
