use crate::models::schema::{build_schema, Schema};
use crate::pools::{db, redis, s3};
use testcontainers::{
    clients::Cli,
    core::WaitFor,
    images::{generic::GenericImage, postgres::Postgres, redis::Redis},
    Container, Image,
};

pub struct TestDocker {
    inner: Cli,
}

impl TestDocker {
    pub fn new() -> Self {
        TestDocker {
            inner: Cli::default(),
        }
    }
    pub async fn run<'a>(&'a self) -> TestNodes<'a> {
        TestNodes::new(&self.inner).await
    }
}

pub struct TestNodes<'a> {
    pub pg: Container<'a, Postgres>,
    pub redis: Container<'a, Redis>,
    pub minio: Container<'a, GenericImage>,
    pub pgpool: db::DbPool,
    pub redisurl: String,
    pub redispool: redis::RedisPool,
    pub s3uploader: s3::S3Uploader,
    pub minio_endpoint: String,
    pub minio_bucket: String,
    pub schema: Schema,
}
impl<'a> TestNodes<'a> {
    pub async fn new(docker: &'a Cli) -> TestNodes<'a> {
        let pg = docker.run(Postgres::default());
        let db_host_port = pg.get_host_port(5432);
        let dburl = format!("postgres://postgres@localhost:{}/postgres", db_host_port);
        let dbpool = db::create_pool(&dburl).await.unwrap();

        let redis = docker.run(Redis::default());
        let redis_host_port = redis.get_host_port(6379);
        let redisurl = format!("redis://localhost:{}", redis_host_port);
        let redispool = redis::create_pool(&redisurl).await.unwrap();

        let minio = docker.run(minio_image());
        let minio_host_port = minio.get_host_port(9000);
        let minio_accesskey = "minioadmin";
        let minio_secretkey = "minioadmin";
        std::env::set_var("AWS_ACCESS_KEY_ID", minio_accesskey);
        std::env::set_var("AWS_SECRET_ACCESS_KEY", minio_secretkey);
        let minio_bucket = "bucket";
        let minio_endpoint = format!("http://localhost:{}", minio_host_port);
        /*let minio_region = rusoto_core::Region::Custom {
            name: "".to_string(),
            endpoint: minio_endpoint,
        };*/

        let _mc = docker.run(minio_initialize(
            &minio,
            minio_bucket,
            minio_accesskey,
            minio_secretkey,
        ));

        let s3uploader = s3::S3Uploader::new(minio_bucket, "", Some(&minio_endpoint)).unwrap();

        let mut migrator = sqlx::migrate!();
        migrator
            .migrations
            .to_mut()
            .retain(|migration| !migration.description.ends_with(".down"));
        migrator.run(&dbpool).await.unwrap();

        TestNodes {
            pg,
            redis,
            minio,
            redisurl,
            minio_endpoint,
            minio_bucket: minio_bucket.to_string(),
            pgpool: dbpool.clone(),
            redispool: redispool.clone(),
            s3uploader: s3uploader.clone(),
            schema: build_schema(dbpool, redispool, s3uploader).await.unwrap(),
        }
    }
}

fn minio_image() -> GenericImage {
    GenericImage::new("minio/minio")
        .with_args(["server", "/data"].iter().map(|t| t.to_string()).collect())
        .with_wait_for(WaitFor::message_on_stdout("IAM initialization complete"))
}

fn minio_initialize(
    minio: &Container<'_, GenericImage>,
    bucket: &str,
    accesskey: &str,
    secretkey: &str,
) -> GenericImage {
    GenericImage::new("minio/mc")
        .with_entrypoint("sh")
        .with_env_var("MINIO_IP", minio.get_bridge_ip_address().to_string())
        .with_env_var("MINIO_PORT", minio.get_host_port(9000).to_string())
        .with_env_var("MINIO_ACCESS_KEY", accesskey)
        .with_env_var("MINIO_SECRET_KEY", secretkey)
        .with_env_var("MINIO_BUCKET", bucket)
        .with_args(
            [
                "-c",
                r#"\
  mc config host add myminio http://$MINIO_IP:9000 $MINIO_ACCESS_KEY $MINIO_SECRET_KEY && \
  mc rb --force myminio/$MINIO_BUCKET || true && \
  mc mb myminio/$MINIO_BUCKET && \
  mc policy set public myminio/$MINIO_BUCKET \
                   "#,
            ]
            .iter()
            .map(|t| t.to_string())
            .collect(),
        )
        .with_wait_for(WaitFor::message_on_stdout(
            "Access permission for `myminio/bucket` is set to `public`",
        ))
}

#[actix_rt::test]
async fn test_minio_public_bucket() {
    use reqwest;
    use rusoto_core::Region;
    use rusoto_s3::S3;
    use rusoto_s3::{PutObjectRequest, S3Client};
    let docker = TestDocker::new();
    let db = docker.run().await;
    let s3 = S3Client::new(Region::Custom {
        name: "minio".to_string(),
        endpoint: db.minio_endpoint.clone(),
    });
    s3.put_object(PutObjectRequest {
        bucket: db.minio_bucket.clone(),
        key: "test".to_string(),
        body: Some("hi".to_string().into_bytes().into()),
        ..Default::default()
    })
    .await
    .unwrap();

    let body = reqwest::get(format!(
        "{}/{}/{}",
        db.minio_endpoint, db.minio_bucket, "test"
    ))
    .await
    .unwrap()
    .text()
    .await
    .unwrap();
    assert_eq!(body, "hi");
}
