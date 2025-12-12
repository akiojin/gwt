#!/usr/bin/env node
import('../dist/index.js').then(module => {
  if (module.main) {
    return module.main();
  } else {
    throw new Error('main function not found in index.js');
  }
}).catch(err => {
  console.error(err);
  process.exit(1);
});
