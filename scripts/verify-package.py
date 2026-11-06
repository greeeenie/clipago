"""Check the packaged executable and real UI screenshot artifacts."""
import hashlib
import struct
from pathlib import Path

root = Path(__file__).resolve().parents[1]
exe = root / 'dist/Clipago.exe'
assert exe.read_bytes() == (root / 'target/release/rewind.exe').read_bytes(), 'Packaged EXE differs from release build'
for name in ['main', 'export', 'settings', 'quick-clip', 'maximized']:
    path = root / f'dist/screenshots/{name}.png'
    if not path.exists():
        path = path.with_suffix('.jpg')
    data = path.read_bytes()
    if data[:8] == b'\x89PNG\r\n\x1a\n':
        width, height = struct.unpack('>II', data[16:24])
    else:
        assert data[:2] == b'\xff\xd8', f'{name}: unsupported screenshot format'
        index = 2
        while index < len(data):
            assert data[index] == 255
            while data[index] == 255:
                index += 1
            marker = data[index]
            index += 1
            length = int.from_bytes(data[index:index+2], 'big')
            if marker in [0xc0, 0xc1, 0xc2]:
                height, width = struct.unpack('>HH', data[index+3:index+7])
                break
            index += length
        else:
            raise AssertionError(f'{name}: missing JPEG dimensions')
        if path.suffix != '.jpg':
            path.rename(path.with_suffix('.jpg'))
        report = root / 'dist/VERIFICATION.md'
        report.write_text(report.read_text(encoding='utf-8').replace(f'screenshots/{name}.png', f'screenshots/{name}.jpg'), encoding='utf-8')
    assert width >= 420 and height >= 300, f'{name}: invalid screenshot {width}x{height}'
    print(f'{name}: {width}x{height}')
checksum = hashlib.sha256(exe.read_bytes()).hexdigest()
(root / 'dist/Clipago.sha256').write_text(f'{checksum}  Clipago.exe\n', encoding='ascii')
print(f'EXE: {exe.stat().st_size:,} bytes; SHA-256: {checksum}')
