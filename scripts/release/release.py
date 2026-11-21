#!/usr/bin/env python3
"""Release helper for cross-control. Standard library only (Python 3.8+).

    release.py version [--tag vX.Y.Z]   print the version; check every place
                                        that carries it agrees (and the tag)
    release.py notes VERSION            print that version's CHANGELOG section
    release.py package --target T --version V --bin-dir DIR --out DIR
                                        build the release archive for target T
    release.py linux-packages --target T --version V --bin-dir DIR --out DIR
                [--nfpm PATH]           build the .deb and .rpm for target T
    release.py homebrew --version V --sums FILE [--base-url URL]
                                        print the Homebrew formula
    release.py scoop --version V --sums FILE [--base-url URL]
                                        print the Scoop manifest
    release.py bump NEW_VERSION         set a new version everywhere, and open
                                        its CHANGELOG section from [Unreleased]

The version lives in the workspace Cargo.toml (package and internal
dependency versions), Cargo.lock, and the PowerShell module manifest.
"""

import argparse
import datetime
import hashlib
import json
import os
import re
import subprocess
import sys
import tempfile
import tarfile
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
CARGO_TOML = ROOT / "Cargo.toml"
CARGO_LOCK = ROOT / "Cargo.lock"
CHANGELOG = ROOT / "CHANGELOG.md"
PSD1 = ROOT / "deploy/powershell/CrossControl/CrossControl.psd1"

REPO = "Adjoint-uk/cross-control"
DESCRIPTION = "Share one keyboard and mouse across Linux, Windows and macOS machines"
HOMEPAGE = "https://cross-control.adjoint.uk"

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

    write_sha256(archive)
    print(archive)


def write_sha256(path):
    digest = hashlib.sha256(path.read_bytes()).hexdigest()
    (path.parent / f"{path.name}.sha256").write_text(f"{digest}  {path.name}\n", encoding="utf-8")


# --- Linux packages (.deb, .rpm) ---------------------------------------------

LINUX_ARCH = {"x86_64": "amd64", "aarch64": "arm64"}  # nfpm's (Go's) names


def nfpm_config(version, arch, bin_dir):
    """nfpm's configuration, as JSON (which is valid YAML)."""
    linux = ROOT / "packaging/linux"
    doc = "/usr/share/doc/cross-control"
    contents = [
        {"src": str(Path(bin_dir) / "cross-control"), "dst": "/usr/bin/cross-control",
         "file_info": {"mode": 0o755}},
        {"src": str(linux / "70-cross-control.rules"), "dst": "/usr/lib/udev/rules.d/70-cross-control.rules"},
        {"src": str(linux / "cross-control.conf"), "dst": "/usr/lib/modules-load.d/cross-control.conf"},
        {"src": str(ROOT / "examples/config.toml"), "dst": f"{doc}/config.example.toml"},
    ]
    for name in COMMON_FILES:
        contents.append({"src": str(ROOT / name), "dst": f"{doc}/{name}"})
    return {
        "name": "cross-control",
        "arch": arch,
        "platform": "linux",
        "version": version,
        "version_schema": "semver",
        "maintainer": "Adjoint Ltd <hello@adjoint.uk>",
        "description": DESCRIPTION + ", over an encrypted QUIC connection.",
        "vendor": "Adjoint Ltd",
        "homepage": HOMEPAGE,
        "license": "MIT OR Apache-2.0",
        "section": "utils",
        "priority": "optional",
        "contents": contents,
        "scripts": {"postinstall": str(linux / "postinstall.sh")},
    }


def cmd_linux_packages(args):
    machine = args.target.split("-")[0]
    if "linux" not in args.target or machine not in LINUX_ARCH:
        die(f"no Linux packages for target {args.target}")
    if not (Path(args.bin_dir) / "cross-control").is_file():
        die(f"{args.bin_dir}/cross-control not found")
    out = Path(args.out).resolve()
    out.mkdir(parents=True, exist_ok=True)
    config = nfpm_config(args.version, LINUX_ARCH[machine], Path(args.bin_dir).resolve())
    with tempfile.TemporaryDirectory() as tmp:
        config_path = Path(tmp) / "nfpm.yaml"
        config_path.write_text(json.dumps(config, indent=2), encoding="utf-8")
        for packager in ("deb", "rpm"):
            before = set(out.iterdir())
            subprocess.run(
                [args.nfpm, "package", "--config", str(config_path), "--packager", packager, "--target", str(out)],
                check=True,
                stdout=subprocess.DEVNULL,
            )
            for path in sorted(set(out.iterdir()) - before):
                write_sha256(path)
                print(path)


# --- Homebrew and Scoop ------------------------------------------------------


def read_sums(path):
    """{file name: sha256} from a SHA256SUMS file."""
    sums = {}
    for line in Path(path).read_text(encoding="utf-8").splitlines():
        parts = line.split()
        if len(parts) == 2:
            sums[parts[1].lstrip("*")] = parts[0]
    return sums


def release_base(version, base_url):
    return (base_url or f"https://github.com/{REPO}/releases/download/v{version}").rstrip("/")


def archive_name(version, target):
    ext = "zip" if "windows" in target else "tar.gz"
    return f"cross-control-{version}-{target}.{ext}"


HOMEBREW_TARGETS = [
    ("on_macos", "on_arm", "aarch64-apple-darwin"),
    ("on_macos", "on_intel", "x86_64-apple-darwin"),
    ("on_linux", "on_arm", "aarch64-unknown-linux-gnu"),
    ("on_linux", "on_intel", "x86_64-unknown-linux-gnu"),
]


def homebrew_formula(version, sums, base_url=None, partial=False):
    base = release_base(version, base_url)
    blocks = {}
    for system, cpu, target in HOMEBREW_TARGETS:
        name = archive_name(version, target)
        if name not in sums:
            if partial:
                continue
            die(f"{name} is not in the checksums")
        blocks.setdefault(system, []).append(
            f"    {cpu} do\n"
            f'      url "{base}/{name}"\n'
            f'      sha256 "{sums[name]}"\n'
            f"    end\n"
        )
    if not blocks:
        die("no Homebrew targets in the checksums")
    platforms = "".join(f"  {system} do\n{''.join(cpus)}  end\n\n" for system, cpus in blocks.items())
    return f'''# Generated by scripts/release/release.py for each release; edit the generator.
class CrossControl < Formula
  desc "{DESCRIPTION}"
  homepage "{HOMEPAGE}"
  version "{version}"
  license any_of: ["MIT", "Apache-2.0"]

{platforms}  def install
    bin.install "cross-control"
    pkgshare.install "config.example.toml"
  end

  def caveats
    <<~EOS
      Example configuration: #{{opt_pkgshare}}/config.example.toml
      Start at login: cross-control service install --enable

      On macOS, grant #{{opt_bin}}/cross-control both Input Monitoring and
      Accessibility in System Settings > Privacy & Security, then run
      `cross-control doctor`. macOS may ask again after an upgrade.

      On Linux, your user must be in the input group and /dev/uinput writable;
      `cross-control doctor` shows how.
    EOS
  end

  test do
    assert_match "cross-control #{{version}}", shell_output("#{{bin}}/cross-control --version")
    ENV["CROSS_CONTROL_CONFIG_DIR"] = (testpath/"config").to_s
    ENV["CROSS_CONTROL_RUNTIME_DIR"] = (testpath/"run").to_s
    assert_match "SHA256:", shell_output("#{{bin}}/cross-control identity --json")
  end
end
'''


def scoop_manifest(version, sums, base_url=None):
    target = "x86_64-pc-windows-msvc"
    name = archive_name(version, target)
    if name not in sums:
        die(f"{name} is not in the checksums")
    base = release_base(version, base_url)
    manifest = {
        "##": "Generated by scripts/release/release.py for each release; edit the generator.",
        "version": version,
        "description": DESCRIPTION + ".",
        "homepage": HOMEPAGE,
        "license": "MIT|Apache-2.0",
        "notes": [
            "Start at login: cross-control service install --enable",
            "Check this machine: cross-control doctor",
        ],
        "architecture": {"64bit": {"url": f"{base}/{name}", "hash": sums[name]}},
        "extract_dir": f"cross-control-{version}-{target}",
        "bin": ["cross-control.exe", "cross-controlw.exe"],
        "checkver": {"github": f"https://github.com/{REPO}"},
        "autoupdate": {
            "architecture": {
                "64bit": {
                    "url": f"https://github.com/{REPO}/releases/download/v$version/"
                    f"cross-control-$version-{target}.zip"
                }
            },
            "hash": {"url": "$baseurl/SHA256SUMS"},
            "extract_dir": f"cross-control-$version-{target}",
        },
    }
    return json.dumps(manifest, indent=4) + "\n"


def cmd_homebrew(args):
    sys.stdout.write(homebrew_formula(args.version, read_sums(args.sums), args.base_url, args.partial))


def cmd_scoop(args):
    sys.stdout.write(scoop_manifest(args.version, read_sums(args.sums), args.base_url))


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

    p = sub.add_parser("linux-packages")
    p.add_argument("--target", required=True)
    p.add_argument("--version", required=True)
    p.add_argument("--bin-dir", required=True)
    p.add_argument("--out", required=True)
    p.add_argument("--nfpm", default="nfpm", help="the nfpm executable")
    p.set_defaults(func=cmd_linux_packages)

    for name, func in (("homebrew", cmd_homebrew), ("scoop", cmd_scoop)):
        p = sub.add_parser(name)
        p.add_argument("--version", required=True)
        p.add_argument("--sums", required=True, help="the release's SHA256SUMS file")
        p.add_argument("--base-url", help="where the archives are (default: the GitHub release)")
        if name == "homebrew":
            p.add_argument("--partial", action="store_true", help="leave out targets missing from the checksums")
        p.set_defaults(func=func)

    p = sub.add_parser("bump")
    p.add_argument("version")
    p.add_argument("--date", help="release date for the CHANGELOG heading (default: today)")
    p.set_defaults(func=cmd_bump)

    args = parser.parse_args()
    args.func(args)


if __name__ == "__main__":
    os.chdir(ROOT)
    main()
