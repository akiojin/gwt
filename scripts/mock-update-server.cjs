#!/usr/bin/env node
/**
 * SPEC-2041 Phase 19 Gate 2 mock smoke server.
 *
 * Drives the update-modal flow against a synthetic GitHub Releases endpoint
 * so the post-click flow (downloading -> ready -> Restart now / Later) can be
 * exercised without publishing real releases. Combine with the
 * `GWT_UPDATE_API_BASE_URL` env override that landed alongside this script.
 *
 * Usage:
 *   node scripts/mock-update-server.cjs --port 18080 --version 99.99.0 \
 *     [--asset path/to/gwt-macos-arm64.tar.gz]
 *
 * Then in another shell:
 *   GWT_UPDATE_API_BASE_URL=http://127.0.0.1:18080 ./target/release/gwt
 *
 * The server returns a synthetic release at
 *   GET /repos/akiojin/gwt/releases/latest
 * that points `browser_download_url` at the mock's own /assets/<filename>
 * endpoint so the prepare path can both observe progress and (when --asset is
 * a real tarball) succeed the restart-now path. Without --asset the download
 * still completes but extraction fails — useful for smoking the failure
 * modal (`update_apply_error` with stage="Download asset" or
 * stage="Persist pending").
 */

const http = require("http");
const fs = require("fs");
const path = require("path");

const args = process.argv.slice(2);
const opts = {
  port: 18080,
  version: "99.99.0",
  asset: null,
};
for (let i = 0; i < args.length; i++) {
  const flag = args[i];
  const value = args[i + 1];
  if (flag === "--port") {
    opts.port = parseInt(value, 10);
    i += 1;
  } else if (flag === "--version") {
    opts.version = value;
    i += 1;
  } else if (flag === "--asset") {
    opts.asset = value;
    i += 1;
  } else if (flag === "--help" || flag === "-h") {
    printUsage();
    process.exit(0);
  }
}

function printUsage() {
  console.log(`SPEC-2041 Phase 19 Gate 2 mock smoke server

Usage:
  node scripts/mock-update-server.cjs [--port PORT] [--version VERSION] [--asset PATH]

Options:
  --port      TCP port to bind on 127.0.0.1 (default: 18080)
  --version   Tag name to advertise as the latest release (default: 99.99.0)
  --asset     Local path to a real tarball/zip to serve as the asset.
              When omitted, the server returns a 32-byte payload so the
              download stage succeeds and extraction surfaces a failure
              modal -- handy for smoking the failure UX.

Then run gwt with:
  GWT_UPDATE_API_BASE_URL=http://127.0.0.1:PORT ./target/release/gwt
`);
}

let assetBuffer;
// Always advertise the canonical asset name for this platform so the updater's
// release-contract matcher (`scripts/release-assets.cjs::releaseAssetName`)
// finds the asset regardless of the local file the operator points at.
const assetName = pickAssetName(process.platform, process.arch);
if (opts.asset) {
  if (!fs.existsSync(opts.asset)) {
    console.error(`[mock] --asset path does not exist: ${opts.asset}`);
    process.exit(1);
  }
  assetBuffer = fs.readFileSync(opts.asset);
} else {
  // 32 bytes of random payload so the download progress callback fires at
  // least once and the persist path runs. The eventual extract_archive will
  // fail with `Invalid gzip` / `not a tar archive`, exercising the failure
  // modal.
  assetBuffer = Buffer.alloc(32, 0x5a);
}

function pickAssetName(platform, arch) {
  if (platform === "darwin" && arch === "arm64") return "gwt-macos-arm64.tar.gz";
  if (platform === "darwin" && (arch === "x64" || arch === "x86_64"))
    return "gwt-macos-x86_64.tar.gz";
  if (platform === "linux" && arch === "x64") return "gwt-linux-x86_64.tar.gz";
  if (platform === "linux" && arch === "arm64")
    return "gwt-linux-aarch64.tar.gz";
  if (platform === "win32") return "gwt-windows-x86_64.zip";
  return "gwt-update.tar.gz";
}

const server = http.createServer((req, res) => {
  const url = new URL(req.url, `http://${req.headers.host}`);
  console.log(`[mock] ${req.method} ${url.pathname}`);

  if (
    url.pathname.startsWith("/repos/") &&
    url.pathname.endsWith("/releases/latest")
  ) {
    const baseUrl = `http://127.0.0.1:${opts.port}`;
    // Strip a single leading `v` from --version so passing either `9.26.0` or
    // `v9.26.0` yields the canonical `vN.N.N` tag the updater's
    // `parse_tag_version` accepts (it tolerates exactly one `v` prefix).
    const normalizedVersion = String(opts.version).replace(/^v/, "");
    const release = {
      tag_name: `v${normalizedVersion}`,
      html_url: `${baseUrl}/release-page`,
      assets: [
        {
          name: assetName,
          browser_download_url: `${baseUrl}/assets/${assetName}`,
        },
      ],
    };
    res.writeHead(200, { "Content-Type": "application/json" });
    res.end(JSON.stringify(release));
    return;
  }

  if (url.pathname.startsWith("/assets/")) {
    res.writeHead(200, {
      "Content-Type": "application/octet-stream",
      "Content-Length": String(assetBuffer.length),
    });
    res.end(assetBuffer);
    return;
  }

  res.writeHead(404, { "Content-Type": "text/plain" });
  res.end("Not Found");
});

server.listen(opts.port, "127.0.0.1", () => {
  const advertisedTag = `v${String(opts.version).replace(/^v/, "")}`;
  console.log(
    `[mock] update server listening on http://127.0.0.1:${opts.port}`,
  );
  console.log(`[mock] advertising tag ${advertisedTag} (asset: ${assetName})`);
  console.log("[mock] run gwt with:");
  console.log(
    `  GWT_UPDATE_API_BASE_URL=http://127.0.0.1:${opts.port} ./target/release/gwt`,
  );
  console.log("[mock] press ctrl-c to stop.");
});
