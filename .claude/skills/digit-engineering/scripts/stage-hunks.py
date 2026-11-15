#!/usr/bin/env python3
"""Stage chosen changes of one file without `git add -p` (which needs a TTY).

    stage-hunks.py FILE            # list the file's changes, numbered
    stage-hunks.py FILE 1 3 4      # stage only changes 1, 3 and 4

Builds the new index blob directly: the index version of FILE with the chosen
changes taken from the working tree. The working tree is never touched, so
unstaged work stays exactly where it is. Run the list again after staging:
numbers are relative to what is still unstaged.
"""
import difflib
import subprocess
import sys


def git(*args, data=None):
    return subprocess.run(["git", *args], input=data, capture_output=True, check=True).stdout


def main():
    if len(sys.argv) < 2:
        sys.exit(__doc__)
    path, picks = sys.argv[1], {int(n) for n in sys.argv[2:]}
    root = git("rev-parse", "--show-toplevel").decode().strip()
    rel = git("ls-files", "--full-name", "--error-unmatch", path).decode().strip() if git("ls-files", path).strip() else None
    if rel is None:
        sys.exit(f"{path} is not tracked; use `git add {path}` for a new file")
    staged = git("show", f":{rel}").decode().splitlines(keepends=True)
    with open(f"{root}/{rel}", encoding="utf-8") as f:
        working = f.read().splitlines(keepends=True)
    changes = [op for op in difflib.SequenceMatcher(None, staged, working, autojunk=False).get_opcodes() if op[0] != "equal"]
    if not picks:
        for n, (_, i1, i2, j1, j2) in enumerate(changes, 1):
            print(f"── {n}: index lines {i1 + 1}-{i2}, working lines {j1 + 1}-{j2}")
            for line in staged[i1:i2][:8]:
                print("-" + line, end="" if line.endswith("\n") else "\n")
            for line in working[j1:j2][:8]:
                print("+" + line, end="" if line.endswith("\n") else "\n")
        if not changes:
            print("nothing unstaged")
        return
    bad = picks - set(range(1, len(changes) + 1))
    if bad:
        sys.exit(f"no change numbered {sorted(bad)}")
    out, at = [], 0
    for n, (_, i1, i2, j1, j2) in enumerate(changes, 1):
        out += staged[at:i1]
        out += working[j1:j2] if n in picks else staged[i1:i2]
        at = i2
    out += staged[at:]
    mode = git("ls-files", "-s", "--", rel).decode().split()[0]
    sha = git("hash-object", "-w", "--stdin", data="".join(out).encode()).decode().strip()
    git("update-index", "--cacheinfo", f"{mode},{sha},{rel}")
    print(f"staged {len(picks)} of {len(changes)} changes in {rel}")


if __name__ == "__main__":
    main()
