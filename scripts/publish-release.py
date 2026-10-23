"""Publish one complete, signed artifact set; never overwrite a release."""
import argparse
import hashlib
import json
import re
import subprocess
import tempfile
from pathlib import Path


def expected_assets(kind, version):
    targets = ['x86_64-unknown-linux-gnu', 'aarch64-unknown-linux-gnu',
               'x86_64-apple-darwin', 'aarch64-apple-darwin', 'x86_64-pc-windows-msvc']
    prefix = {'server': 'glyph-server', 'apps': 'glyph', 'rdlp': 'r-dlp'}[kind]
    names = {f'{prefix}-{version}-{target}.{"zip" if "windows" in target else "tar.gz"}'
             for target in targets}
    if kind == 'server':
        names.add(f'glyph-server-{version}.tgz')
    elif kind == 'apps':
        names.update({f'glyph-{version}-windows-x64-setup.exe', f'glyph-{version}-android.apk'})
        names.update(f'glyph-{version}-macos-{arch}.dmg' for arch in ['x64', 'arm64'])
        names.update(f'glyph-{version}-linux-{arch}.{ext}'
                     for arch in ['x64', 'arm64'] for ext in ['deb', 'rpm'])
    return names


def digest(file):
    with file.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def manifest(directory, expected):
    files = {p.name for p in directory.iterdir() if p.is_file()}
    if files != expected | {name + '.sha256' for name in expected}:
        raise ValueError('Release artifact inventory does not match the expected platform set')
    lines = []
    for name in sorted(expected):
        match = re.fullmatch(r'([a-fA-F0-9]{64})[ \t]+\*?([^\r\n]+)\r?\n?',
                             (directory / (name + '.sha256')).read_text())
        if not match or match[2] != name or match[1].lower() != digest(directory / name):
            raise ValueError(f'Release checksum mismatch: {name}')
        lines.append(f'{match[1].lower()}  {name}\n')
    return ''.join(lines)


def gh(*args):
    return subprocess.check_output(['gh', *args], text=True)


def require_unpublished(release):
    if not release.get('draft'):
        raise ValueError('Refusing to modify an already published release; use a new version')
    if release.get('assets'):
        raise ValueError('Draft already contains assets; review the interrupted release before retrying')


def publish(kind, tag, repo, directory):
    if not re.fullmatch(r'v\d+\.\d+\.\d+(?:-[A-Za-z0-9.-]+)?', tag):
        raise ValueError('Invalid release tag')
    expected = expected_assets(kind, tag[1:])
    content = manifest(directory, expected)
    sums = directory / 'SHA256SUMS.txt'
    sums.write_text(content, encoding='utf-8', newline='\n')
    subprocess.run(['sh', 'scripts/sign-release.sh', str(sums), repo, tag], check=True)
    # API absence must be a genuine 404, never an authentication/network error.
    lookup = subprocess.run(['gh', 'api', f'repos/{repo}/releases/tags/{tag}'],
                            text=True, capture_output=True)
    if lookup.returncode == 0:
        require_unpublished(json.loads(lookup.stdout))
    elif 'HTTP 404' not in lookup.stderr:
        raise RuntimeError(lookup.stderr)
    else:
        gh('release', 'create', tag, '--repo', repo, '--verify-tag', '--draft',
           '--notes-file', 'RELEASE_NOTES.md', *(['--prerelease'] if '-' in tag else []))
    # No clobber: concurrent or resumed publishers fail closed.
    uploads = sorted(expected | {'SHA256SUMS.txt', 'SHA256SUMS.txt.minisig'})
    gh('release', 'upload', tag, '--repo', repo, *(str(directory / n) for n in uploads))
    with tempfile.TemporaryDirectory(prefix='glyph-release-verify-') as tmp:
        gh('release', 'download', tag, '--repo', repo, '--dir', tmp)
        downloaded = Path(tmp)
        if {p.name for p in downloaded.iterdir()} != set(uploads):
            raise ValueError('Remote release inventory differs from the verified build')
        for name in uploads:
            if digest(downloaded / name) != digest(directory / name):
                raise ValueError(f'Remote release checksum mismatch: {name}')
    gh('release', 'edit', tag, '--repo', repo, '--draft=false',
       '--latest=false' if '-' in tag else '--latest')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('kind', choices=['server', 'apps', 'rdlp'])
    parser.add_argument('tag')
    parser.add_argument('repo')
    parser.add_argument('directory', type=Path)
    args = parser.parse_args()
    publish(args.kind, args.tag, args.repo, args.directory.resolve())
