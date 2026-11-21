use super::{Outcome, outcome};

#[test]
fn a_small_svg_draws_an_image() {
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><rect width="10" height="10" fill="#f00"/></svg>"##;
    assert!(matches!(outcome(svg), Outcome::Drawn(_)));
}

#[test]
fn links_to_files_or_the_network_turn_the_preview_off() {
    for svg in [
        r#"<svg xmlns="http://www.w3.org/2000/svg"><image href="file:///C:/x.png"/></svg>"#,
        r#"<svg xmlns="http://www.w3.org/2000/svg"><image href="https://x/y.svg"/></svg>"#,
        r#"<svg xmlns:xlink="http://www.w3.org/1999/xlink"><use xlink:href = 'a.svg#b'/></svg>"#,
    ] {
        assert!(matches!(outcome(svg), Outcome::Outside), "{svg}");
    }
}

#[test]
fn inner_and_data_links_are_fine() {
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="4" height="4"><defs><rect id="a" width="4" height="4"/></defs><use xlink:href="#a"/></svg>"##;
    assert!(matches!(outcome(svg), Outcome::Drawn(_)));
}

#[test]
fn broken_svg_could_not_be_drawn() {
    assert!(matches!(outcome("<svg"), Outcome::Invalid));
}
