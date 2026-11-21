#!/usr/bin/env python3
"""Release helper for cross-control. Standard library only (Python 3.8+).

    release.py version [--tag vX.Y.Z]   print the version; check every place
                                        that carries it agrees (and the tag)
    release.py notes VERSION            print that version's CHANGELOG section
    release.py package --target T --version V --bin-dir DIR --out DIR
                                        build the release archive for target T
    release.py bump NEW_VERSION         set a new version everywhere, and open
                                        its CHANGELOG section from [Unreleased]

The version lives in the workspace Cargo.toml (package and internal
dependency versions), Cargo.lock, and the PowerShell module manifest.
"""

import argparse
import datetime
import hashlib
import os
import re
import sys
import tarfile
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
CARGO_TOML = ROOT / "Cargo.toml"
CARGO_LOCK = ROOT / "Cargo.lock"
CHANGELOG = ROOT / "CHANGELOG.md"
PSD1 = ROOT / "deploy/powershell/CrossControl/CrossControl.psd1"

SEMVER = re.compile(r"^(\d+)\.(\d+)\.(\d+)(?:-([0-9A-Za-z.-]+))?$")


def die(message):
    print(f"release.py: {message}", file=sys.stderr)
    sys.exit(1)


def read(path):
    return path.read_text(encoding="utf-8")


def write(path, text):
    path.write_text(text, encoding="utf-8", newline="\n")


# --- Version -------------------------------------------------------------


def workspace_version():
    match = re.search(
        r'^\[workspace\.package\][^\[]*?^version = "([^"]+)"', read(CARGO_TOML), re.M | re.S
    )
    if not match:
        die("no version in [workspace.package] of Cargo.toml")
    return match.group(1)


def split_prerelease(version):
    match = SEMVER.match(version)
    if not match:
        die(f"{version!r} is not a semantic version (X.Y.Z or X.Y.Z-pre)")
    return ".".join(match.group(1, 2, 3)), match.group(4) or ""


def psd1_prerelease_tag(prerelease):
    # PowerShellGet allows only ASCII letters and digits in Prerelease.
    return re.sub(r"[^0-9A-Za-z]", "", prerelease)


def version_problems(version):
    problems = []
    text = read(CARGO_TOML)
    for name, found in re.findall(r'^(cross-control-[a-z-]+) = \{ version = "([^"]+)"', text, re.M):
        if found != version:
            problems.append(f"Cargo.toml: {name} dependency is {found}")
    lock = read(CARGO_LOCK)
    for name, found in re.findall(r'name = "(cross-control[a-z-]*)"\nversion = "([^"]+)"', lock):
        if found != version:
            problems.append(f"Cargo.lock: {name} is {found} (run `cargo update --workspace`)")
    base, prerelease = split_prerelease(version)
    psd1 = read(PSD1)
    module_version = re.search(r"ModuleVersion\s*=\s*'([^']+)'", psd1)
    if not module_version or module_version.group(1) != base:
        problems.append(f"CrossControl.psd1: ModuleVersion should be {base}")
    tag = re.search(r"Prerelease\s*=\s*'([^']*)'", psd1)
    want = psd1_prerelease_tag(prerelease)
    if (tag.group(1) if tag else "") != want:
        problems.append(f"CrossControl.psd1: Prerelease should be '{want}'")
    return problems


def cmd_version(args):
    version = workspace_version()
    split_prerelease(version)
    problems = version_problems(version)
    if args.tag:
        tag = re.sub(r"^refs/tags/", "", args.tag)
        if tag != f"v{version}":
            problems.append(f"tag {tag} does not match the version {version} (expected v{version})")
    if problems:
        die("versions disagree:\n  " + "\n  ".join(problems))
    print(version)


# --- Release notes ----------------------------------------------------------


def changelog_section(version):
    """The body of `## [version]`, up to the next `## [`; None if absent."""
    lines = read(CHANGELOG).splitlines()
    heading = re.compile(r"^## \[" + re.escape(version) + r"\]")
    for start, line in enumerate(lines):
        if heading.match(line):
            body = []
            for line in lines[start + 1 :]:
                if line.startswith("## ["):
                    break
                body.append(line)
            return "\n".join(body).strip() + "\n"
    return None


def cmd_notes(args):
    notes = changelog_section(args.version)
    if notes is None:
        if not args.allow_unreleased:
            die(f"CHANGELOG.md has no section for [{args.version}]")
        notes = changelog_section("Unreleased") or ""
        notes = f"_Unreleased changes (dry run)._\n\n{notes}"
    sys.stdout.write(notes)


# --- Packaging --------------------------------------------------------------

COMMON_FILES = ["README.md", "CHANGELOG.md", "LICENSE-MIT", "LICENSE-APACHE"]


def package_contents(target, bin_dir):
    """(source path, path inside the archive) pairs."""
    windows = "windows" in target
    exe = ".exe" if windows else ""
    contents = [(Path(bin_dir) / f"cross-control{exe}", f"cross-control{exe}")]
    if windows:
        contents.append((Path(bin_dir) / "cross-controlw.exe", "cross-controlw.exe"))
        module = ROOT / "deploy/powershell/CrossControl"
        for path in sorted(module.iterdir()):
            contents.append((path, f"CrossControl/{path.name}"))
    for name in COMMON_FILES:
        contents.append((ROOT / name, name))
    contents.append((ROOT / "examples/config.toml", "config.example.toml"))
    return contents


def cmd_package(args):
    windows = "windows" in args.target
    stem = f"cross-control-{args.version}-{args.target}"
    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    contents = package_contents(args.target, args.bin_dir)
    for source, _ in contents:
        if not source.is_file():
            die(f"{source} not found")

    if windows:
        archive = out / f"{stem}.zip"
        with zipfile.ZipFile(archive, "w", zipfile.ZIP_DEFLATED) as zf:
            for source, name in contents:
                zf.write(source, f"{stem}/{name}")
    else:
        archive = out / f"{stem}.tar.gz"
        with tarfile.open(archive, "w:gz") as tf:
            for source, name in contents:
                info = tf.gettarinfo(str(source), f"{stem}/{name}")
                info.uid = info.gid = 0
                info.uname = info.gname = ""
                info.mode = 0o755 if name == "cross-control" else 0o644
                with open(source, "rb") as fh:
                    tf.addfile(info, fh)

    digest = hashlib.sha256(archive.read_bytes()).hexdigest()
    (out / f"{archive.name}.sha256").write_text(f"{digest}  {archive.name}\n", encoding="utf-8")
    print(archive)


# --- Bump -------------------------------------------------------------------


def cmd_bump(args):
    new = args.version
    base, prerelease = split_prerelease(new)
    old = workspace_version()
    if new == old:
        die(f"the version is already {new}")

    text = read(CARGO_TOML)
    text = re.sub(
        r'(^\[workspace\.package\][^\[]*?^version = ")[^"]+(")',
        lambda m: m.group(1) + new + m.group(2),
        text,
        count=1,
        flags=re.M | re.S,
    )
    text = re.sub(
        r'(^cross-control-[a-z-]+ = \{ version = ")[^"]+(")',
        lambda m: m.group(1) + new + m.group(2),
        text,
        flags=re.M,
    )
    write(CARGO_TOML, text)

    lock = read(CARGO_LOCK)
    lock = re.sub(
        r'(name = "cross-control[a-z-]*"\nversion = ")[^"]+(")',
        lambda m: m.group(1) + new + m.group(2),
        lock,
    )
    write(CARGO_LOCK, lock)

    psd1 = read(PSD1)
    psd1 = re.sub(r"(ModuleVersion\s*=\s*')[^']+(')", lambda m: m.group(1) + base + m.group(2), psd1)
    tag = psd1_prerelease_tag(prerelease)
    if re.search(r"Prerelease\s*=", psd1):
        psd1 = re.sub(r"(Prerelease\s*=\s*')[^']*(')", lambda m: m.group(1) + tag + m.group(2), psd1)
    else:
        psd1 = psd1.replace("PSData = @{", f"PSData = @{{\n            Prerelease = '{tag}'", 1)
    write(PSD1, psd1)

    changelog = read(CHANGELOG)
    if changelog_section(new) is None:
        today = args.date or datetime.date.today().isoformat()
        if "## [Unreleased]" not in changelog:
            die("CHANGELOG.md has no [Unreleased] section")
        changelog = changelog.replace(
            "## [Unreleased]", f"## [Unreleased]\n\n## [{new}] - {today}", 1
        )
        write(CHANGELOG, changelog)

    problems = version_problems(new)
    if problems:
        die("bump left disagreements:\n  " + "\n  ".join(problems))
    print(f"{old} -> {new}")


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = parser.add_subparsers(dest="command", required=True)

    p = sub.add_parser("version")
    p.add_argument("--tag", help="the git tag being released (vX.Y.Z or refs/tags/vX.Y.Z)")
    p.set_defaults(func=cmd_version)

    p = sub.add_parser("notes")
    p.add_argument("version")
    p.add_argument("--allow-unreleased", action="store_true", help="fall back to [Unreleased] (dry runs)")
    p.set_defaults(func=cmd_notes)

    p = sub.add_parser("package")
    p.add_argument("--target", required=True)
    p.add_argument("--version", required=True)
    p.add_argument("--bin-dir", required=True)
    p.add_argument("--out", required=True)
    p.set_defaults(func=cmd_package)

    p = sub.add_parser("bump")
    p.add_argument("version")
    p.add_argument("--date", help="release date for the CHANGELOG heading (default: today)")
    p.set_defaults(func=cmd_bump)

    args = parser.parse_args()
    args.func(args)


if __name__ == "__main__":
    os.chdir(ROOT)
    main()
