import { describe, it, expect } from 'vitest';
import { createRequire } from 'node:module';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const ADDON = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  '../../native/backdrop/build/Release/backdrop.node',
);

// The addon's argument checks, exercised from plain node: without a window
// nothing can be attached, so what is left to pin is that a wrong argument
// is refused up front — with NAPI_DISABLE_CPP_EXCEPTIONS a cast of the wrong
// type leaves a pending exception and a garbage value, and a call that
// carried on with it would still "succeed" at placing the view.
describe.skipIf(process.platform !== 'darwin')('backdrop addon arguments', () => {
  const addon = createRequire(import.meta.url)(ADDON);

  it('refuses a handle that is not a buffer', () => {
    expect(() => addon.attach('nope', 0, 0, 480, 130, 16)).toThrow(TypeError);
  });

  it('refuses a handle that does not hold a view', () => {
    expect(() => addon.attach(Buffer.alloc(8), 0, 0, 480, 130, 16)).toThrow(TypeError);
  });

  it('refuses a rect that is not numbers, before touching the window', () => {
    expect(() => addon.attach(Buffer.alloc(8), 'x', 0, 480, 130, 16)).toThrow(TypeError);
    expect(() => addon.setFrame('x', 0, 480, 130)).toThrow(TypeError);
  });

  it('has nothing to show, hide or move with no window attached', () => {
    expect(addon.state()).toBeNull();
    expect(() => addon.detach()).not.toThrow();
    expect(() => addon.setFrame(0, 0, 480, 130)).not.toThrow();
    expect(addon.state()).toBeNull();
  });
});
