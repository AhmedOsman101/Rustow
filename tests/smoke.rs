#[test]
fn help_exits_zero() {
  assert_cmd::Command::cargo_bin("rustow")
    .unwrap()
    .arg("--help")
    .assert()
    .success();
}
