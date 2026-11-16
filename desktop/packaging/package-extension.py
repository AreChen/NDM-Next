#!/usr/bin/env python3
"""Package the exact companion extension belonging to a desktop release tag."""
import argparse
import base64
import hashlib
import json
from pathlib import Path
import re
import zipfile


def package(output: Path, tag: str | None = None) -> Path:
    repo = Path(__file__).resolve().parents[2]
    cargo = (repo / 'desktop/Cargo.toml').read_text()
    version = re.search(r'^version = "([0-9]+\.[0-9]+\.[0-9]+)"$', cargo, re.M)[1]
    if tag is not None and tag != f'desktop-v{version}':
        raise ValueError('Release tag does not match the desktop version')
    extension = repo / 'extension'
    manifest = json.loads((extension / 'manifest.json').read_text())
    key = base64.b64decode(manifest['key'], validate=True)
    extension_id = ''.join(chr(ord('a') + int(n, 16)) for n in hashlib.sha256(key).hexdigest()[:32])
    for registration in ['desktop/crates/ndm-host/src/macos.rs', 'desktop/packaging/windows/ndm-next.nsi']:
        if f'"{extension_id}"' not in (repo / registration).read_text(encoding='utf-8-sig'):
            raise ValueError(f'Unpacked extension ID missing from {registration}')
    output.mkdir(parents=True, exist_ok=True)
    archive = output / f'NDM-Next-Extension-Desktop-{version}-v{manifest["version"]}.zip'
    files = {str(path.relative_to(extension)): path for path in sorted(extension.rglob('*')) if path.is_file()}
    files['INSTALL.md'] = repo / 'docs/extension-install.md'
    files['enable-dev-extension.ps1'] = repo / 'desktop/packaging/windows/enable-dev-extension.ps1'
    with zipfile.ZipFile(archive, 'w', compression=zipfile.ZIP_DEFLATED) as zipped:
        for name, source in files.items():
            if source.is_symlink() or source.name.startswith('.'):
                raise ValueError(f'Unexpected extension file: {source}')
            # Stable timestamps and permissions keep the ZIP reproducible.
            info = zipfile.ZipInfo(name, date_time=(2020, 1, 1, 0, 0, 0))
            info.compress_type = zipfile.ZIP_DEFLATED
            info.external_attr = 0o100644 << 16
            zipped.writestr(info, source.read_bytes())
    with zipfile.ZipFile(archive) as zipped:
        assert zipped.testzip() is None
        for name, source in files.items():
            if zipped.read(name) != source.read_bytes():
                raise ValueError(f'Packaged extension differs from source: {name}')
    digest = hashlib.sha256(archive.read_bytes()).hexdigest()
    archive.with_suffix('.zip.sha256').write_text(f'{digest}  {archive.name}\n')
    print(f'Extension {manifest["version"]}: {extension_id}')
    print(archive)
    return archive


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('output', type=Path)
    parser.add_argument('--tag')
    args = parser.parse_args()
    package(args.output, args.tag)
