#!/usr/bin/env python3
"""Build Specular's offline ROM identification tables from libretro-database.

Downloads the No-Intro and Redump .dat files (game lists in clrmamepro's text
format) for each supported system from a pinned libretro-database commit and
writes one small TSV per system: a `#` line naming the source, then one line per
ROM file, sorted by name:

    CRC32<TAB>SERIAL<TAB>GAME NAME

SERIAL is the game code a ROM header holds, or a disc's serial, where the .dat
lists one. Disc serials are cut to the part a disc's boot file names: Redump's
"SLUS-01272GH" and "SLES-02375-P" become "SLUS-01272" and "SLES-02375".

Usage:
    python3 tools/trim-db.py [--commit SHA] [--out DIR]
"""

import argparse
import pathlib
import re
import sys
import urllib.parse
import urllib.request

DEFAULT_COMMIT = "12c21cf79f1e20eff6ac68b80875816c717d0a7a"
URL = "https://raw.githubusercontent.com/libretro/libretro-database/{commit}/metadat/{path}"

# Specular system id -> libretro-database .dat file, under metadat/.
SYSTEMS = {
    "gb": "no-intro/Nintendo - Game Boy.dat",
    "gbc": "no-intro/Nintendo - Game Boy Color.dat",
    "gba": "no-intro/Nintendo - Game Boy Advance.dat",
    "nes": "no-intro/Nintendo - Nintendo Entertainment System.dat",
    "snes": "no-intro/Nintendo - Super Nintendo Entertainment System.dat",
    "md": "no-intro/Sega - Mega Drive - Genesis.dat",
    "ps1": "redump/Sony - PlayStation.dat",
}

# Systems whose serials are disc serials -> the form a disc's boot file gives
# ("SLUS_012.34" is "SLUS-01234"); a serial that starts with it is cut to it.
DISC_SERIAL = {"ps1": re.compile(r"[A-Z]{4}-\d{5}")}

NOTICE = """\
The tables in this directory are derived from libretro-database
(https://github.com/libretro/libretro-database), which is licensed under
CC-BY-SA 4.0 (https://creativecommons.org/licenses/by-sa/4.0/). These derived
tables are distributed under the same licence.

They contain only each ROM's CRC32, serial number and game name.

Regenerate them with: python3 tools/trim-db.py
"""

# A token is a quoted string (which may contain parentheses and \" escapes),
# a parenthesis, or a bare word.
TOKEN = re.compile(r'"((?:[^"\\]|\\.)*)"|([()])|([^\s()"]+)')


def parse(text):
    """Parse clrmamepro text into nested lists of (key, value) pairs.

    A value is either a string or, for `key ( ... )`, a list of pairs.
    """
    tokens = []
    for quoted, paren, word in TOKEN.findall(text):
        if paren:
            tokens.append(paren)
        elif word:
            tokens.append(("str", word))
        else:
            tokens.append(("str", re.sub(r"\\(.)", r"\1", quoted)))

    pos = 0

    def block():
        nonlocal pos
        pairs = []
        while pos < len(tokens) and tokens[pos] != ")":
            key = tokens[pos]
            if key == "(" or pos + 1 >= len(tokens):
                raise ValueError(f"unexpected token at {pos}: {key!r}")
            pos += 1
            if tokens[pos] == "(":
                pos += 1
                value = block()
                if pos >= len(tokens):
                    raise ValueError("unclosed block")
                pos += 1  # skip ")"
            else:
                value = tokens[pos][1]
                pos += 1
            pairs.append((key[1], value))
        return pairs

    result = block()
    if pos != len(tokens):
        raise ValueError(f"unmatched ')' at token {pos}")
    return result


def first(pairs, key):
    """Return the first string value for `key`, or "" if absent."""
    return next((v for k, v in pairs if k == key and isinstance(v, str)), "")


def clean(field):
    """Replace tabs and line breaks, which would break the TSV, with spaces."""
    return re.sub(r"[\t\r\n]", " ", field)


def table_lines(text, disc_serial=None):
    """Return the sorted, de-duplicated TSV lines for one .dat file.

    `disc_serial`, if given, is the pattern serials are cut to (see DISC_SERIAL).
    """
    rows = set()
    for key, game in parse(text):
        if key != "game":
            continue
        name = first(game, "name")
        for rom in (v for k, v in game if k == "rom"):
            crc = first(rom, "crc")
            if not crc:
                print(f"  skipping rom without crc in {name!r}", file=sys.stderr)
                continue
            if not re.fullmatch(r"[0-9A-Fa-f]{1,8}", crc):
                raise ValueError(f"bad crc {crc!r} in {name!r}")
            serial = first(rom, "serial") or first(game, "serial")
            if disc_serial and (m := disc_serial.match(serial)):
                serial = m.group(0)
            rows.add((clean(name), crc.upper().zfill(8), clean(serial)))
    return [f"{crc}\t{serial}\t{name}" for name, crc, serial in sorted(rows)]


def main():
    default_out = pathlib.Path(__file__).resolve().parent.parent / "data" / "db"
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--commit", default=DEFAULT_COMMIT)
    ap.add_argument("--out", type=pathlib.Path, default=default_out)
    args = ap.parse_args()

    args.out.mkdir(parents=True, exist_ok=True)
    for system, path in SYSTEMS.items():
        url = URL.format(commit=args.commit, path=urllib.parse.quote(path))
        print(f"{system}: {url}", file=sys.stderr)
        with urllib.request.urlopen(url) as response:
            text = response.read().decode("utf-8")
        header = (
            f"# From libretro-database {args.commit}, metadat/{path} "
            "(CC-BY-SA 4.0). Generated by tools/trim-db.py; do not edit."
        )
        lines = [header] + table_lines(text, DISC_SERIAL.get(system))
        (args.out / f"{system}.tsv").write_text("\n".join(lines) + "\n", encoding="utf-8")
    (args.out / "NOTICE").write_text(NOTICE, encoding="utf-8")


if __name__ == "__main__":
    main()
