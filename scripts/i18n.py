#!/usr/bin/env python3
"""Ferricone's translation template and catalogs (see po/README.md).

  scripts/i18n.py extract   po/ferricone.pot from ui/*.slint (@tr) and src/ (tr!, trn!, n_!)
  scripts/i18n.py update    extract, then merge it into every po/<lang>/LC_MESSAGES/ferricone.po
  scripts/i18n.py check     fail if the template is out of date or a catalog doesn't compile (CI)
  scripts/i18n.py new LANG  start po/LANG/LC_MESSAGES/ferricone.po from the template

`update`, `check` and `new` need GNU gettext (msgmerge, msgfmt, msginit)."""

import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
POT = ROOT / "po" / "ferricone.pot"
STR = r'"((?:[^"\\]|\\.)*)"'
# @tr("text"), @tr("text", args), @tr("one" | "many" % n)
SLINT = re.compile(r'@tr\(\s*' + STR + r'(?:\s*\|\s*' + STR + r')?')
RUST = re.compile(r'\b(tr|n_)!\(\s*' + STR + r'|\btrn!\(\s*' + STR + r'\s*,\s*' + STR)


def strings():
    """{(msgid, plural or None): [file:line, ...]} in source order."""
    found = {}

    def add(msgid, plural, path, text, at):
        line = text.count("\n", 0, at) + 1
        found.setdefault((msgid, plural), []).append(f"{path.relative_to(ROOT)}:{line}")

    for path in sorted((ROOT / "ui").glob("*.slint")):
        text = path.read_text()
        for m in SLINT.finditer(text):
            add(m.group(1), m.group(2), path, text, m.start())
    for path in sorted((ROOT / "src").rglob("*.rs")):
        text = path.read_text()
        cut = text.find("#[cfg(test)]")
        body = text if cut < 0 else text[:cut]
        # Comments (as blank lines, to keep line numbers): examples in docs.
        body = "\n".join("" if l.lstrip().startswith("//") else l for l in body.split("\n"))
        for m in RUST.finditer(body):
            if m.group(2) is not None:
                add(m.group(2), None, path, body, m.start())
            else:
                add(m.group(3), m.group(4), path, body, m.start())
    return found


def template():
    out = [
        "# Ferricone's translation template, made by scripts/i18n.py extract: don't edit.",
        'msgid ""',
        'msgstr ""',
        '"Project-Id-Version: Ferricone\\n"',
        '"MIME-Version: 1.0\\n"',
        '"Content-Type: text/plain; charset=UTF-8\\n"',
        '"Content-Transfer-Encoding: 8bit\\n"',
        "",
    ]
    for (msgid, plural), places in strings().items():
        out.append("#: " + " ".join(places))
        if "{" in msgid:
            out.append("#. Keep each {} or {n}: it stands for a value.")
        out.append(f'msgid "{msgid}"')
        if plural is None:
            out.append('msgstr ""')
        else:
            out += [f'msgid_plural "{plural}"', 'msgstr[0] ""', 'msgstr[1] ""']
        out.append("")
    return "\n".join(out)


def catalogs():
    return sorted((ROOT / "po").glob("*/LC_MESSAGES/ferricone.po"))


def run(*args):
    result = subprocess.run(args, capture_output=True, text=True)
    if result.returncode:
        sys.exit(f"{' '.join(map(str, args))}:\n{result.stderr}")
    return result.stdout


def main():
    command = sys.argv[1] if len(sys.argv) > 1 else ""
    if command == "extract":
        POT.write_text(template())
        print(f"{POT.relative_to(ROOT)}: {len(strings())} strings")
    elif command == "update":
        POT.write_text(template())
        for po in catalogs():
            run("msgmerge", "--quiet", "--update", "--backup=none", "--no-fuzzy-matching", po, POT)
            print(f"{po.relative_to(ROOT)}: merged")
    elif command == "new" and len(sys.argv) == 3:
        lang = sys.argv[2]
        po = ROOT / "po" / lang / "LC_MESSAGES" / "ferricone.po"
        if po.exists():
            sys.exit(f"{po.relative_to(ROOT)} exists")
        po.parent.mkdir(parents=True)
        run("msginit", "--no-translator", f"--locale={lang}.UTF-8", "--input", POT, "--output-file", po)
        print(f"{po.relative_to(ROOT)}: add X-Language-Name (the language's own name) to its header")
    elif command == "check":
        problems = []
        if not POT.exists() or POT.read_text() != template():
            problems.append("po/ferricone.pot is out of date: run scripts/i18n.py update")
        for po in catalogs():
            result = subprocess.run(
                ["msgfmt", "--check", "--output-file=/dev/null", po], capture_output=True, text=True
            )
            if result.returncode:
                problems.append(f"{po.relative_to(ROOT)}:\n{result.stderr}")
            if "X-Language-Name:" not in po.read_text():
                problems.append(f"{po.relative_to(ROOT)}: no X-Language-Name in its header")
        if problems:
            sys.exit("\n".join(problems))
        print(f"translations ok: {len(strings())} strings, {len(catalogs())} languages")
    else:
        sys.exit(__doc__)


if __name__ == "__main__":
    main()
