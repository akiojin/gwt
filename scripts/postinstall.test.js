import { test } from 'node:test';
import assert from 'node:assert/strict';

import {
  RETRY_CONFIG,
  buildReleaseDownloadUrl,
  formatFailureGuidance,
  getRetryDelayMs,
  isRetryableError,
  isRetryableStatus,
} from './postinstall.js';

test('buildReleaseDownloadUrl uses version tag', () => {
  const url = buildReleaseDownloadUrl('6.17.0', 'gwt-linux-x86_64');
  assert.equal(
    url,
    'https://github.com/akiojin/gwt/releases/download/v6.17.0/gwt-linux-x86_64',
  );
});

test('getRetryDelayMs applies exponential backoff with cap', () => {
  assert.equal(getRetryDelayMs(1, RETRY_CONFIG), 500);
  assert.equal(getRetryDelayMs(2, RETRY_CONFIG), 1000);
  assert.equal(getRetryDelayMs(3, RETRY_CONFIG), 2000);
  assert.equal(getRetryDelayMs(5, RETRY_CONFIG), 5000);
});

test('isRetryableStatus matches 403/404/5xx', () => {
  assert.equal(isRetryableStatus(403), true);
  assert.equal(isRetryableStatus(404), true);
  assert.equal(isRetryableStatus(500), true);
  assert.equal(isRetryableStatus(429), false);
  assert.equal(isRetryableStatus(400), false);
});

test('isRetryableError uses statusCode or treats network errors as retryable', () => {
  const notFound = new Error('not found');
  notFound.statusCode = 404;
  assert.equal(isRetryableError(notFound), true);

  const badRequest = new Error('bad request');
  badRequest.statusCode = 400;
  assert.equal(isRetryableError(badRequest), false);

  const networkError = new Error('socket hang up');
  assert.equal(isRetryableError(networkError), true);
});

test('formatFailureGuidance includes versioned URL and release page', () => {
  const guidance = formatFailureGuidance('1.2.3', 'gwt-linux-x86_64');
  assert.equal(
    guidance.versionedUrl,
    'https://github.com/akiojin/gwt/releases/download/v1.2.3/gwt-linux-x86_64',
  );
  assert.equal(guidance.releasePageUrl, 'https://github.com/akiojin/gwt/releases');
  assert.equal(guidance.buildCommand, 'cargo build --release');
});
