#!/usr/bin/env python3
"""Build the native macOS application without installing it."""
import hashlib
import os
from pathlib import Path
import plistlib
import shutil
import subprocess
import tomllib

root = Path(__file__).resolve().parents[2]
subprocess.run(['cargo', 'build', '--locked', '--release', '-p', 'zebar-native'], cwd=root, check=True)
target = Path(os.environ.get('CARGO_TARGET_DIR', root / 'target')).resolve()
binary = target / 'release/zebar-native'
version = tomllib.loads((root / 'packages/native-macos/Cargo.toml').read_text())['package']['version']
output = root / 'target/native.noindex' / ('build-' + hashlib.sha256(binary.read_bytes()).hexdigest()[:16])
app = output / 'Zebar.app'
(app / 'Contents/MacOS').mkdir(parents=True, exist_ok=True)
(app / 'Contents/Resources').mkdir(exist_ok=True)
shutil.copy2(binary, app / 'Contents/MacOS/zebar')
icon = root / 'packages/desktop/resources/icons/icon.icns'
shutil.copy2(icon, app / 'Contents/Resources/icon.icns')
info = {
    'CFBundleIdentifier': 'com.glzr.zebar', 'CFBundleName': 'Zebar',
    'CFBundleDisplayName': 'Zebar', 'CFBundleExecutable': 'zebar',
    'CFBundlePackageType': 'APPL', 'CFBundleIconFile': 'icon.icns',
    'CFBundleShortVersionString': version, 'CFBundleVersion': '20261111.3',
    'LSUIElement': True, 'LSMinimumSystemVersion': '14.6',
    'NSHighResolutionCapable': True,
    'NSAppleEventsUsageDescription': 'Displays and controls playback in Music and Spotify.',
}
with (app / 'Contents/Info.plist').open('wb') as file:
    plistlib.dump(info, file)
subprocess.run(['codesign', '--force', '--deep', '--sign', '-', str(app)], check=True)
subprocess.run(['codesign', '--verify', '--deep', str(app)], check=True)
assert 'native Rust/AppKit' in subprocess.check_output([str(app / 'Contents/MacOS/zebar'), '--version'], text=True)
print(app)
