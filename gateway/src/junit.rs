//! Best-effort JUnit artifact ingestion for completed workflow runs.
//!
//! GitHub credentials stay in the gateway: it lists and downloads run
//! artifacts, parses bounded XML files, then writes the observed test facts.
//! Analytics remains a pure Postgres + RabbitMQ consumer.

use std::{
    collections::HashMap,
    io::{Cursor, Read},
};

use chrono::{DateTime, Utc};
use quick_xml::de::from_str;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use tracing::{info, warn};
use uuid::Uuid;
use zip::ZipArchive;

use crate::{
    error::AppError,
    github_api::{self, GitHubApi},
    state::AppState,
};

const MAX_ARTIFACTS_PER_RUN: usize = 50;
const MAX_ARTIFACT_BYTES: i64 = 50 * 1024 * 1024;
const MAX_XML_FILES_PER_ARCHIVE: usize = 500;
const MAX_XML_BYTES_PER_FILE: u64 = 10 * 1024 * 1024;
const MAX_XML_BYTES_PER_ARCHIVE: u64 = 50 * 1024 * 1024;

#[derive(Clone)]
pub struct JunitCapture {
    pub repository_id: Uuid,
    pub workflow_run_id: Uuid,
    pub owner: String,
    pub name: String,
    pub github_run_id: i64,
    pub executed_at: DateTime<Utc>,
}

pub async fn capture_run_tests(state: &AppState, capture: JunitCapture) -> Result<(), AppError> {
    let Some(token) = github_api::repository_token(state, capture.repository_id).await? else {
        info!(
            repository_id = %capture.repository_id,
            "no usable GitHub token for JUnit artifact capture; skipping"
        );
        return Ok(());
    };
    let api = GitHubApi::new(state, &token);
    let artifacts = api
        .run_artifacts(
            &capture.owner,
            &capture.name,
            capture.github_run_id,
            MAX_ARTIFACTS_PER_RUN,
        )
        .await?;
    let mut results = HashMap::new();

    for artifact in artifacts
        .into_iter()
        .filter(|artifact| !artifact.expired && artifact.size_in_bytes <= MAX_ARTIFACT_BYTES)
        .take(MAX_ARTIFACTS_PER_RUN)
    {
        let Some(bytes) = api
            .artifact_zip(&capture.owner, &capture.name, artifact.id)
            .await?
        else {
            continue;
        };
        let artifact_name = artifact.name.clone();
        let parsed = tokio::task::spawn_blocking(move || parse_archive(&bytes))
            .await
            .map_err(AppError::internal)?;
        match parsed {
            Ok(parsed) => {
                for result in parsed {
                    results.insert(result.test_key.clone(), result);
                }
            }
            Err(error) => {
                warn!(artifact = %artifact_name, %error, "could not parse JUnit artifact")
            }
        }
    }

    if results.is_empty() {
        return Ok(());
    }
    let mut transaction = state.db.begin().await?;
    for result in results.values() {
        sqlx::query(
            "INSERT INTO test_results
                (repository_id, workflow_run_id, workflow_job_id, test_key,
                 suite, classname, name, status, duration_ms, failure_type,
                 failure_message, executed_at)
             VALUES ($1, $2, NULL, $3, $4, $5, $6, $7, $8, $9, $10, $11)
             ON CONFLICT ON CONSTRAINT test_results_run_job_test_key DO UPDATE SET
                 suite = EXCLUDED.suite,
                 classname = EXCLUDED.classname,
                 name = EXCLUDED.name,
                 status = EXCLUDED.status,
                 duration_ms = EXCLUDED.duration_ms,
                 failure_type = EXCLUDED.failure_type,
                 failure_message = EXCLUDED.failure_message,
                 executed_at = EXCLUDED.executed_at",
        )
        .bind(capture.repository_id)
        .bind(capture.workflow_run_id)
        .bind(&result.test_key)
        .bind(&result.suite)
        .bind(&result.classname)
        .bind(&result.name)
        .bind(&result.status)
        .bind(result.duration_ms)
        .bind(&result.failure_type)
        .bind(&result.failure_message)
        .bind(capture.executed_at)
        .execute(&mut *transaction)
        .await?;
    }
    transaction.commit().await?;
    info!(
        workflow_run_id = %capture.workflow_run_id,
        tests = results.len(),
        "captured JUnit test results"
    );
    Ok(())
}

fn parse_archive(bytes: &[u8]) -> Result<Vec<TestResult>, String> {
    let mut archive = ZipArchive::new(Cursor::new(bytes)).map_err(|error| error.to_string())?;
    let mut results = Vec::new();
    let mut xml_files = 0;
    let mut total_xml_bytes = 0;
    for index in 0..archive.len() {
        let mut file = archive.by_index(index).map_err(|error| error.to_string())?;
        if file.is_dir() || !file.name().to_ascii_lowercase().ends_with(".xml") {
            continue;
        }
        if file.enclosed_name().is_none() {
            continue;
        }
        xml_files += 1;
        if xml_files > MAX_XML_FILES_PER_ARCHIVE {
            return Err("artifact contains too many XML files".to_string());
        }
        if file.size() > MAX_XML_BYTES_PER_FILE {
            continue;
        }
        total_xml_bytes += file.size();
        if total_xml_bytes > MAX_XML_BYTES_PER_ARCHIVE {
            return Err("artifact contains too much uncompressed XML".to_string());
        }
        let mut xml = Vec::with_capacity(file.size() as usize);
        file.by_ref()
            .take(MAX_XML_BYTES_PER_FILE + 1)
            .read_to_end(&mut xml)
            .map_err(|error| error.to_string())?;
        if xml.len() as u64 > MAX_XML_BYTES_PER_FILE {
            continue;
        }
        let Ok(xml) = std::str::from_utf8(&xml) else {
            continue;
        };
        results.extend(parse_document(xml));
    }
    Ok(results)
}

fn parse_document(xml: &str) -> Vec<TestResult> {
    if let Ok(root) = from_str::<TestSuites>(xml)
        && !root.suites.is_empty()
    {
        let mut results = Vec::new();
        for suite in root.suites {
            collect_suite(suite, None, &mut results);
        }
        return results;
    }
    let Ok(suite) = from_str::<TestSuite>(xml) else {
        return Vec::new();
    };
    let mut results = Vec::new();
    collect_suite(suite, None, &mut results);
    results
}

fn collect_suite(suite: TestSuite, parent: Option<&str>, results: &mut Vec<TestResult>) {
    let suite_name = suite.name.as_deref().or(parent).map(str::to_owned);
    for case in suite.cases {
        let Some(name) = case.name.filter(|name| !name.is_empty()) else {
            continue;
        };
        let (status, failure) = if let Some(failure) = case.failure {
            ("failed", Some(failure))
        } else if let Some(error) = case.error {
            ("error", Some(error))
        } else if case.skipped.is_some() {
            ("skipped", None)
        } else {
            ("passed", None)
        };
        let failure_type = failure.as_ref().and_then(|failure| failure.kind.clone());
        let failure_message = failure.and_then(|failure| failure.message.or(failure.text));
        let identity = format!(
            "{}\0{}\0{}",
            suite_name.as_deref().unwrap_or_default(),
            case.classname.as_deref().unwrap_or_default(),
            name
        );
        results.push(TestResult {
            test_key: format!("{:x}", Sha256::digest(identity.as_bytes())),
            suite: suite_name.clone(),
            classname: case.classname,
            name,
            status: status.to_string(),
            duration_ms: case.time.and_then(seconds_to_millis),
            failure_type,
            failure_message,
        });
    }
    for child in suite.suites {
        collect_suite(child, suite_name.as_deref(), results);
    }
}

fn seconds_to_millis(value: String) -> Option<i64> {
    let seconds = value.parse::<f64>().ok()?;
    (seconds.is_finite() && seconds >= 0.0).then(|| (seconds * 1_000.0).round() as i64)
}

#[derive(Debug)]
struct TestResult {
    test_key: String,
    suite: Option<String>,
    classname: Option<String>,
    name: String,
    status: String,
    duration_ms: Option<i64>,
    failure_type: Option<String>,
    failure_message: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TestSuites {
    #[serde(rename = "testsuite", default)]
    suites: Vec<TestSuite>,
}

#[derive(Debug, Deserialize)]
struct TestSuite {
    #[serde(rename = "@name")]
    name: Option<String>,
    #[serde(rename = "testcase", default)]
    cases: Vec<TestCase>,
    #[serde(rename = "testsuite", default)]
    suites: Vec<TestSuite>,
}

#[derive(Debug, Deserialize)]
struct TestCase {
    #[serde(rename = "@name")]
    name: Option<String>,
    #[serde(rename = "@classname")]
    classname: Option<String>,
    #[serde(rename = "@time")]
    time: Option<String>,
    failure: Option<Failure>,
    error: Option<Failure>,
    skipped: Option<Skipped>,
}

#[derive(Debug, Deserialize)]
struct Failure {
    #[serde(rename = "@type")]
    kind: Option<String>,
    #[serde(rename = "@message")]
    message: Option<String>,
    #[serde(rename = "$text")]
    text: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Skipped {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_common_junit_outcomes_and_stable_keys() {
        let xml = r#"<?xml version="1.0"?>
            <testsuite name="unit">
              <testcase classname="math.Calculator" name="adds" time="0.012"/>
              <testcase classname="math.Calculator" name="divides" time="0.2">
                <failure type="AssertionError" message="expected 2">stack</failure>
              </testcase>
              <testcase classname="math.Calculator" name="optional"><skipped/></testcase>
            </testsuite>"#;
        let results = parse_document(xml);
        assert_eq!(results.len(), 3);
        assert_eq!(results[0].status, "passed");
        assert_eq!(results[0].duration_ms, Some(12));
        assert_eq!(results[1].status, "failed");
        assert_eq!(results[1].failure_type.as_deref(), Some("AssertionError"));
        assert_eq!(results[2].status, "skipped");
        assert_eq!(results[0].test_key.len(), 64);
    }

    #[test]
    fn parses_testsuites_root_and_nested_suites() {
        let xml = r#"<testsuites><testsuite name="outer"><testsuite name="inner">
            <testcase name="works"/></testsuite></testsuite></testsuites>"#;
        let results = parse_document(xml);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].suite.as_deref(), Some("inner"));
    }
}
