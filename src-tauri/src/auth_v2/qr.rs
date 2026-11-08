//! The QR code on "Sign in with another device" (DW-A07), drawn here as SVG so the web view needs no encoder.
//! The link it encodes is the public approval page; it carries no code (task 1734): the approver still types the code.

/// One SVG with a quiet zone of 4 modules, a `viewBox` in modules and `shape-rendering="crispEdges"`. Dark modules
/// are one `path` in `currentColor`, so the page decides the colour.
pub(crate) fn svg(link: &str) -> Option<String> {
    let code = qrcode::QrCode::with_error_correction_level(link.as_bytes(), qrcode::EcLevel::M).ok()?;
    let width = code.width();
    let colors = code.to_colors();
    let quiet = 4;
    let size = width + 2 * quiet;
    let mut d = String::new();
    for (i, c) in colors.iter().enumerate() {
        if *c == qrcode::Color::Dark {
            let (x, y) = (i % width + quiet, i / width + quiet);
            d.push_str(&format!("M{x} {y}h1v1h-1z"));
        }
    }
    Some(format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {size} {size}" shape-rendering="crispEdges" role="img" aria-label="QR code"><path fill="currentColor" d="{d}"/></svg>"#
    ))
}

#[cfg(test)]
mod tests {
    #[test]
    fn draws_one_path_with_modules_and_no_code() {
        let svg = super::svg("https://app.beebeeb.io/cli-auth").expect("encodes");
        assert_eq!(svg.matches("<path").count(), 1);
        assert!(svg.matches('M').count() > 100, "modules drawn");
        assert!(!svg.contains("cli-auth"), "the link is encoded, not printed");
    }
}
