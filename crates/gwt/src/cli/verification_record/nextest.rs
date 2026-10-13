//! Command-local nextest configuration and JUnit evidence.

use std::{fs, path::Path};

use quick_xml::{events::Event, Reader};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TestDuration {
    pub classname: String,
    pub name: String,
    pub duration_seconds: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NextestEvidence {
    pub xml: String,
    pub total_tests: usize,
    pub slowest: Vec<TestDuration>,
}

pub struct Capture {
    directory: tempfile::TempDir,
}

impl Capture {
    pub fn new(worktree: &Path) -> Result<Self, String> {
        let source = fs::read_to_string(worktree.join(".config/nextest.toml"))
            .map_err(|error| format!("read nextest configuration: {error}"))?;
        let mut config: toml::Value = toml::from_str(&source)
            .map_err(|error| format!("parse nextest configuration: {error}"))?;
        let directory = tempfile::tempdir().map_err(|error| error.to_string())?;
        set_config_value(
            &mut config,
            &["store", "dir"],
            directory.path().to_string_lossy().into_owned(),
        )?;
        set_config_value(
            &mut config,
            &["profile", "gwt-verify", "junit", "path"],
            "junit.xml".to_string(),
        )?;
        let rendered = toml::to_string(&config).map_err(|error| error.to_string())?;
        fs::write(directory.path().join("nextest.toml"), rendered)
            .map_err(|error| error.to_string())?;
        Ok(Self { directory })
    }

    pub fn arguments(&self) -> Vec<String> {
        vec![
            "--config-file".to_string(),
            self.directory
                .path()
                .join("nextest.toml")
                .to_string_lossy()
                .into_owned(),
        ]
    }

    pub fn evidence(&self) -> Result<NextestEvidence, String> {
        let xml = fs::read_to_string(self.directory.path().join("gwt-verify/junit.xml"))
            .map_err(|error| format!("read nextest JUnit report: {error}"))?;
        let mut reader = Reader::from_str(&xml);
        let mut tests = Vec::new();
        let mut depth = 0usize;
        let mut root_seen = false;
        loop {
            let event = reader
                .read_event()
                .map_err(|error| format!("parse nextest JUnit: {error}"))?;
            let empty = matches!(&event, Event::Empty(_));
            match event {
                Event::Start(element) | Event::Empty(element) => {
                    if depth == 0 {
                        if root_seen
                            || !matches!(element.name().as_ref(), "testsuites" | "testsuite")
                        {
                            return Err("nextest JUnit must have one testsuites or testsuite root"
                                .to_string());
                        }
                        root_seen = true;
                    }
                    if element.name().as_ref() == "testcase" {
                        let mut classname = String::new();
                        let mut name = None;
                        let mut duration = None;
                        for attribute in element.attributes() {
                            let attribute = attribute.map_err(|error| error.to_string())?;
                            // Preserve attribute whitespace as well as expanding XML references.
                            let value = quick_xml::escape::unescape(&attribute.value)
                                .map_err(|error| error.to_string())?
                                .into_owned();
                            match attribute.key.as_ref() {
                                "classname" => classname = value,
                                "name" => name = Some(value),
                                "time" => duration = Some(value),
                                _ => {}
                            }
                        }
                        let duration_seconds = duration.ok_or("JUnit testcase is missing time")?;
                        let seconds: f64 = duration_seconds
                            .parse()
                            .map_err(|_| "JUnit testcase time is not numeric")?;
                        if !seconds.is_finite() || seconds < 0.0 {
                            return Err(
                                "JUnit testcase time must be finite and non-negative".to_string()
                            );
                        }
                        tests.push((
                            seconds,
                            TestDuration {
                                classname,
                                name: name.ok_or("JUnit testcase is missing name")?,
                                duration_seconds,
                            },
                        ));
                    }
                    if !empty {
                        depth += 1;
                    }
                }
                Event::End(_) => {
                    depth = depth
                        .checked_sub(1)
                        .ok_or("unexpected JUnit closing element")?;
                }
                Event::Text(text) => {
                    let unescaped =
                        quick_xml::escape::unescape(&text).map_err(|error| error.to_string())?;
                    if depth == 0 && !unescaped.trim().is_empty() {
                        return Err("text outside nextest JUnit root".to_string());
                    }
                }
                Event::Eof => break,
                _ => {}
            }
        }
        if !root_seen || depth != 0 {
            return Err("nextest JUnit report is empty or truncated".to_string());
        }
        let total_tests = tests.len();
        tests.sort_by(|left, right| right.0.total_cmp(&left.0));
        Ok(NextestEvidence {
            xml,
            total_tests,
            slowest: tests.into_iter().take(20).map(|(_, test)| test).collect(),
        })
    }
}

fn set_config_value(config: &mut toml::Value, path: &[&str], value: String) -> Result<(), String> {
    let mut current = config;
    for key in &path[..path.len() - 1] {
        let table = current
            .as_table_mut()
            .ok_or("nextest configuration section is not a table")?;
        current = table
            .entry((*key).to_string())
            .or_insert_with(|| toml::Value::Table(toml::map::Map::new()));
    }
    current
        .as_table_mut()
        .ok_or("nextest configuration section is not a table")?
        .insert(path[path.len() - 1].to_string(), toml::Value::String(value));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, path::PathBuf};

    fn fixture() -> tempfile::TempDir {
        let directory = tempfile::tempdir().unwrap();
        fs::create_dir(directory.path().join(".config")).unwrap();
        fs::write(
            directory.path().join(".config/nextest.toml"),
            r#"
[profile.default]
fail-fast = false
[test-groups.git-process]
max-threads = 1
[[profile.default.overrides]]
filter = 'test(cli::workspace::)'
test-group = 'git-process'
[profile.gwt-verify]
retries = 0
[profile.gwt-verify.junit]
path = 'stale.xml'
"#,
        )
        .unwrap();
        directory
    }

    fn config(capture: &Capture) -> toml::Value {
        let arguments = capture.arguments();
        assert_eq!(arguments.len(), 2);
        assert_eq!(arguments[0], "--config-file");
        toml::from_str(&fs::read_to_string(&arguments[1]).unwrap()).unwrap()
    }

    fn report_path(capture: &Capture) -> PathBuf {
        let config = config(capture);
        PathBuf::from(config["store"]["dir"].as_str().unwrap())
            .join("gwt-verify")
            .join("junit.xml")
    }

    fn write_report(capture: &Capture, xml: &str) {
        let path = report_path(capture);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, xml).unwrap();
    }

    #[test]
    fn capture_preserves_configuration_and_isolates_each_report() {
        let fixture = fixture();
        let source = fixture.path().join(".config/nextest.toml");
        let before = fs::read_to_string(&source).unwrap();
        let first = Capture::new(fixture.path()).unwrap();
        let second = Capture::new(fixture.path()).unwrap();
        let config = config(&first);
        assert_eq!(
            config["profile"]["default"]["fail-fast"].as_bool(),
            Some(false)
        );
        assert_eq!(
            config["test-groups"]["git-process"]["max-threads"].as_integer(),
            Some(1)
        );
        assert_eq!(
            config["profile"]["default"]["overrides"][0]["test-group"].as_str(),
            Some("git-process")
        );
        assert_eq!(
            config["profile"]["gwt-verify"]["retries"].as_integer(),
            Some(0)
        );
        assert_eq!(
            config["profile"]["gwt-verify"]["junit"]["path"].as_str(),
            Some("junit.xml")
        );
        assert!(report_path(&first).is_absolute());
        assert_ne!(report_path(&first), report_path(&second));
        write_report(&first, "<testsuites><testsuite><testcase classname=\"unit\" name=\"a\" time=\"1\"/></testsuite></testsuites>");
        assert!(first.evidence().is_ok());
        assert!(
            second.evidence().is_err(),
            "must not reuse another run's report"
        );
        assert_eq!(fs::read_to_string(source).unwrap(), before);
    }

    #[test]
    fn evidence_retains_xml_and_reports_twenty_slowest_tests_numerically() {
        let fixture = fixture();
        let capture = Capture::new(fixture.path()).unwrap();
        let mut xml = String::from("<?xml version=\"1.0\"?><testsuites><testsuite>");
        for duration in 1..=25 {
            xml.push_str(&format!("<testcase classname=\"unit&amp;integration\" name=\"test&lt;{duration}&gt;\" time=\"{duration}.25\"></testcase>"));
        }
        xml.push_str("</testsuite></testsuites>");
        write_report(&capture, &xml);
        let evidence = capture.evidence().unwrap();
        assert_eq!(evidence.xml, xml);
        assert_eq!(evidence.total_tests, 25);
        assert_eq!(evidence.slowest.len(), 20);
        assert_eq!(evidence.slowest[0].classname, "unit&integration");
        assert_eq!(evidence.slowest[0].name, "test<25>");
        assert_eq!(evidence.slowest[0].duration_seconds, "25.25");
        assert_eq!(evidence.slowest[19].duration_seconds, "6.25");
    }

    #[test]
    fn missing_or_malformed_junit_is_not_verification_evidence() {
        let fixture = fixture();
        let capture = Capture::new(fixture.path()).unwrap();
        assert!(capture.evidence().is_err());
        for xml in [
            "<testsuites><testsuite>",
            "<testsuites><testcase name=\"a\" time=\"oops\"/></testsuites>",
        ] {
            write_report(&capture, xml);
            assert!(
                capture.evidence().is_err(),
                "accepted invalid report: {xml}"
            );
        }
    }
}
