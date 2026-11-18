const { execFileSync, fork } = require("node:child_process");
const { once } = require("node:events");
const { dirname, join } = require("node:path");
const { setTimeout: delay } = require("node:timers/promises");

exports.watchExactRelaunchProcesses = async function (home, capacityExecutable) {
  // verify.run may SIGKILL the worker's entire group. A separate group and
  // an IPC disconnect let cleanup run even when no JS finally can execute.
  const guard = fork(__filename, [home, capacityExecutable], {
    detached: true, execArgv: [], stdio: ["ignore", "ignore", "inherit", "ipc"],
    cwd: dirname(process.execPath),
    env: { PATH: process.env.PATH, LC_ALL: "C" },
  });
  const finished = new Promise(resolve => {
    guard.once("error", resolve);
    guard.once("exit", (code, signal) => resolve(code === 0 ? undefined
      : new Error(`Exact relaunch cleanup exited with ${signal ?? code}`)));
  });
  await Promise.race([once(guard, "message"), finished.then(error => {
    throw error ?? new Error("Exact relaunch cleanup exited before readiness");
  })]);
  guard.unref();
  guard.channel.unref();
  return async () => {
    guard.ref();
    if (guard.connected) guard.disconnect();
    const error = await finished;
    if (error) throw error;
  };
};

function processes(selection) {
  let output;
  try {
    output = execFileSync("ps", ["-ww", ...selection, "-o", "pid=", "-o", "ppid=", "-o", "stat=", "-o", "lstart=", "-o", "command="], {
      encoding: "utf8", maxBuffer: 16 * 1024 * 1024, timeout: 5_000,
    });
  } catch (error) {
    if (selection[0] === "-p" && error.status === 1 && !error.stdout?.trim() && !error.stderr?.trim()) return [];
    throw error;
  }
  return output.split("\n").flatMap(line => {
    const match = line.match(/^\s*(\d+)\s+(\d+)\s+(\S+)\s+((?:\S+\s+){4}\d{4})\s+(.*)$/);
    if (!match || /^[ZX]/.test(match[3])) return [];
    return [{ pid: Number(match[1]), parent: Number(match[2]), birth: match[4], command: match[5] }];
  });
}

async function reap(home, capacityExecutable) {
  const bin = join(home, "bin") + "/";
  // The GUI's argv[0], daemon path, and fake provider scripts carry private
  // fixture paths. They remain identifiable after the fixture cwd is deleted.
  const tags = [`${bin}gwt --no-tray --no-open`, `${bin}gwtd`, `${capacityExecutable} ${join(home, "project")}`];
  const node = process.execPath + " ";
  const ownedPath = command => tags.some(tag => command === tag || command.startsWith(tag + " "))
    || command.startsWith(node + bin) || command.startsWith("node " + bin)
    || command === `${node}-e setInterval(()=>{},1000) GWT_SWITCH_TREE_CHILD ${home}`;
  const observed = new Map();
  const termSent = new Map();
  const started = performance.now();
  let empty = false;
  while (performance.now() - started < 10_000) {
    const snapshot = processes(["-U", String(process.getuid())]);
    const byPid = new Map(snapshot.map(entry => [entry.pid, entry]));
    const owned = entry => {
      for (let ancestor = entry; ancestor?.pid > 0; ancestor = byPid.get(ancestor.parent)) {
        const previous = observed.get(ancestor.pid);
        if (ownedPath(ancestor.command) || (previous?.birth === ancestor.birth && previous.command === ancestor.command)) return true;
      }
      return false;
    };
    // Capture descendants while their parent is alive, before it is signalled.
    const live = snapshot.filter(owned);
    if (!live.length && empty) return;
    empty = !live.length;
    for (const entry of live) observed.set(entry.pid, entry);
    const force = performance.now() - started >= 5_000;
    for (const entry of live) {
      const identity = `${entry.birth}\0${entry.command}`;
      if (!force && termSent.get(entry.pid) === identity) continue;
      const [current] = processes(["-p", String(entry.pid)]);
      if (current?.birth !== entry.birth || current.command !== entry.command) continue;
      try { process.kill(entry.pid, force ? "SIGKILL" : "SIGTERM"); }
      catch (error) { if (error.code !== "ESRCH") throw error; }
      termSent.set(entry.pid, identity);
    }
    await delay(100);
  }
  throw new Error(`Exact relaunch processes remained after cleanup for ${home}`);
}

if (require.main === module && process.send) {
  process.once("disconnect", () => {
    reap(process.argv[2], process.argv[3]).catch(error => {
      console.error(error.message);
      process.exitCode = 1;
    });
  });
  process.send("ready");
}
