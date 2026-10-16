use std::process::Command;

#[test]
fn executable_owner_flow_reports_stages_and_negative_case_fails() {
    let success = Command::new(env!("CARGO_BIN_EXE_withings-runtime-check"))
        .output()
        .expect("run runtime acceptance binary");
    assert!(
        success.status.success(),
        "{}",
        String::from_utf8_lossy(&success.stderr)
    );
    let stdout = String::from_utf8(success.stdout).expect("UTF-8 stage output");
    let expected = [
        "OFFLINE SYNTHETIC NOT HOST0.3",
        "PASS callback → store readback",
        "PASS discover",
        "PASS accept",
        "PASS subscriptions readback",
        "PASS event",
        "PASS drop owner → restore",
        "PASS activate",
        "PASS duplicate silent",
        "PASS refresh final-save failure → delivery blocked",
    ];
    let positions: Vec<_> = expected
        .iter()
        .map(|stage| stdout.find(stage).expect(stage))
        .collect();
    assert!(positions.windows(2).all(|pair| pair[0] < pair[1]));

    let failure = Command::new(env!("CARGO_BIN_EXE_withings-runtime-check"))
        .arg("--break-refresh-response")
        .output()
        .expect("run deliberate failure scenario");
    assert!(!failure.status.success());
    let failed_stdout = String::from_utf8_lossy(&failure.stdout);
    assert!(failed_stdout.contains("PASS duplicate silent"));
    assert!(!failed_stdout.contains("PASS refresh final-save failure"));
    assert!(String::from_utf8_lossy(&failure.stderr).contains("FAIL refresh"));
}
