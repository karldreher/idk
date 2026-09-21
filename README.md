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
```

The destination tag is overwritten. Other metadata is left untouched, and files whose destination already matches are not rewritten.

| Tag           | ID3v2 frame | Aliases                |
|---------------|-------------|------------------------|
| `artist`      | `TPE1`      | `tpe1`                 |
| `albumartist` | `TPE2`      | `album-artist`, `tpe2` |

Tag names are case-insensitive.

| Option            | Description                                                          |
|-------------------|----------------------------------------------------------------------|
| `--fail-on-empty` | Treat files with no source value as failures instead of skipping them |
| `-j, --jobs <N>`  | Maximum files processed concurrently (default: available CPUs)       |
| `-q, --quiet`     | Hide the progress bar and summary; errors are still reported         |

Exit codes: `0` success, `1` one or more files failed, `2` invalid usage.

## Release

1. Bump `version` in `Cargo.toml` and merge to `main`.
2. Tag and push: `git tag v0.2.0 && git push origin v0.2.0`.
3. The Release workflow builds macOS (arm64, x86_64), Linux x86_64 and Windows x86_64 archives and attaches them to a draft GitHub Release for review.
