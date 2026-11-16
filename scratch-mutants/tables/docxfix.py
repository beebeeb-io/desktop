F = "wordcraft/crates/docx/src/read/story/drawing.rs"
MUT = [
    dict(id="DX-1", crate="office", note="ov-docx-fix 453a9249 (lead 04:33): read_pic takes the first a:blip anywhere again (pic.find)",
         files=[(F, '        let blip = find_chosen(pic, "a:blip")?;\n', '        let blip = pic.find("a:blip")?;\n')],
         cmd="cargo test --locked --test docx_alternate_content",
         expect=["a_choice_that_needs_an_unknown_namespace_gives_way_to_the_fallback", "bug51170_every_page_renders_with_nothing_missing"]),
]
