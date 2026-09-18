"""Generate the README keyboard table from the desktop binding source."""
import argparse
from pathlib import Path

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--check", action="store_true", help="Fail when README needs updating")
args = parser.parse_args()
root = Path(__file__).resolve().parent.parent
rows = (root / "crates/duckie-desktop/src/shortcuts.tsv").read_text(encoding="utf-8").splitlines()[1:]
table = "| Shortcut | Action | Context |\n| --- | --- | --- |\n"
for row in rows:
    _, chord, action, context = row.split("\t")
    table += f"| **{chord}** | {action} | {context} |\n"
readme = root / "README.md"
text = readme.read_text(encoding="utf-8")
start = "<!-- shortcuts:start -->\n"
end = "<!-- shortcuts:end -->"
before, remainder = text.split(start, 1)
_, after = remainder.split(end, 1)
updated = before + start + table + end + after
if args.check:
    if updated != text:
        parser.exit(1, "README shortcuts are stale; run python scripts/generate-shortcuts.py\n")
else:
    readme.write_text(updated, encoding="utf-8")
