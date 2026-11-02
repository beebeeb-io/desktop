//! Data-residency DTOs for `GET`/`PUT /api/v1/me/region`.
//!
//! Storage location is described by country, never by city: the legacy
//! `available_regions` key decodes into `regions`, and a city the server
//! still sends is not read here, so it cannot reach the IPC.

use super::*;

// ── 2b. Data residency / region — GET /api/v1/me/region ───────────────────────
//
// Task 2050 (Guus, 2026-10-19): storage location is described by COUNTRY, never by city. The wire
// key of the legacy list is `available_regions` (alias keeps the desktop IPC name `regions`). The
// server still sends `example_city` for one release; it is NOT read: no field here carries it, so a
// city can neither be decoded nor reach the UI. The country is `example_country`. The server omits
// provider metadata; ignore any legacy provider field rather than forwarding it.

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RegionInfo {
    pub continent: String,
    pub display_name: String,
    /// `example_country` of a server that speaks countries; null from an older one (which sent only
    /// the deprecated `example_city`, never read).
    #[serde(default, alias = "example_country")]
    pub country: Option<String>,
    pub is_default: bool,
}

/// One row of the storage-location list (task 1919, by country since task 2050): `choices` of
/// `GET /api/v1/me/region`, the same array as the public `GET /api/v1/storage-regions`. The server
/// owns every field. `id` is sent back verbatim as `preferred_region` (`europe` or a lower-case
/// country code). There is no city, provider, endpoint or pool id in this struct: a `city` key an
/// older server still sends is ignored here and absent from the IPC.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct StorageChoice {
    pub id: String,
    /// `region` (the "Europe" default: any data center), `country`, or `datacenter` (an older
    /// server's one-row-per-datacenter list, folded into countries by the UI).
    pub kind: String,
    /// `Europe`, or the country's name (`Germany`).
    pub name: String,
    #[serde(default)]
    pub country: Option<String>,
    #[serde(default)]
    pub country_code: Option<String>,
    /// `live` or `coming_soon`.
    pub status: String,
    pub selectable: bool,
    pub is_default: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct UserRegionResponse {
    /// `europe`, a live country id (`de`, `fi`), or null (no choice: placed like `europe`). An older
    /// server may still hold a datacenter id here.
    #[serde(default)]
    pub preferred_region: Option<String>,
    /// "Only store in <country>" (task 1952): when the chosen country cannot take a new file the
    /// upload stops instead of going to another EU country. False from a server that predates it.
    #[serde(default)]
    pub region_strict: bool,
    #[serde(alias = "available_regions")]
    pub regions: Vec<RegionInfo>,
    /// The storage-location list. A server that predates task 1919 sends none: the IPC then carries
    /// an empty list and the view falls back to `regions`.
    #[serde(default)]
    pub choices: Vec<StorageChoice>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SetPreferredRegionResponse {
    #[serde(default)]
    pub preferred_region: Option<String>,
    #[serde(default)]
    pub region_strict: bool,
}
