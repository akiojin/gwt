"""Issue #6080: complete GTK caches avoid apt network access on warm runners."""

import os
import re
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest


ROOT = Path(__file__).resolve().parent.parent
BASH = shutil.which("bash") if os.name != "nt" else "C:/Program Files/Git/bin/bash.exe"


class AptCacheTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="gwt-apt-cache-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        for directory in ("archives", "apt/apt.conf.d", "apt/sources.list.d"):
            (self.root / directory).mkdir(parents=True)
        self.apt = self.root / "apt-get"
        self.apt.write_text(
            '#!/usr/bin/env bash\n'
            'echo "$*" >> "$GWT_MOCK_TRACE"\n'
            'if grep -qr packages.microsoft.com "$GWT_APT_ETC_DIR"/sources.list*; then exit 98; fi\n'
            'if [[ "${GWT_MOCK_EXPECT_HTTPS:-0}" == 1 ]] &&\n'
            '   grep -qrE "mirror\\+file:|http://(azure\\.archive|archive|security)\\.ubuntu\\.com" "$GWT_APT_ETC_DIR"/sources.list*; then exit 97; fi\n'
            'if [[ "${GWT_MOCK_OFFLINE:-0}" == 1 && "$*" != *--no-download* ]]; then exit 99; fi\n'
            'if [[ "$*" == *install* && "$*" != *--no-download* ]]; then\n'
            '  echo package > "$GWT_APT_CACHE_DIR/mock_1_amd64.deb"\n'
            'fi\n'
            'exit "${GWT_MOCK_EXIT:-0}"\n', encoding="utf-8", newline="\n",
        )
        self.apt.chmod(0o755)
        self.env = dict(os.environ, GWT_APT_GET=self.apt.as_posix(),
                        GWT_APT_CACHE_DIR=(self.root / "archives").as_posix(),
                        GWT_APT_CONF_DIR=(self.root / "apt/apt.conf.d").as_posix(),
                        GWT_APT_ETC_DIR=(self.root / "apt").as_posix(),
                        GWT_APT_OFFLINE_CACHE="1", GWT_APT_ATTEMPTS="1",
                        GWT_APT_LOCK_PROBE="false",
                        GWT_APT_RETRY_DELAY="0", GWT_MOCK_TRACE=(self.root / "trace").as_posix())

    def install(self, *packages, timeout=30, **env):
        return subprocess.run([BASH, (ROOT / "scripts/ci-apt.sh").as_posix(), "gtk-deps", *packages],
                              env=dict(self.env, **env), text=True, encoding="utf-8",
                              capture_output=True, timeout=timeout)

    def test_complete_cache_restores_without_update_or_download(self):
        cold = self.install()
        self.assertEqual(cold.returncode, 0, cold.stdout + cold.stderr)
        (self.root / "archives/unmanifested_1_amd64.deb").write_text("extra archive")
        (self.root / "trace").unlink()
        warm = self.install(GWT_MOCK_OFFLINE="1")
        self.assertEqual(warm.returncode, 0, warm.stdout + warm.stderr)
        calls = (self.root / "trace").read_text()
        self.assertNotIn(" update", calls)
        self.assertIn("--no-download", calls)
        self.assertIn("--no-remove", calls)
        self.assertIn("mock_1_amd64.deb", calls)
        self.assertNotIn("unmanifested_1_amd64.deb", calls)
        self.assertIn("cache_state=offline", warm.stdout)

    def test_incomplete_or_failed_cache_is_not_published_as_complete(self):
        failed = self.install(GWT_MOCK_EXIT="100")
        self.assertNotEqual(failed.returncode, 0)
        self.assertFalse((self.root / "archives/.gwt-complete").exists())
        (self.root / "trace").unlink()
        retry = self.install()
        self.assertEqual(retry.returncode, 0, retry.stdout + retry.stderr)
        self.assertIn(" update", (self.root / "trace").read_text())

    def test_corrupt_cache_is_rebuilt_instead_of_replayed_offline(self):
        self.assertEqual(self.install().returncode, 0)
        (self.root / "archives/mock_1_amd64.deb").write_text("corrupt")
        (self.root / "trace").unlink()
        warm = self.install()
        self.assertEqual(warm.returncode, 0, warm.stdout + warm.stderr)
        self.assertIn(" update", (self.root / "trace").read_text())
        self.assertNotIn("--no-download", (self.root / "trace").read_text())

    def test_microsoft_sources_are_removed_before_cold_apt(self):
        sources = self.root / "apt/sources.list.d"
        mixed = sources / "mixed.list"
        mixed.write_text("deb https://archive.ubuntu.com/ubuntu noble main\n"
                         "deb https://packages.microsoft.com/repos/code stable main\n")
        deb822 = sources / "mixed.sources"
        deb822.write_text("Types: deb\nURIs: https://archive.ubuntu.com/ubuntu\nSuites: noble\n\n"
                          "Types: deb\nURIs: https://packages.microsoft.com/ubuntu\nSuites: noble\n")
        main = self.root / "apt/sources.list"
        main.write_text("deb https://packages.microsoft.com/repos/edge stable main\n")
        result = self.install()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        for path in (mixed, deb822, main):
            self.assertNotIn("packages.microsoft.com", path.read_text())
        self.assertIn("archive.ubuntu.com", mixed.read_text())
        self.assertIn("archive.ubuntu.com", deb822.read_text())

    def test_ci_cold_sources_bypass_the_runner_http_mirror(self):
        sources = self.root / "apt/sources.list.d"
        deb822 = sources / "ubuntu.sources"
        deb822.write_text(
            "Types: deb\nURIs: mirror+file:/etc/apt/apt-mirrors.txt\n"
            "Suites: noble noble-updates\nComponents: main universe\n"
            "Signed-By: /usr/share/keyrings/ubuntu-archive-keyring.gpg\n\n"
            "Types: deb\nURIs: mirror+file:/etc/apt/apt-mirrors-security.txt\n"
            "Suites: noble-security\nComponents: main universe\n")
        legacy = sources / "ubuntu.list"
        legacy.write_text("deb http://azure.archive.ubuntu.com/ubuntu/ noble main\n"
                          "deb http://security.ubuntu.com/ubuntu noble-security main\n"
                          "deb https://example.org/packages stable main\n")
        result = self.install(GITHUB_ACTIONS="true", GWT_MOCK_EXPECT_HTTPS="1")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertNotIn("mirror+file:", deb822.read_text())
        self.assertIn("URIs: https://archive.ubuntu.com/ubuntu", deb822.read_text())
        self.assertIn("URIs: https://security.ubuntu.com/ubuntu", deb822.read_text())
        self.assertIn("Suites: noble noble-updates", deb822.read_text())
        self.assertIn("Signed-By: /usr/share/keyrings/ubuntu-archive-keyring.gpg", deb822.read_text())
        self.assertNotIn("azure.archive", legacy.read_text())
        self.assertNotIn("http://", legacy.read_text())
        self.assertIn("https://example.org/packages stable main", legacy.read_text())
        # The injected apt process must see the prepared sources on its first call.
        self.assertIn("source-mirror=ubuntu-https", result.stdout)

    # Issue #7081: `apt-get update` exits 0 when every InRelease fetch fails on
    # DNS, so the failure used to surface only as "Unable to locate package".
    def dns_outage(self, down):
        self.apt.write_text(
            '#!/usr/bin/env bash\n'
            'echo "$*" >> "$GWT_MOCK_TRACE"\n'
            'if [[ "$*" == *update* ]]; then\n'
            '  for host in archive.ubuntu.com azure.archive.ubuntu.com; do\n'
            '    if grep -qr "://$host/" "$GWT_APT_ETC_DIR"/sources.list* && [[ " ' + down + ' " == *" $host "* ]]; then\n'
            '      printf "Err:1 https://%s/ubuntu resolute InRelease\\n  %s\\n" "$host" "${GWT_MOCK_REASON:-Could not resolve $host}"\n'
            '      echo "W: Some index files failed to download."\n'
            '      exit 0\n'
            '    fi\n'
            '  done\n'
            'fi\n'
            'if [[ "$*" == *install* && "$*" != *--no-download* ]]; then\n'
            '  if grep -qr "://archive.ubuntu.com/" "$GWT_APT_ETC_DIR"/sources.list* && [[ " ' + down + ' " == *" archive.ubuntu.com "* ]]; then\n'
            '    echo "E: Unable to locate package libgtk-3-dev"; exit 100\n'
            '  fi\n'
            'fi\n'
            'exit 0\n', encoding="utf-8", newline="\n",
        )
        (self.root / "apt/sources.list").write_text(
            "deb http://archive.ubuntu.com/ubuntu resolute main\n"
            "deb http://security.ubuntu.com/ubuntu resolute-security main\n")

    def outage_install(self, **env):
        return self.install(GITHUB_ACTIONS="true", GITHUB_STEP_SUMMARY="", timeout=120,
                            GWT_APT_OFFLINE_CACHE="0", **env)

    def test_unresolvable_archive_fails_over_to_the_runner_mirror(self):
        self.dns_outage("archive.ubuntu.com")
        result = self.outage_install()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("status=mirror-unreachable", result.stdout)
        self.assertIn("mirror-switch", result.stdout)
        sources = (self.root / "apt/sources.list").read_text()
        self.assertIn("http://azure.archive.ubuntu.com/ubuntu resolute main", sources)
        self.assertIn("http://azure.archive.ubuntu.com/ubuntu resolute-security main", sources)
        calls = (self.root / "trace").read_text().splitlines()
        self.assertEqual(sum(" update" in f" {c}" for c in calls), 2, calls)

    def test_all_mirrors_down_backs_off_and_reports_an_external_cause(self):
        self.dns_outage("archive.ubuntu.com azure.archive.ubuntu.com")
        result = self.outage_install(GWT_APT_RETRY_DELAY="1", GWT_APT_TOTAL_DEADLINE="40")
        output = result.stdout + result.stderr
        self.assertNotEqual(result.returncode, 0, output)
        waits = [int(w) for w in re.findall(r"backoff=(\d+)s", output)]
        self.assertGreaterEqual(len(waits), 2, output)
        self.assertEqual(waits, [2 ** n for n in range(len(waits))], output)
        self.assertIn("mirror unreachable (external cause", output)
        self.assertIn("ミラー到達不能（外部要因）", output)
        self.assertNotIn("install", (self.root / "trace").read_text())

    def test_all_mirrors_down_installs_the_newest_cached_archives(self):
        self.dns_outage("archive.ubuntu.com azure.archive.ubuntu.com")
        for name in ("foo_1.0_amd64.deb", "foo_1.10_amd64.deb", "bar_2_amd64.deb"):
            (self.root / "archives" / name).write_text("archive")
        result = self.outage_install(GWT_APT_TOTAL_DEADLINE="30")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("cache-replay", result.stdout)
        install = [c for c in (self.root / "trace").read_text().splitlines() if "install" in c]
        self.assertEqual(len(install), 1, install)
        self.assertIn("--no-download", install[0])
        self.assertIn("foo_1.10_amd64.deb", install[0])
        self.assertIn("bar_2_amd64.deb", install[0])
        self.assertNotIn("foo_1.0_amd64.deb", install[0])

    # Issue #7717: reasons from run 44518013157. apt prints the host on the Err
    # line and the reason on the next one, so neither matched the #7081 regex.
    SERVER_FD = ("Could not wait for server fd - select (11: Resource temporarily "
                 "unavailable) [IP: 91.189.91.83 443]")
    TLS_RESET = ("SSL connection failed: error:00000000:lib(0)::reason(0) / "
                 "Connection reset by peer [IP: 91.189.92.24 443]")

    def assert_in_order(self, output, *markers):
        positions = [output.find(m) for m in markers]
        self.assertNotIn(-1, positions, output)
        self.assertEqual(positions, sorted(positions), output)

    def test_server_fd_wait_failure_fails_over_to_the_runner_mirror(self):
        self.dns_outage("archive.ubuntu.com")
        result = self.outage_install(GWT_MOCK_REASON=self.SERVER_FD)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assert_in_order(result.stdout, "status=mirror-unreachable",
                             "mirror-switch to=http://azure.archive.ubuntu.com/ubuntu")
        self.assertNotIn("Unable to locate package", result.stdout)

    def test_tls_reset_on_every_mirror_replays_the_cache(self):
        self.dns_outage("archive.ubuntu.com azure.archive.ubuntu.com")
        (self.root / "archives/foo_1_amd64.deb").write_text("archive")
        result = self.outage_install(GWT_MOCK_REASON=self.TLS_RESET, GWT_APT_TOTAL_DEADLINE="30")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assert_in_order(result.stdout, "mirror-switch", "cache-replay")
        self.assertNotIn("backoff=", result.stdout)

    def test_tls_reset_without_a_cache_backs_off_instead_of_giving_up(self):
        self.dns_outage("archive.ubuntu.com azure.archive.ubuntu.com")
        result = self.outage_install(GWT_MOCK_REASON=self.TLS_RESET,
                                     GWT_APT_RETRY_DELAY="1", GWT_APT_TOTAL_DEADLINE="10")
        output = result.stdout + result.stderr
        self.assertNotEqual(result.returncode, 0, output)
        self.assert_in_order(output, "mirror-switch", "backoff=1s", "backoff=2s")
        self.assertIn("mirror unreachable (external cause", output)
        self.assertNotIn("install", (self.root / "trace").read_text())

    def test_unlocatable_packages_after_every_index_failed_is_a_mirror_outage(self):
        self.dns_outage("archive.ubuntu.com")
        result = self.outage_install(GWT_MOCK_REASON="Connection failed [IP: 91.189.91.83 443]")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assert_in_order(result.stdout, "Unable to locate package", "status=mirror-unreachable",
                             "mirror-switch to=http://azure.archive.ubuntu.com/ubuntu")

    def test_local_install_preserves_the_configured_mirror(self):
        source = self.root / "apt/sources.list"
        body = "deb http://azure.archive.ubuntu.com/ubuntu/ noble main\n"
        source.write_text(body)
        result = self.install(GITHUB_ACTIONS="false")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(source.read_text(), body)


if __name__ == "__main__":
    unittest.main()
