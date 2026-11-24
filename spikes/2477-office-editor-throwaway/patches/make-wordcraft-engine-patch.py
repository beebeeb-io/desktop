import re, sys
root = sys.argv[1]
p = root + "/crates/engine/src/packs.rs"
lines = open(p).read().split("\n")
NS = '#[cfg(all(not(target_arch = "wasm32"), feature = "font-store"))]'
W = '#[cfg(any(target_arch = "wasm32", not(feature = "font-store")))]'
n = 0
for i, l in enumerate(lines[:180]):
    new = l.replace('#[cfg(not(target_arch = "wasm32"))]', NS).replace('#[cfg(target_arch = "wasm32")]', W)
    if new != l:
        lines[i] = new; n += 1
open(p, "w").write("\n".join(lines))
c = root + "/crates/engine/Cargo.toml"
t = open(c).read()
a = 'craft-font-store = { workspace = true }\ncraft-font-catalog = { workspace = true }'
assert a in t
t = t.replace(a, 'craft-font-store = { workspace = true, optional = true }\ncraft-font-catalog = { workspace = true, optional = true }')
a = 'test-support = []'
assert a in t
t = t.replace(a, a + '\n# Beebeeb patch: the font / language-pack downloader (ureq, rustls) is opt-out. Upstream behaviour is the default.\ndefault = ["font-store"]\nfont-store = ["dep:craft-font-store", "dep:craft-font-catalog"]')
open(c, "w").write(t)
print("packs.rs lines changed:", n)
