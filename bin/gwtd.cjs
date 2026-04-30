#!/usr/bin/env node
/**
 * Wrapper script to execute the gwtd Rust binary.
 * If the bundle is not found (e.g., bunx skips postinstall),
 * it will be downloaded on-demand from GitHub Releases.
 */

const { daemonBinaryNameForPlatform } = require("../scripts/release-assets.cjs");
const { createLauncher } = require("./launcher.cjs");

const launcher = createLauncher({
  commandName: "gwtd",
  binaryNameForPlatform: daemonBinaryNameForPlatform,
});

if (require.main === module) {
  launcher.main();
}

module.exports = {
  ensureBinary: launcher.ensureBinary,
  main: launcher.main,
  readVersion: launcher.readVersion,
};
