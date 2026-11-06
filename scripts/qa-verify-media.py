"""Validate the actual clips produced by the isolated desktop UI checks."""
import json
import subprocess
from pathlib import Path

root = Path(__file__).resolve().parents[1]
clips = root / 'target/ui-qa-react/profile/clips'
results = []
for path in sorted(clips.glob('Clip_*.mp4')):
    data = json.loads(subprocess.check_output([
        'ffprobe', '-v', 'error', '-show_streams', '-show_format', '-of', 'json', str(path)
    ]))
    video = next(stream for stream in data['streams'] if stream['codec_type'] == 'video')
    duration = float(data['format']['duration'])
    decoded = subprocess.run([
        'ffmpeg', '-v', 'error', '-nostdin', '-i', str(path), '-map', '0:v:0', '-f', 'null', '-'
    ], capture_output=True, check=True)
    assert not decoded.stderr, decoded.stderr.decode(errors='replace')
    assert duration > .05
    results.append({'file': path.name, 'duration': duration, 'width': video['width'],
                    'height': video['height'], 'codec': video['codec_name'], 'decode': 'passed'})
assert any(abs(item['duration'] - 9.8) < .05 for item in results), 'Missing quick trim result'
assert any('_edit' in item['file'] for item in results), 'Missing UI export result'
print(json.dumps(results, ensure_ascii=False, indent=2))
(root / 'dist/media-checks.json').write_text(json.dumps(results, ensure_ascii=False, indent=2), encoding='utf-8')
