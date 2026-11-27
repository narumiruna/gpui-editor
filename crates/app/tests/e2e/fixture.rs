use std::fmt::Write;

pub(crate) fn document(size: &str, variant: &str) -> Option<String> {
    if !matches!(size, "small" | "long") || !matches!(variant, "start" | "code" | "latex") {
        return None;
    }
    let mut doc = String::from("# Block editing\n\nControl paragraph.\n\nCode entry\n\n");
    if variant == "code" {
        doc.push_str("```\nlet result = 123456789;\n    return result + 2;\n```\n");
    } else {
        doc.push('\n');
    }
    doc.push_str("\nLeave code here.\n\n$$\n");
    doc.push_str(if variant == "latex" {
        "\\frac{1}{2}"
    } else {
        "x^2"
    });
    doc.push_str("\n$$\n\nLeave formula here.\n");
    if size == "long" {
        // Tilde fences cannot close the backtick fence typed above them.
        for block in 1..=32 {
            writeln!(
                doc,
                "\n## Section {block:02}\n\nUnedited paragraph {block:02}.\n\n~~~"
            )
            .unwrap();
            for line in 1..=22 {
                writeln!(doc, "let value_{line:02} = {};", block * line).unwrap();
            }
            writeln!(doc, "~~~\n\n$$\nx^{{{block}}} + y\n$$").unwrap();
        }
    }
    Some(doc)
}

#[cfg(not(test))]
fn main() -> std::process::ExitCode {
    let mut args = std::env::args().skip(1);
    match (args.next(), args.next(), args.next()) {
        (Some(size), Some(variant), None) => {
            if let Some(doc) = document(&size, &variant) {
                print!("{doc}");
                return std::process::ExitCode::SUCCESS;
            }
        }
        _ => {}
    }
    eprintln!("Usage: fixture {{small|long}} {{start|code|latex}}");
    std::process::ExitCode::from(2)
}
