//! The spacing, radius, size and type scale (stub values: filled by drive-complete-022).
//! `tokens.rs` stays generated; nobody hand-edits it.
#![allow(dead_code)]

pub mod space {
    pub const S2: f32 = 2.0;
    pub const S4: f32 = 4.0;
    pub const S6: f32 = 6.0;
    pub const S7: f32 = 7.0;
    pub const S8: f32 = 8.0;
    pub const S9: f32 = 9.0;
    pub const S10: f32 = 10.0;
    pub const S12: f32 = 12.0;
    pub const S14: f32 = 14.0;
    pub const S16: f32 = 16.0;
    pub const S18: f32 = 18.0;
    pub const S20: f32 = 20.0;
    pub const S24: f32 = 24.0;
    pub const S28: f32 = 28.0;
    pub const S36: f32 = 36.0;
}

pub mod radius {
    pub const TILE: f32 = 9.0;
    pub const PILL: f32 = 999.0;
    pub const DIALOG: f32 = 12.0;
    pub const PANEL: f32 = 10.0;
}

pub mod size {
    pub const CONTROL_SM: f32 = 24.0;
    pub const CONTROL_MD: f32 = 30.0;
    pub const CONTROL_LG: f32 = 36.0;
    pub const ROW: f32 = 36.0;
    pub const ICON_SM: f32 = 14.0;
    pub const ICON_MD: f32 = 18.0;
}

pub mod text {
    use rune::TextStyle;
    pub const SUB: TextStyle = TextStyle::new(10.5, 400.0).lh(13.5);
}
