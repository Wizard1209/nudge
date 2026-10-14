import { execFileSync, spawn } from 'node:child_process';
import { mkdtempSync } from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const SOURCE = path.join(path.dirname(fileURLToPath(import.meta.url)), 'macos-input.swift');

// Compiles macos-input.swift once per test file into a temp dir and wraps its
// commands. The binary posts real CGEvents, so the process running the tests
// needs the Accessibility permission (the terminal's, when run from one).
export function macosInput() {
  const bin = path.join(mkdtempSync(path.join(os.tmpdir(), 'nudge-input-')), 'macos-input');
  execFileSync('swiftc', ['-O', '-o', bin, SOURCE], { stdio: 'inherit' });
  const run = (...args) => execFileSync(bin, args.map(String), { encoding: 'utf8' });
  return {
    click: (x, y) => run('click', x, y),
    key: (code, ...mods) => run('key', code, ...mods),
    frontPid: () => Number(run('front').trim()),
    locked: () => run('locked').trim() === '1',
    windows: () => run('windows').trim().split('\n').filter(Boolean).map((l) => JSON.parse(l)),
    /** Re-activate `bundleId` every `ms` until the returned stop() is called. */
    contend(bundleId, ms) {
      const child = spawn(bin, ['activate', bundleId, String(ms)], { stdio: 'ignore' });
      return () => new Promise((resolve) => {
        if (child.exitCode !== null) return resolve();
        child.once('exit', resolve);
        child.kill('SIGTERM');
      });
    },
  };
}

export const KEY = { esc: 53, tab: 48, space: 49 };
