//! Build-log storage.
//!
//! Logs never enter Postgres (a single verbose CI job emits tens of megabytes);
//! only a `build_logs` metadata row does. The bytes go to S3/MinIO, addressed
//! path-style so one endpoint hostname works for both. This module is the thin
//! seam over the S3 client so the rest of the gateway does not depend on the
//! concrete crate.

use std::sync::Arc;

use s3::{Bucket, creds::Credentials, region::Region};

use crate::config::S3Config;

#[derive(Clone)]
pub struct LogStore {
    bucket: Arc<Bucket>,
}

impl LogStore {
    /// Building the client does no network I/O, so a bad endpoint or malformed
    /// credentials fail here at boot rather than on the first upload. Actual
    /// reachability is only discovered when we try to put an object, which is
    /// why uploads are best-effort and never block fact ingestion.
    pub fn new(config: &S3Config) -> Result<Self, String> {
        let region = Region::Custom {
            region: config.region.clone(),
            endpoint: config.endpoint.as_str().trim_end_matches('/').to_string(),
        };
        let credentials = Credentials::new(
            Some(&config.access_key),
            Some(&config.secret_key),
            None,
            None,
            None,
        )
        .map_err(|e| format!("s3 credentials: {e}"))?;
        let bucket = Bucket::new(&config.logs_bucket, region, credentials)
            .map_err(|e| format!("s3 bucket: {e}"))?
            .with_path_style();
        Ok(Self {
            bucket: Arc::new(*bucket),
        })
    }

    pub fn bucket_name(&self) -> &str {
        self.bucket.name.as_str()
    }

    /// Uploads an object and returns its byte length. The caller records the
    /// `build_logs` row only if this succeeds.
    pub async fn put(&self, key: &str, bytes: &[u8], content_type: &str) -> Result<(), String> {
        let response = self
            .bucket
            .put_object_with_content_type(key, bytes, content_type)
            .await
            .map_err(|e| format!("s3 put {key}: {e}"))?;
        let status = response.status_code();
        if !(200..300).contains(&status) {
            return Err(format!("s3 put {key} returned status {status}"));
        }
        Ok(())
    }
}
