const { spawnSync } = require('child_process');
const path = require('path');

function run(command, args, options = {}) {
  const result = spawnSync(command, args, { stdio: 'inherit', ...options });
  if (result.status !== 0) {
    process.exit(result.status || 1);
  }
}

const root = path.resolve(__dirname, '..', '..');
const platform = process.platform;

if (platform === 'win32') {
  const script = path.join(root, 'scripts', 'packaging', 'windows', 'build-installer.ps1');
  run('powershell', ['-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', script]);
} else if (platform === 'darwin') {
  const script = path.join(root, 'scripts', 'packaging', 'mac', 'build-installer.sh');
  run('bash', [script]);
} else {
  const script = path.join(root, 'scripts', 'packaging', 'linux', 'build-package.sh');
  run('bash', [script]);
}
