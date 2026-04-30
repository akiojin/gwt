const { spawn } = require("child_process");
const path = require("path");
const fs = require("fs");

const {
  bundleBinaryNamesForPlatform,
  installReleaseBinary,
  releaseAssetUrl,
} = require("../scripts/release-assets.cjs");

const REPO = "akiojin/gwt";
const BIN_DIR = __dirname;

function readVersion() {
  const pkg = path.join(__dirname, "..", "package.json");
  return JSON.parse(fs.readFileSync(pkg, "utf8")).version;
}

function createLauncher({ commandName, binaryNameForPlatform }) {
  const binName = binaryNameForPlatform(process.platform);
  const binPath = path.join(BIN_DIR, binName);
  const bundleBinaries = bundleBinaryNamesForPlatform(process.platform);

  async function ensureBinary() {
    if (bundleBinaries.every((name) => fs.existsSync(path.join(BIN_DIR, name)))) {
      return;
    }

    const version = readVersion();
    const { url } = releaseAssetUrl(REPO, version, process.platform, process.arch);

    console.log(`Downloading gwt bundle for ${process.platform}-${process.arch}...`);
    console.log(`Downloading from: ${url}`);

    await installReleaseBinary({
      repo: REPO,
      version,
      binDir: BIN_DIR,
      platform: process.platform,
      arch: process.arch,
    });

    console.log(`gwt bundle installed successfully: ${bundleBinaries.join(", ")}`);
  }

  async function main() {
    try {
      await ensureBinary();
    } catch (err) {
      console.error(`Failed to download gwt binary: ${err.message}`);
      console.error(`https://github.com/${REPO}/releases`);
      process.exit(1);
    }

    const child = spawn(binPath, process.argv.slice(2), {
      stdio: "inherit",
      env: process.env,
    });

    child.on("error", (err) => {
      console.error(`Failed to start ${commandName}: ${err.message}`);
      process.exit(1);
    });

    child.on("exit", (code, signal) => {
      if (signal) {
        process.kill(process.pid, signal);
      } else {
        process.exit(code ?? 0);
      }
    });
  }

  return {
    ensureBinary,
    main,
    readVersion,
  };
}

module.exports = {
  createLauncher,
  readVersion,
};
