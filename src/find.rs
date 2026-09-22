//! `idk find`: print the files whose tags match every condition.

use std::io::Write;
use std::path::Path;
use std::process::ExitCode;

use clap::error::ErrorKind;
use id3::Tag;
use regex::{Regex, RegexBuilder};

use crate::cli::{FindArgs, usage_error};
use crate::input;
use crate::runner::{self, Order};
use crate::tag_field::TagField;

/// One check against a file's tag.
#[derive(Debug)]
enum Condition {
    /// `FIELD=VALUE` or `FIELD=@OTHER` (`FIELD!=...` when `negate`).
    Compare {
        field: TagField,
        operand: Operand,
        negate: bool,
    },
    /// `FIELD~REGEX`.
    Matches { field: TagField, regex: Regex },
    /// `--missing FIELD`.
    Missing(TagField),
    /// `--present FIELD`.
    Present(TagField),
}

/// The right-hand side of a comparison.
#[derive(Debug, PartialEq)]
enum Operand {
    /// A literal value.
    Value(String),
    /// Another field's value, written `@FIELD`.
    Field(TagField),
}

/// Parses a `--where` condition; `ignore_case` applies to its regex.
fn parse_condition(raw: &str, ignore_case: bool) -> Result<Condition, String> {
    let at = raw
        .find(['=', '~'])
        .ok_or("expected FIELD=VALUE, FIELD!=VALUE or FIELD~REGEX")?;
    let rest = &raw[at + 1..];
    if raw[at..].starts_with('~') {
        let regex = RegexBuilder::new(rest)
            .case_insensitive(ignore_case)
            .build()
            .map_err(|err| format!("bad regex: {err}"))?;
        return Ok(Condition::Matches {
            field: raw[..at].parse()?,
            regex,
        });
    }
    let (name, negate) = match raw[..at].strip_suffix('!') {
        Some(name) => (name, true),
        None => (&raw[..at], false),
    };
    let operand = match rest.strip_prefix('@') {
        Some(other) => Operand::Field(other.parse()?),
        None => Operand::Value(rest.to_owned()),
    };
    Ok(Condition::Compare {
        field: name.parse()?,
        operand,
        negate,
    })
}

/// Every condition from the command line; a file matches when all of them hold.
pub struct Matcher {
    conditions: Vec<Condition>,
    ignore_case: bool,
}

impl Matcher {
    /// Parses `--where` conditions and adds `--missing` / `--present` checks.
    pub fn new(
        conditions: &[String],
        missing: &[TagField],
        present: &[TagField],
        ignore_case: bool,
    ) -> Result<Self, String> {
        let mut parsed = Vec::new();
        for raw in conditions {
            let condition = parse_condition(raw, ignore_case)
                .map_err(|message| format!("invalid condition '{raw}': {message}"))?;
            parsed.push(condition);
        }
        parsed.extend(missing.iter().cloned().map(Condition::Missing));
        parsed.extend(present.iter().cloned().map(Condition::Present));
        Ok(Matcher {
            conditions: parsed,
            ignore_case,
        })
    }

    /// Whether `tag` (`None` for an untagged file) satisfies every condition.
    /// A missing field compares as `""`.
    pub fn matches(&self, tag: Option<&Tag>) -> bool {
        let value = |field: &TagField| tag.and_then(|tag| field.read(tag)).unwrap_or_default();
        self.conditions.iter().all(|condition| match condition {
            Condition::Compare {
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
            Condition::Matches { field, regex } => regex.is_match(&value(field)),
            Condition::Missing(field) => value(field).trim().is_empty(),
            Condition::Present(field) => !value(field).trim().is_empty(),
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
    let matcher = Matcher::new(
        &args.conditions,
        &args.missing,
        &args.present,
        args.ignore_case,
    )
    .unwrap_or_else(|message| usage_error(ErrorKind::ValueValidation, message));
    let separator: &[u8] = if args.null { b"\0" } else { b"\n" };
    let show_progress = args.run.progress_for_stdout();
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

    runner::exit_code(failed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use id3::TagLike;

    #[test]
    fn parses_every_operator() {
        let parse = |raw| parse_condition(raw, false).unwrap();
        assert!(matches!(
            parse("genre=Rock"),
            Condition::Compare { field: TagField::Genre, operand: Operand::Value(v), negate: false } if v == "Rock"
        ));
        assert!(matches!(
            parse("genre!="),
            Condition::Compare { operand: Operand::Value(v), negate: true, .. } if v.is_empty()
        ));
        assert!(matches!(
            parse("albumartist!=@artist"),
            Condition::Compare {
                field: TagField::AlbumArtist,
                operand: Operand::Field(TagField::Artist),
                negate: true
            }
        ));
        assert!(matches!(
            parse("title~^Live"),
            Condition::Matches { field: TagField::Title, regex } if regex.as_str() == "^Live"
        ));
        assert!(matches!(
            parse("comment=a=b"),
            Condition::Compare { operand: Operand::Value(v), .. } if v == "a=b"
        ));
    }

    #[test]
    fn rejects_invalid_conditions() {
        let cases = [
            ("genre", "invalid condition 'genre': expected FIELD=VALUE"),
            ("title~(", "invalid condition 'title~(': bad regex"),
            (
                "genres=Rock",
                "invalid condition 'genres=Rock': unknown tag 'genres'",
            ),
            (
                "artist=@nope",
                "invalid condition 'artist=@nope': unknown tag 'nope'",
            ),
        ];
        for (raw, message) in cases {
            let err = Matcher::new(&[raw.to_owned()], &[], &[], false)
                .err()
                .unwrap();
            assert!(err.starts_with(message), "{err}");
        }
    }

    fn tag(build: impl FnOnce(&mut Tag)) -> Tag {
        let mut tag = Tag::new();
        build(&mut tag);
        tag
    }

    fn matcher(conditions: &[&str], missing: &[TagField], ignore_case: bool) -> Matcher {
        let conditions: Vec<String> = conditions.iter().map(|c| c.to_string()).collect();
        Matcher::new(&conditions, missing, &[], ignore_case).unwrap()
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
        let present = Matcher::new(&[], &[], &[TagField::Artist], false).unwrap();
        assert!(present.matches(Some(&tag)));
        assert!(!present.matches(None));
    }

    #[test]
    fn no_conditions_match_everything() {
        assert!(Matcher::new(&[], &[], &[], false).unwrap().matches(None));
    }
}
