use std::process::Command;

#[test]
fn help_explains_how_to_start_without_a_path() {
    let output = Command::new(env!("CARGO_BIN_EXE_gpui-editor"))
        .arg("--help")
        .output()
        .expect("launch the built CLI");

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("Usage: gpui-editor\n"));
    assert!(stdout.contains("Choose a folder inside the app"));
}

#[test]
fn path_arguments_do_not_start_the_app() {
    let output = Command::new(env!("CARGO_BIN_EXE_gpui-editor"))
        .arg(".")
        .output()
        .expect("launch the built CLI");

    assert_eq!(output.status.code(), Some(2));
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("does not accept file or folder paths")
    );
}

#[cfg(not(target_os = "macos"))]
#[test]
fn reports_that_the_desktop_app_requires_macos() {
    let output = Command::new(env!("CARGO_BIN_EXE_gpui-editor"))
        .output()
        .expect("launch the built CLI");
    assert!(!output.status.success());
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("requires macOS")
    );
}
