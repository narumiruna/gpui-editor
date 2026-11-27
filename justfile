set windows-shell := ["cmd.exe", "/C"]

[unix]
run:
    DEVELOPER_DIR="${DEVELOPER_DIR:-/Applications/Xcode.app/Contents/Developer}" cargo run --locked

[windows]
run:
    cargo run --locked
