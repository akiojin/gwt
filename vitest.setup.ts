import '@testing-library/jest-dom';

// Ensure jsdom is loaded
if (typeof document === 'undefined') {
  // Force jsdom environment
  const { JSDOM } = require('jsdom');
  const dom = new JSDOM('<!DOCTYPE html><html><body></body></html>', {
    url: 'http://localhost',
  });
  global.document = dom.window.document;
  global.window = dom.window as any;
}
