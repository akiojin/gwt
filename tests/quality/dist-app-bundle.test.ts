import { access, readFile } from 'node:fs/promises';
import { constants } from 'node:fs';
import { resolve } from 'node:path';
import { describe, expect, it } from 'vitest';

const bundlePath = resolve(process.cwd(), 'dist/ui/components/App.js');

describe('Dist bundle integrity', () => {
  it('reflects BranchList cleanup UI implementation', async () => {
    await expect(access(bundlePath, constants.F_OK)).resolves.toBeUndefined();

    const content = await readFile(bundlePath, 'utf8');

    expect(content).toContain('cleanupIndicators');
    expect(content).not.toMatch(/PRCleanupScreen/);
  });
});
