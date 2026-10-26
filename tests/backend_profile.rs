use clap::Parser;
use geesefs_rs::{backend::s3::S3, config::Config};
use std::{process::Stdio, time::Duration};

/// The child has a whitelist environment, so exercising the real provider chain
/// cannot consume the developer's credentials or invoke their configured tools.
#[tokio::test]
async fn shared_config_profile_and_disabled_unbounded_credential_process() {
    if let Ok(mode) = std::env::var("GEESEFS_PROFILE_CHILD") {
        let endpoint = std::env::var("GEESEFS_PROFILE_ENDPOINT").unwrap();
        let file = std::env::var("GEESEFS_PROFILE_FILE").unwrap();
        let config = Config::try_parse_from([
            "geesefs-rs",
            "-f",
            "--endpoint",
            &endpoint,
            "--http-timeout=2s",
            "--sdk-max-retries=0",
            "--profile=fixture",
            "--shared-config",
            &file,
            "bucket",
            "/tmp/mount",
        ])
        .unwrap();
        let store = S3::from_config(&config).await.unwrap();
        let result = store.head("file").await;
        if mode == "static" {
            result.unwrap();
        } else {
            assert!(
                result.is_err(),
                "unbounded credential_process must not execute"
            );
        }
        return;
    }
    let work = tempfile::tempdir().unwrap();
    for mode in ["static", "process"] {
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}", server.server_addr());
        let task =
            (mode == "static").then(|| {
                std::thread::spawn(move || {
                    let request = server
                        .recv_timeout(Duration::from_secs(5))
                        .unwrap()
                        .expect("missing profile-authenticated request");
                    let auth = request
                        .headers()
                        .iter()
                        .find(|h| h.field.equiv("Authorization"))
                        .unwrap()
                        .value
                        .as_str()
                        .to_owned();
                    request
                        .respond(tiny_http::Response::empty(200).with_header(
                            tiny_http::Header::from_bytes("ETag", "generation").unwrap(),
                        ))
                        .unwrap();
                    auth
                })
            });
        let file = work.path().join("config");
        let marker = work.path().join("must-not-exist");
        let contents = if mode == "static" {
            "[profile fixture]\naws_access_key_id = file-access\naws_secret_access_key = file-secret\n".to_owned()
        } else {
            format!(
                "[profile fixture]\ncredential_process = /usr/bin/timeout 1s /usr/bin/touch {}\n",
                marker.display()
            )
        };
        std::fs::write(&file, contents).unwrap();
        let mut child = tokio::process::Command::new(std::env::current_exe().unwrap());
        child
            .args([
                "--exact",
                "shared_config_profile_and_disabled_unbounded_credential_process",
                "--nocapture",
            ])
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", work.path())
            .env("AWS_ACCESS_KEY_ID", "env-should-not-be-used")
            .env("AWS_SECRET_ACCESS_KEY", "env-fake-secret")
            .env("AWS_EC2_METADATA_DISABLED", "true")
            .env("AWS_CONFIG_FILE", "/dev/null")
            .env("AWS_SHARED_CREDENTIALS_FILE", "/dev/null")
            .env("GEESEFS_PROFILE_CHILD", mode)
            .env("GEESEFS_PROFILE_ENDPOINT", endpoint)
            .env("GEESEFS_PROFILE_FILE", file)
            .stdin(Stdio::null())
            .kill_on_drop(true);
        let output = tokio::time::timeout(Duration::from_secs(10), child.output())
            .await
            .expect("profile child timed out")
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        if let Some(task) = task {
            assert!(task.join().unwrap().contains("Credential=file-access/"));
        }
        assert!(
            !marker.exists(),
            "a configured credential subprocess must not execute"
        );
    }
}
