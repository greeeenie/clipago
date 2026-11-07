import {createHash} from 'node:crypto';
import {readFileSync, readdirSync, writeFileSync} from 'node:fs';
import {fileURLToPath} from 'node:url';
import {join} from 'node:path';

const root = fileURLToPath(new URL('../', import.meta.url));
const dist = join(root, 'web-dist');
const stamp = 'clipago-ui-host.txt';
const config = JSON.parse(readFileSync(join(root, 'tauri.conf.json'), 'utf8'));

function files(directory) {
  return readdirSync(directory, {withFileTypes: true}).flatMap(entry =>
    entry.isDirectory()
      ? files(join(directory, entry.name)).map(name => entry.name + '/' + name)
      : [entry.name]
  );
}

const [command, output] = process.argv.slice(2);
if (command === 'stamp') {
  const hash = createHash('sha256');
  const native = ['build.rs', 'Cargo.toml', 'Cargo.lock', 'tauri.conf.json', ...config.bundle.icon];
  for (const dir of ['src', 'capabilities', 'windows']) {
    native.push(...files(join(root, dir)).map(name => dir + '/' + name));
  }
  for (const name of native.sort()) {
    let bytes = readFileSync(join(root, name));
    if (!config.bundle.icon.includes(name)) {
      let source = bytes.toString('utf8').replaceAll('\r\n', '\n');
      if (name === 'Cargo.toml') source = source.replace(/^version = "[^"]+"/m, 'version = "0"');
      if (name === 'Cargo.lock') source = source.replace(/(name = "rewind"\n)version = "[^"]+"/, '$1version = "0"');
      if (name === 'tauri.conf.json') source = JSON.stringify({...config, version: undefined});
      bytes = Buffer.from(source);
    }
    hash.update(JSON.stringify([name, bytes.length])).update(bytes);
  }
  writeFileSync(join(dist, stamp), hash.digest('hex'));
} else if (command === 'pack' && output) {
  const host = readFileSync(join(dist, stamp), 'utf8');
  if (!/^[a-f0-9]{64}$/.test(host)) throw new Error('Invalid UI compatibility stamp');
  const assets = Object.fromEntries(files(dist).filter(name => name !== stamp).sort().map(name =>
    [name, readFileSync(join(dist, name)).toString('base64')]
  ));
  writeFileSync(output, JSON.stringify({host, version: config.version, files: assets}));
} else {
  throw new Error('Use stamp or pack <output>');
}
