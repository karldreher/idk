# idk
The ID3 Knife, CLI Tool for MP3 metadata edits

## Install

Download the archive for your platform from [Releases](https://github.com/karldreher/idk/releases), or build from source:

```bash
cargo install --path .
```

## Usage

### Copy one tag into another

```bash
idk copy tags --from artist --to albumartist song.mp3
idk copy tags --from albumartist --to artist *.mp3
idk copy tags --from artist --to albumartist --dry-run *.mp3   # preview only
```

The destination tag is overwritten. Other metadata is left untouched, and files whose destination already matches are not rewritten.

| Tag                  | ID3v2 frame                          | Aliases                |
|----------------------|--------------------------------------|------------------------|
| `artist`             | `TPE1`                               | `tpe1`                 |
| `albumartist`        | `TPE2`                               | `album-artist`, `tpe2` |
| `title`              | `TIT2`                               | `tit2`                 |
| `album`              | `TALB`                               | `talb`                 |
| `tracknumber`        | `TRCK`                               | `track`, `trck`        |
| `discnumber`         | `TPOS`                               | `disc`, `tpos`         |
| `date`               | `TDRC` (v2.4) / `TYER`, year only (v2.3) | `year`, `tdrc`, `tyer` |
| `genre`              | `TCON`                               | `tcon`                 |
| `composer`           | `TCOM`                               | `tcom`                 |
| `comment`            | `COMM` (empty description, `eng`)    | `comm`                 |
| `txxx:<description>` | `TXXX` with that description         |                        |

Tag names are case-insensitive; `txxx:` descriptions are matched exactly.

| Option            | Description                                                          |
|-------------------|----------------------------------------------------------------------|
| `-n, --dry-run`   | Print each change (`song.mp3: albumartist "Old" -> "New"`) without writing any file |
| `--fail-on-empty` | Treat files with no source value as failures instead of skipping them |
| `-j, --jobs <N>`  | Maximum files processed concurrently (default: available CPUs)       |
| `-q, --quiet`     | Hide the progress bar and summary; errors are still reported         |

Exit codes: `0` success, `1` one or more files failed, `2` invalid usage.

### Merge genres and artists

Replace variant values with one canonical value. Matching ignores case and surrounding whitespace, `""` matches a missing or empty value, and genre references such as `(9)` match by name (`Metal`).

```bash
idk merge genres --from "Heavy Metal" Metal --to Rock *.mp3
idk merge artists --from Beatles "the beatles" --to "The Beatles" *.mp3
idk merge genres --from "" --to Unknown *.mp3      # fill missing genres
```

`--from` takes several values, so put `--to` after it (or repeat `--from` per value). `merge genres` changes only `TCON`; `merge artists` changes only `TPE1`. `--dry-run`, `--jobs` and `--quiet` work as for `copy tags`.

### Config file

Operations can read their settings from YAML instead of flags. The top-level `tags` key is required, unknown keys are rejected, and each command reads only its own section.

```yaml
tags:
  merge:
    genres:
      from:
        - "Heavy Metal"
        - "Metal"
      to: Rock              # a single string, not a list
    artists:
      from:
        - "Beatles"
      to: The Beatles
  copy:
    from: artist
    to: albumartist
```

```bash
idk merge genres --config my-cool-file.yaml *.mp3
idk merge artists *.mp3 --config      # bare --config reads ./idk.yaml
idk copy tags *.mp3 --config
```

The config file is optional, but whenever one is given it is validated against idk's JSON Schema before any audio file is touched. Every violation is reported with its key path (for example `tags.merge.genres.to: ["Rock"] is not of type "string"`), and the run exits with `1`.

`--config` can't be combined with `--from`/`--to`. A bare `--config` takes the next word as its file unless that word starts with `-`, so put it after the files (or before another flag).

### Show tags

```bash
idk show song.mp3
idk show --json *.mp3 | jq '.[].tags.albumartist'
idk show --field artist --field albumartist *.mp3
```

Known frames are shown by the field names above; other frames by their frame ID (comments with a description as `COMM:<description>`). Pictures are summarized as `image/jpeg, 12345 bytes`. Output follows input order.

With `--json`, stdout is one array:

```json
[{ "path": "song.mp3", "version": "2.4", "tags": { "artist": "Artist", "title": "Title" } }]
```

Files without an ID3v2 tag have `"version": null` and empty `tags`. `-j/--jobs` and `-q/--quiet` apply as for `copy tags`.

## Release

1. Bump `version` in `Cargo.toml` and merge to `main`.
2. Tag and push: `git tag v0.2.0 && git push origin v0.2.0`.
3. The Release workflow builds macOS (arm64, x86_64), Linux x86_64 and Windows x86_64 archives and publishes a GitHub Release with them attached.
