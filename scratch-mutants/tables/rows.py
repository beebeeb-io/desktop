F = "native/beebeeb-drive-rune/src/frames/office/docs_live.rs"
D = "native/beebeeb-drive-rune/src/frames/office/docs.rs"
ROW = ("    for row_ in first..last {\n", "    for row_ in 0..rows {\n")
SIZE = ("        .filter(|(w, h)| w.is_finite() && h.is_finite() && *w > 0.0 && *h > 0.0 && h / w <= 20.0 && w / h <= 20.0)\n",
        "        .filter(|(w, h)| *w > 0.0 && *h > 0.0)\n")
T = "office_docs_an_absurd_page_size_is_bounded"
MUT = [
    dict(id="LR-rows", crate="native", note="lead 04:31 (1): drop the first..last row restriction alone (walk every tile row of the page)",
         files=[(F,) + ROW], filt=T, expect=[T]),
    dict(id="LR-size", crate="native", note="0b004601's size_pt bound dropped alone (= V5-sizept, again for the pair's control)",
         files=[(D,) + SIZE], filt=T, expect=[T]),
    dict(id="LR-both", crate="native", note="both guards dropped at once",
         files=[(F,) + ROW, (D,) + SIZE], filt=T, expect=[T]),
]
