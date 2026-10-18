"""Acceptance tests use an independent local Git/Cargo fixture."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time
import unittest

SCRIPT = Path(__file__).with_name("ci_coverage_cache.py").resolve()
PROJECT = Path(subprocess.check_output(["git", "-C", str(SCRIPT.parent), "rev-parse", "--show-toplevel"]).decode().strip())
BASE = (PROJECT / "target" / "coverage-cache-test").resolve()
BUILD_SOURCE = (PROJECT / "crates/gwt/build.rs").read_text(encoding="utf-8")


class CoverageCacheTests(unittest.TestCase):
    def setUp(self):
        BASE.mkdir(parents=True, exist_ok=True)
        self.temp = tempfile.TemporaryDirectory(prefix="fixture-", dir=BASE)
        self.root = Path(self.temp.name).resolve()
        self.assertTrue(self.root.is_relative_to(BASE))
        self.write(".gitignore", "/target/\n.claude/skills/ignored*\n")
        self.write("Cargo.toml", '[workspace]\nmembers=["crates/gwt"]\nexclude=["vendor/winresource"]\nresolver="2"\n')
        self.write("crates/gwt/Cargo.toml", '[package]\nname="coverage-cache-fixture"\nversion="0.1.0"\nedition="2021"\n[build-dependencies]\nwinresource={path="../../vendor/winresource"}\n')
        self.write("crates/gwt/src/main.rs", 'fn main(){println!("{}:{}|{}",include_str!("../../../assets/input.txt").trim(),env!("INPUTS"),env!("GWT_BUILD_COMMIT"));}\n')
        self.write("assets/input.txt", "one\n")
        self.write("inputs/old.txt", "old\n")
        self.write(".claude/skills/demo/SKILL.md", "demo\n")
        self.write("crates/gwt/actual-build.rs", BUILD_SOURCE)
        self.write("crates/gwt/build.rs", '''#[allow(dead_code)]
mod production {include!("actual-build.rs");pub fn stamp(){emit_build_stamp();}}
fn main(){production::stamp();
println!("cargo:rerun-if-changed=../../inputs");
let mut n:Vec<_>=std::fs::read_dir("../../inputs").unwrap().map(|e|e.unwrap().file_name().to_str().unwrap().to_owned()).collect();
n.sort();println!("cargo:rustc-env=INPUTS={}",n.join(","));}
''')
        # The included Windows resource function is compiled but never called.
        self.write("vendor/winresource/Cargo.toml", '[package]\nname="winresource"\nversion="0.1.0"\nedition="2021"\n[workspace]\n')
        self.write("vendor/winresource/src/lib.rs", '''pub struct WindowsResource;
impl WindowsResource {pub fn new()->Self{Self} pub fn set_icon(&mut self,_:&str){}
pub fn set(&mut self,_:&str,_:&str){} pub fn set_manifest(&mut self,_:&str){}
pub fn compile(&mut self)->std::io::Result<()>{Ok(())}}
''')
        self.command("git", "init", "--quiet")
        self.command("git", "config", "user.name", "Cache fixture")
        self.command("git", "config", "user.email", "cache-fixture@example.invalid")
        self.command("cargo", "generate-lockfile", "--offline", "--manifest-path", str(self.root / "Cargo.toml"))
        self.commit()
        self.output = self.root / "target/outputs"

    def tearDown(self):
        self.assertTrue(self.root.resolve().is_relative_to(BASE))
        self.temp.cleanup()

    def write(self, name, text):
        path = self.root / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text, encoding="utf-8")

    def command(self, *args):
        result = subprocess.run(args, cwd=self.root, env={**os.environ, "CARGO_LLVM_COV": "1"}, capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        return result.stdout + result.stderr

    def commit(self):
        self.command("git", "add", "--all")
        self.command("git", "commit", "--quiet", "-m", "test: fixture")

    def cli(self, action, success=True, restored=True):
        self.output.parent.mkdir(exist_ok=True)
        result = subprocess.run([sys.executable, str(SCRIPT), action], cwd=self.root,
                                env={**os.environ, "GITHUB_OUTPUT": str(self.output),
                                     "COVERAGE_CACHE_RESTORED": "fixture-matched-key" if restored else ""}, capture_output=True, text=True)
        self.assertEqual(result.returncode == 0, success, result.stdout + result.stderr)
        return result

    def refresh(self):
        names = self.command("git", "ls-files", "-z").split("\0")
        directories, stamp = {self.root}, time.time_ns()
        for name in filter(None, names):
            path = self.root / name
            os.utime(path, ns=(stamp, stamp))
            directories.update(p for p in path.parents if p == self.root or p.is_relative_to(self.root))
        for path in sorted(directories, key=lambda p: -len(p.parts)):
            os.utime(path, ns=(stamp, stamp))

    def metadata(self):
        return {str(p): p.lstat().st_mtime_ns for p in (self.root / ".git").rglob("*")}

    def build(self):
        log = self.command("cargo", "build", "--offline", "--manifest-path", str(self.root / "Cargo.toml"),
                           "--target-dir", str(self.root / "target/llvm-cov-target"), "-j", "1", "-vv")
        return "\n".join(line for line in log.splitlines() if any(word in line for word in ("Dirty", "Fresh", "Compiling", "Finished")))

    def test_real_cargo_reuses_unchanged_inputs_and_rebuilds_changes(self):
        self.cli("restore")
        self.assertIn("Compiling coverage-cache-fixture", self.build())
        self.write("target/llvm-cov-target/nested/old.profraw", "raw")
        self.write("target/llvm-cov-target/old.profdata", "merged")
        self.write("target/llvm-cov-target/nested/keep.rlib", "artifact")
        self.write("target/unrelated/keep.profraw", "unrelated")
        capture = self.cli("capture")
        self.assertIn("cacheable=true", self.output.read_text(), capture.stdout)
        self.assertFalse((self.root / "target/llvm-cov-target/nested/old.profraw").exists())
        self.assertFalse((self.root / "target/llvm-cov-target/old.profdata").exists())
        self.assertTrue((self.root / "target/llvm-cov-target/nested/keep.rlib").exists())
        self.assertTrue((self.root / "target/unrelated/keep.profraw").exists())
        sidecar = self.root / "target/coverage-cache-source.json"
        self.assertTrue(sidecar.is_file())
        self.refresh()
        metadata = self.metadata()
        self.cli("restore")
        self.assertEqual(self.metadata(), metadata)
        self.assertIn("Fresh coverage-cache-fixture", self.build())
        self.cli("capture")
        self.write("assets/input.txt", "two\n")
        self.commit()
        self.refresh()
        stamp = (self.root / "assets/input.txt").stat().st_mtime_ns
        self.cli("restore")
        self.assertEqual((self.root / "assets/input.txt").stat().st_mtime_ns, stamp)
        self.assertIn("Compiling coverage-cache-fixture", self.build())
        self.cli("capture")
        self.command("git", "rm", "--quiet", "inputs/old.txt")
        self.write("inputs/new.txt", "new\n")
        self.commit()
        self.refresh()
        stamp = (self.root / "inputs").stat().st_mtime_ns
        self.cli("restore")
        self.assertEqual((self.root / "inputs").stat().st_mtime_ns, stamp)
        self.assertIn("Compiling coverage-cache-fixture", self.build())
        executable = "coverage-cache-fixture" + (".exe" if os.name == "nt" else "")
        binary = str(self.root / "target/llvm-cov-target/debug" / executable)
        head = self.command("git", "rev-parse", "HEAD").strip()
        self.assertEqual(self.command(binary).strip(), "two:new.txt|" + head)
        self.cli("capture")
        self.command("git", "commit", "--quiet", "--allow-empty", "-m", "test: new head with unchanged source")
        self.refresh()
        metadata = self.metadata()
        self.cli("restore")
        self.assertEqual(self.metadata(), metadata)
        log = self.build()
        head = self.command("git", "rev-parse", "HEAD").strip()
        self.assertEqual(self.command(binary).strip(), "two:new.txt|" + head, log)
        self.cli("capture")
        # A real fresh checkout also refreshes .git/HEAD. It must rebuild the
        # stamped crate even when the source commit is unchanged.
        self.refresh()
        stamp = time.time_ns()
        os.utime(self.root / ".git/HEAD", ns=(stamp, stamp))
        metadata = self.metadata()
        self.cli("restore")
        self.assertEqual(self.metadata(), metadata)
        self.assertIn("Compiling coverage-cache-fixture", self.build())
        self.assertEqual(self.command(binary).strip(), "two:new.txt|" + head)

    def test_bad_metadata_clears_only_instrumented_target(self):
        self.cli("restore")
        self.write("target/llvm-cov-target/stale", "stale")
        self.write("target/unrelated/keep", "keep")
        data = json.loads((self.root / "target/coverage-cache-inputs.json").read_text())["payload"]
        data["files"]["assets/input.txt"]["mtime_ns"] = -1
        from ci_coverage_cache import envelope
        self.write("target/coverage-cache-source.json", json.dumps(envelope(data)))
        self.cli("restore")
        self.assertFalse((self.root / "target/llvm-cov-target").exists())
        self.assertTrue((self.root / "target/unrelated/keep").exists())
        self.assertTrue((self.root / "target/coverage-cache-inputs.json").is_file())
        data = json.loads((self.root / "target/coverage-cache-inputs.json").read_text())["payload"]
        bad_checksum = envelope(data)
        bad_checksum["sha256"] = "0" * 64
        self.write("target/coverage-cache-source.json", json.dumps(bad_checksum))
        self.write("target/llvm-cov-target/stale", "stale")
        self.cli("restore")
        self.assertFalse((self.root / "target/llvm-cov-target").exists())
        self.assertTrue((self.root / "target/unrelated/keep").exists())
        self.write("target/coverage-cache-source.json", json.dumps(envelope(data)))
        self.write("target/llvm-cov-target/partially-restored", "partial")
        self.cli("restore", restored=False)
        self.assertFalse((self.root / "target/llvm-cov-target").exists())
        self.assertTrue((self.root / "target/unrelated/keep").exists())
        self.write("target/llvm-cov-target/stale", "stale embedded inventory")
        self.cli("capture")
        self.write(".claude/skills/demo/added.md", "new embedded input")
        self.commit()
        self.cli("restore")
        self.assertFalse((self.root / "target/llvm-cov-target").exists())
        self.assertTrue((self.root / "target/unrelated/keep").exists())

    def test_capture_refuses_changed_head_and_ignored_embedded_input(self):
        self.cli("restore")
        self.write("target/llvm-cov-target/marker", "artifact")
        self.command("git", "commit", "--quiet", "--allow-empty", "-m", "test: changed head")
        self.cli("capture")
        self.assertNotIn("cacheable=true", self.output.read_text())
        self.assertFalse((self.root / "target/coverage-cache-source.json").exists())
        self.cli("restore")
        self.write("target/llvm-cov-target/marker", "artifact")
        self.write(".claude/skills/ignored-extra.txt", "runtime-visible, git-ignored")
        self.assertEqual(self.command("git", "status", "--porcelain").strip(), "")
        self.cli("capture")
        self.assertNotIn("cacheable=true", self.output.read_text())

    def test_symlink_target_boundary_never_removes_outside_files(self):
        self.write("outside/keep", "keep")
        self.commit()
        (self.root / "target").mkdir(exist_ok=True)
        try:
            os.symlink(self.root / "outside", self.root / "target/llvm-cov-target", target_is_directory=True)
        except OSError as error:
            if os.name != "nt":
                self.skipTest(str(error))
            junction = subprocess.run(["cmd", "/d", "/c", "mklink", "/J", str(self.root / "target/llvm-cov-target"),
                                       str(self.root / "outside")], capture_output=True, text=True)
            if junction.returncode:
                self.skipTest(junction.stderr)
        self.cli("restore", success=False)
        self.assertEqual((self.root / "outside/keep").read_text(), "keep")


if __name__ == "__main__":
    unittest.main()
