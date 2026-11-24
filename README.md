# GPUI Editor

GPUI Editor is a local Markdown editor for macOS. It uses GPUI to show formatted text, images, links, Mermaid flowcharts, and LaTeX formulas in one editing surface. It saves Markdown without rewriting parts you did not edit.

## Build and run on macOS

Install Rust and Xcode. The native build was verified on Apple Silicon with macOS 26.4.1, Rust 1.95.0, and Xcode 26.6. Other configurations are not yet verified. GPUI compiles shaders at runtime, so the separate Metal compiler component is not required.

From the repository root, with Xcode at its default location:

```sh
export DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer
cargo build --locked
./target/debug/gpui-editor
```

The command takes no path. In the window, choose **Open Folder…**, then select a Markdown file from the sidebar. Use **New** for a blank document and **Save** or `Cmd-S` to save. The sidebar lists only the workspace's top-level `.md` files.

- Use the editor toolbar to format a selection or insert an item. Click front matter, a diagram, or a formula to edit its source. Click outside the item to preview the change; this does not save the file.
- For links, hover to see the full destination in the bottom bar. Click to open the web browser or follow an in-workspace `.md` link. To edit a link without opening it, place the caret at its edge and choose **Edit link**.
- Image previews support local or HTTPS PNG, JPEG, GIF, and WebP still images. Relative paths are based on the document's folder. HTTPS previews load directly from the stated host; redirects are not followed.
- The editor supports a subset of Markdown. The Mermaid renderer supports flat flowcharts; unsupported blocks such as GFM tables are shown as read-only source. If a saved file changed outside the editor, saving reports a conflict instead of overwriting it.

LaTeX previews use STIX Two Math through Mathra. See the [STIX Two Math license](docs/licenses/stix-two-math.txt).

## Checks

```sh
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked --all-targets
prek run --all-files
```

GitHub Actions runs Rust checks with Rust 1.95.0 on Linux and macOS. The Linux job checks the portable document, rendering, and file-handling code; only macOS builds the GPUI application. Automated GPUI tests use a simulated window, not the real macOS input method. See the [manual acceptance steps](docs/prd/gpui-editor.md#macos-原生驗收) for the remaining native checks.

[Changenote](https://github.com/changenote/changenote) uses [`.changenote/`](.changenote/) to maintain release notes and a release pull request. Changes to the editor do not require a manual release workflow run.
