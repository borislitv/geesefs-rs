#[tokio::test]
async fn paired_evidence_gate_fails_closed_on_missing_changed_or_regressed_results() {
    let output = tokio::time::timeout(
        std::time::Duration::from_secs(30),
        tokio::process::Command::new("python3")
            .kill_on_drop(true)
            .args([
                "-m",
                "unittest",
                "discover",
                "-s",
                "bench",
                "-p",
                "test_*.py",
            ])
            .output(),
    )
    .await
    .expect("performance evidence tests must complete within 30 seconds")
    .expect("python3 required for the performance evidence gate");
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
