#[tokio::test]
async fn explicitly_requested_unavailable_mounted_verification_is_a_failure() {
    let directory =
        std::env::temp_dir().join(format!("geesefs-mounted-preflight-{}", std::process::id()));
    let output = tokio::time::timeout(
        std::time::Duration::from_secs(30),
        tokio::process::Command::new("python3")
            .kill_on_drop(true)
            .args([
                "scripts/integration.py",
                "--rust-only",
                "--rust-binary",
                "/nonexistent/geesefs-rs",
                "--output",
            ])
            .arg(&directory)
            .output(),
    )
    .await
    .expect("mounted preflight must complete within 30 seconds")
    .unwrap();
    assert!(!output.status.success());
    let report = std::fs::read_to_string(directory.join("report.json")).unwrap();
    assert!(report.contains("build the Rust binary"));
    assert!(report.contains("\"accepted\": false"));
    std::fs::remove_dir_all(directory).unwrap();
}
