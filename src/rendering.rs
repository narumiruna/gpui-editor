//! Local raster previews of LaTeX and Mermaid source.

use std::sync::{Arc, OnceLock};

use mmdflux::payload::Diagram;
use mmdflux::{EngineAlgorithmId, OutputFormat, RenderConfig, SvgThemeConfig};
use resvg::tiny_skia::{Pixmap, Transform};
use resvg::usvg::{self, fontdb};

const MAX_MATH_BYTES: usize = 4 * 1024;
const MAX_MERMAID_BYTES: usize = 8 * 1024;
const MAX_SVG_BYTES: usize = 2 * 1024 * 1024;
const MAX_SCALE: f32 = 8.0;
const MAX_SIDE: u32 = 4096;
const MAX_PIXELS: u32 = 4 * 1024 * 1024;

/// Row-major sRGB pixels in blue, green, red, alpha order, with straight alpha.
#[derive(Debug)]
pub struct Raster {
    pub width: u32,
    pub height: u32,
    pub bgra: Vec<u8>,
}

/// Render LaTeX on a transparent background. `display` selects display or inline
/// layout. `scale` is the pixel scale relative to an 18 px math font.
pub fn render_math(source: &str, display: bool, scale: f32) -> Result<Raster, String> {
    check_input(source, MAX_MATH_BYTES, scale)?;
    // Mathra auto-detects MathML in LaTeX mode; this preview only accepts LaTeX.
    if mathra::input::mathml::looks_like_mathml(source) {
        return Err("Expected LaTeX, not MathML".into());
    }
    let options = mathra::Options {
        display,
        strict: true,
        animate: false,
        ..mathra::Options::default()
    };
    let output = mathra::convert(source, &options).map_err(|err| format!("LaTeX: {err}"))?;
    rasterize_svg(&output.svg, scale, false)
}

/// Render a flat Mermaid flowchart at the given pixel scale. Labels use installed
/// system fonts; characters without a matching font may not appear.
pub fn render_mermaid(source: &str, scale: f32) -> Result<Raster, String> {
    check_input(source, MAX_MERMAID_BYTES, scale)?;
    // Skip subgraph syntax before parsing: mmdflux follows cluster references
    // recursively while building its graph, before we can check the result.
    if source.to_ascii_lowercase().contains("subgraph") {
        return Err("Mermaid previews do not support subgraphs".into());
    }
    // Grouped endpoints expand into a Cartesian product during parsing.
    if source.bytes().filter(|byte| *byte == b'&').count() > 32 {
        return Err("Too many Mermaid group operators".into());
    }
    check_flowchart(source)?;
    let config = RenderConfig {
        layout_engine: Some(EngineAlgorithmId::FLUX_LAYERED),
        // Use the built-in static theme rather than document theme directives.
        svg_theme: Some(SvgThemeConfig::default()),
        ..RenderConfig::default()
    };
    let svg = mmdflux::render_diagram(source, OutputFormat::Svg, &config)
        .map_err(|err| format!("Mermaid: {err}"))?;
    rasterize_svg(&svg, scale, true)
}

fn check_input(source: &str, max_bytes: usize, scale: f32) -> Result<(), String> {
    if !scale.is_finite() || scale <= 0.0 || scale > MAX_SCALE {
        return Err(format!(
            "Scale must be greater than zero and at most {MAX_SCALE}"
        ));
    }
    if source.len() > max_bytes {
        return Err(format!("Source exceeds the {max_bytes}-byte limit"));
    }
    if source.trim().is_empty() {
        return Err("Source is empty".into());
    }
    Ok(())
}

fn check_flowchart(source: &str) -> Result<(), String> {
    // The rendering facade parses again. Inspect the graph first to bound layout
    // work, without allowing the facade to auto-detect an MMDS document.
    let registry = mmdflux::builtins::default_registry();
    let Some(instance) = registry
        .detect(source)
        .filter(|id| *id == "flowchart")
        .and_then(|id| registry.create(id))
    else {
        return Err("Expected a Mermaid flowchart (graph or flowchart)".into());
    };
    let parsed = instance
        .parse(source)
        .map_err(|err| format!("Mermaid: {err}"))?;
    let Diagram::Flowchart(graph) = parsed
        .into_payload()
        .map_err(|err| format!("Mermaid: {err}"))?
    else {
        return Err("Expected a Mermaid flowchart".into());
    };
    if graph.nodes.is_empty() {
        return Err("Mermaid flowchart has no nodes".into());
    }
    // Extra dashes and long chains can produce many layout ranks and dummy nodes.
    if graph.nodes.len() > 128
        || graph.edges.len() > 256
        || graph.edges.iter().map(|edge| edge.minlen).sum::<i32>() > 512
    {
        return Err("Mermaid flowchart exceeds the layout complexity limit".into());
    }
    Ok(())
}

fn system_fonts() -> Arc<fontdb::Database> {
    static FONTS: OnceLock<Arc<fontdb::Database>> = OnceLock::new();
    Arc::clone(FONTS.get_or_init(|| {
        let mut fonts = fontdb::Database::new();
        // The default macOS scan includes /Network/Library/Fonts. Use only
        // standard local directories, with no paths supplied by the document.
        #[cfg(target_os = "macos")]
        {
            fonts.load_fonts_dir("/System/Library/Fonts");
            fonts.load_fonts_dir("/Library/Fonts");
            if let Some(home) = std::env::var_os("HOME") {
                fonts.load_fonts_dir(std::path::PathBuf::from(home).join("Library/Fonts"));
            }
        }
        #[cfg(not(target_os = "macos"))]
        fonts.load_system_fonts();

        // LastResort claims most Unicode characters, but draws placeholders.
        let placeholders: Vec<_> = fonts
            .faces()
            .filter(|face| face.post_script_name == "LastResort")
            .map(|face| face.id)
            .collect();
        for id in placeholders {
            fonts.remove_face(id);
        }
        // Ensure generic sans-serif resolves even without Microsoft fonts.
        if let Some(family) = ["Arial", "DejaVu Sans", "Liberation Sans", "Noto Sans"]
            .into_iter()
            .find(|family| {
                fonts
                    .faces()
                    .any(|face| face.families.iter().any(|(name, _)| name == family))
            })
            .map(str::to_owned)
            .or_else(|| {
                fonts
                    .faces()
                    .next()?
                    .families
                    .first()
                    .map(|(name, _)| name.clone())
            })
        {
            fonts.set_sans_serif_family(family);
        }
        Arc::new(fonts)
    }))
}

fn rasterize_svg(svg: &str, scale: f32, with_labels: bool) -> Result<Raster, String> {
    if svg.len() > MAX_SVG_BYTES {
        return Err("Generated SVG exceeds the size limit".into());
    }
    let xml = roxmltree::Document::parse_with_options(
        svg,
        roxmltree::ParsingOptions {
            allow_dtd: false,
            nodes_limit: 32_768,
        },
    )
    .map_err(|error| format!("SVG: {error}"))?;
    // resvg paints SVG on the CPU: no browser, scripts, or CSS resource loading.
    // Deny all image references. resources_dir: None alone does not block files.
    let mut options = usvg::Options {
        resources_dir: None,
        image_href_resolver: usvg::ImageHrefResolver {
            resolve_data: Box::new(|_, _, _| None),
            resolve_string: Box::new(|_, _| None),
        },
        font_size: 18.0,
        ..usvg::Options::default()
    };
    if with_labels {
        options.fontdb = system_fonts();
    }
    let tree = usvg::Tree::from_xmltree(&xml, &options).map_err(|err| format!("SVG: {err}"))?;
    let width = pixel_extent(tree.size().width(), scale)?;
    let height = pixel_extent(tree.size().height(), scale)?;
    if width * height > MAX_PIXELS {
        return Err("Raster exceeds the pixel count limit".into());
    }
    let mut pixmap = Pixmap::new(width, height).ok_or("Could not allocate raster")?;
    resvg::render(
        &tree,
        Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );

    // tiny-skia stores premultiplied RGBA. Convert in place to straight BGRA.
    let mut bgra = pixmap.take();
    for pixel in bgra.chunks_exact_mut(4) {
        let [r, g, b, a] = [pixel[0], pixel[1], pixel[2], pixel[3]];
        if a == 0 || a == 255 {
            pixel.swap(0, 2);
        } else {
            let alpha = u16::from(a);
            let straight = |color| ((u16::from(color) * 255 + alpha / 2) / alpha) as u8;
            pixel.copy_from_slice(&[straight(b), straight(g), straight(r), a]);
        }
    }
    Ok(Raster {
        width,
        height,
        bgra,
    })
}

fn pixel_extent(size: f32, scale: f32) -> Result<u32, String> {
    let extent = (f64::from(size) * f64::from(scale)).ceil();
    if (1.0..=f64::from(MAX_SIDE)).contains(&extent) {
        Ok(extent as u32)
    } else {
        Err(format!(
            "Raster side must be between 1 and {MAX_SIDE} pixels"
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raster_is_straight_bgra_and_preserves_transparency() {
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="3" height="1">
            <rect x="0" width="1" height="1" fill="#00ffcc"/>
            <rect x="1" width="1" height="1" fill="#ff0000" opacity="0.5"/>
        </svg>"##;
        let raster = rasterize_svg(svg, 1.0, false).unwrap();
        assert_eq!(raster.bgra, [204, 255, 0, 255, 0, 0, 255, 128, 0, 0, 0, 0]);
    }

    #[test]
    fn svg_ignores_embedded_image_and_scripts() {
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="2" height="2">
            <image width="2" height="2" href="data:image/svg+xml,%3Csvg%20xmlns='http://www.w3.org/2000/svg'%20width='2'%20height='2'%3E%3Crect%20width='2'%20height='2'/%3E%3C/svg%3E"/>
            <script>document.documentElement.appendChild(document.createElement('rect'))</script>
        </svg>"#;
        let raster = rasterize_svg(svg, 1.0, false).unwrap();
        assert!(raster.bgra.iter().all(|&channel| channel == 0));
    }

    #[test]
    fn large_raster_area_is_rejected() {
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="3000" height="3000"/>"#;
        assert!(
            rasterize_svg(svg, 1.0, false)
                .unwrap_err()
                .contains("pixel count")
        );
    }
}
