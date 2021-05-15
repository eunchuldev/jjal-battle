use futures::TryStreamExt;
use rusoto_core::{region::ParseRegionError, Region, RusotoError};
use rusoto_s3::S3;
use rusoto_s3::{PutObjectError, PutObjectRequest, S3Client, StreamingBody};
use std::str::FromStr;
//use tokio_util::{codec, compat::FuturesAsyncReadCompatExt};
use tokio_util::codec;

#[derive(Clone)]
pub struct S3Uploader {
    region: Region,
    s3: S3Client,
    bucket: String,
}

impl S3Uploader {
    pub fn new(
        bucket: &str,
        region: &str,
        endpoint: Option<&str>,
    ) -> Result<Self, ParseRegionError> {
        let region = if let Some(endpoint) = endpoint {
            Region::Custom {
                name: region.to_string(),
                endpoint: endpoint.to_string(),
            }
        } else {
            Region::from_str(region)?
        };

        Ok(Self {
            region: region.to_owned(),
            s3: S3Client::new(region),
            bucket: bucket.to_string(),
        })
    }

    pub fn url(&self, key: &str) -> String {
        match &self.region {
            Region::Custom { endpoint, .. } => format!("{}/{}/{}", endpoint, self.bucket, key),
            _ => format!(
                "https://{}.s3.{}.amazonaws.com/{}",
                self.bucket,
                self.region.name(),
                key
            ),
        }
    }

    pub async fn put_object(
        &self,
        key: &str,
        file: std::fs::File,
        size: Option<i64>,
        mime_type: Option<String>,
    ) -> Result<String, RusotoError<PutObjectError>> {
        let afile = tokio::fs::File::from_std(file);
        //let stream = codec::FramedRead::new(afile, file.into_async_read().compat(), codec::BytesCodec::new());//.map_ok(|bytes| bytes.freeze());
        let stream = codec::FramedRead::new(afile, codec::BytesCodec::new()).map_ok(|b| b.freeze());
        self.s3
            .put_object(PutObjectRequest {
                bucket: self.bucket.to_string(),
                key: key.to_string(),
                body: Some(StreamingBody::new(stream)),
                content_length: size,
                content_type: mime_type,
                ..Default::default()
            })
            .await?;
        Ok(self.url(key))
    }
}
