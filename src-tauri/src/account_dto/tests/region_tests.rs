//! Region and storage-choice decoding tests.
//!
//! These fixtures follow the server contract for `GET`/`PUT /me/region`,
//! including older responses, the country `choices` shape and malformed
//! payloads that must be rejected.

use super::*;

    #[test]
    fn user_region_parses_full_shape() {
        let json = r#"{
            "preferred_region": "europe",
            "regions": [
                {
                    "continent": "europe",
                    "display_name": "Europe",
                    "example_country": "Germany",
                    "provider": "Never Display Storage Provider",
                    "is_default": true
                }
            ]
        }"#;
        let r: UserRegionResponse = serde_json::from_str(json).unwrap();
        assert_eq!(r.preferred_region.as_deref(), Some("europe"));
        assert_eq!(r.regions.len(), 1);
        assert_eq!(r.regions[0].country.as_deref(), Some("Germany"));
        assert!(r.regions[0].is_default);
        assert!(!r.region_strict, "a response without region_strict is not strict");
    }

    #[test]
    fn user_region_null_preferred_parses() {
        // No preferred region set → preferred_region is null; the caller falls
        // back to the default region's continent.
        let json = r#"{
            "preferred_region": null,
            "regions": [
                {
                    "continent": "europe",
                    "display_name": "Europe",
                    "provider": "Never Display Storage Provider",
                    "is_default": true
                }
            ]
        }"#;
        let r: UserRegionResponse = serde_json::from_str(json).unwrap();
        assert_eq!(r.preferred_region, None);
        assert_eq!(r.regions[0].continent, "europe");
        assert_eq!(r.regions[0].country, None);
    }

    // Public API response fixture without provider metadata, from a server that predates task
    // 2050: it sends only the deprecated `example_city`. That key is not read, so it cannot reach the IPC.
    #[test]
    fn region_contract_decodes_older_server_response_and_drops_its_city() {
        let response: UserRegionResponse =
            serde_json::from_str(include_str!("../../../tests/fixtures/region-api-older-server.json"))
                .expect("an older server's response must decode without provider and without a country");
        assert_eq!(response.preferred_region, None);
        assert_eq!(response.regions.len(), 1);
        let region = &response.regions[0];
        assert_eq!(region.continent, "europe");
        assert_eq!(region.display_name, "Europe");
        assert_eq!(region.country, None, "example_city is never turned into a country");
        assert!(region.is_default);
        // An older server sends no `choices`: the IPC carries an empty list, never an invented one.
        assert!(response.choices.is_empty());
        assert_eq!(
            serde_json::to_value(response).unwrap(),
            serde_json::json!({
                "preferred_region": null,
                "region_strict": false,
                "regions": [{"continent": "europe", "display_name": "Europe", "country": null, "is_default": true}],
                "choices": []
            })
        );
    }

    // Task 2050: the server's country shape. Both the legacy list and `choices` are read by country; the
    // `example_city` the server still sends for one release is dropped.
    #[test]
    fn region_country_contract_decodes_and_never_forwards_a_city() {
        let response: UserRegionResponse =
            serde_json::from_str(include_str!("../../../tests/fixtures/region-api-country.json")).unwrap();
        assert_eq!(response.regions[0].country.as_deref(), Some("Germany"));
        let ipc = serde_json::to_string(&response).unwrap();
        assert!(!ipc.contains("Local") && !ipc.contains("city"), "{ipc}");
    }

    // Task 2050: `GET /me/region` carries `choices` by country. The fixture follows the server contract
    // (server PR #251, regions.rs at aead1d01), not a live capture: the server is not deployed yet.
    #[test]
    fn region_choices_decode_country_response() {
        let response: UserRegionResponse =
            serde_json::from_str(include_str!("../../../tests/fixtures/region-choices-country-api.json"))
                .expect("the /me/region country response must decode");
        assert_eq!(response.preferred_region, None);
        assert!(!response.region_strict);
        let ids: Vec<&str> = response.choices.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids, ["europe", "de", "fi", "nl"]);

        let europe = &response.choices[0];
        assert_eq!(europe.kind, "region");
        assert_eq!(europe.name, "Europe");
        assert!(europe.is_default && europe.selectable);
        assert_eq!(
            (europe.country.as_deref(), europe.country_code.as_deref()),
            (None, None)
        );

        let finland = &response.choices[2];
        assert_eq!(finland.kind, "country");
        assert_eq!(finland.name, "Finland");
        assert_eq!(finland.country.as_deref(), Some("Finland"));
        assert_eq!(finland.country_code.as_deref(), Some("FI"));
        assert!(finland.selectable && !finland.is_default);

        let nl = &response.choices[3];
        assert_eq!(nl.status, "coming_soon");
        assert!(!nl.selectable, "a coming-soon country is listed, never selectable");

        // Only the Europe entry is the default.
        assert_eq!(response.choices.iter().filter(|c| c.is_default).count(), 1);
    }

    // An older server (task 1919): one `datacenter` row per datacenter, each with a `city`. It decodes, and the
    // city is not in the IPC. The UI folds these rows into countries.
    #[test]
    fn region_choices_decode_older_server_datacenters_and_drop_the_city() {
        let response: UserRegionResponse =
            serde_json::from_str(include_str!("../../../tests/fixtures/region-choices-older-server-api.json"))
                .expect("an older server's captured /me/region response must decode");
        let ids: Vec<&str> = response.choices.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids, ["europe", "fsn1", "hel1", "nbg1", "ede"]);
        let hel = &response.choices[2];
        assert_eq!(hel.kind, "datacenter");
        assert_eq!(hel.country.as_deref(), Some("Finland"));
        assert_eq!(hel.country_code.as_deref(), Some("FI"));
        let ipc = serde_json::to_string(&response).unwrap();
        assert!(!ipc.contains("\"city\""), "{ipc}");
    }

    #[test]
    fn region_strict_decodes_on_the_get_and_the_put_response() {
        let get: UserRegionResponse = serde_json::from_str(
            r#"{"preferred_region":"de","region_strict":true,"available_regions":[],"choices":[]}"#,
        )
        .unwrap();
        assert!(get.region_strict);
        assert_eq!(
            serde_json::to_value(&get).unwrap()["region_strict"],
            serde_json::json!(true)
        );
        let put: SetPreferredRegionResponse =
            serde_json::from_str(r#"{"preferred_region":"de","region_strict":true}"#).unwrap();
        assert!(put.region_strict);
        let older: SetPreferredRegionResponse = serde_json::from_str(r#"{"preferred_region":"europe"}"#).unwrap();
        assert!(!older.region_strict);
    }

    #[test]
    fn region_choices_never_forward_provider_or_endpoint_to_the_ipc() {
        let mut json: serde_json::Value =
            serde_json::from_str(include_str!("../../../tests/fixtures/region-choices-country-api.json")).unwrap();
        json["choices"][1]["provider"] = serde_json::json!("Never Display Storage Provider");
        json["choices"][1]["endpoint"] = serde_json::json!("https://never.example");
        json["choices"][1]["city"] = serde_json::json!("Never Display City");
        let response: UserRegionResponse = serde_json::from_value(json).unwrap();
        let ipc = serde_json::to_string(&response).unwrap();
        assert!(!ipc.contains("Never Display"), "{ipc}");
        assert!(!ipc.contains("never.example"), "{ipc}");
        assert!(
            !ipc.contains("provider") && !ipc.contains("endpoint") && !ipc.contains("city"),
            "{ipc}"
        );
    }

    #[test]
    fn region_choices_reject_malformed_entries() {
        for json in [
            r#"{"preferred_region":null,"available_regions":[],"choices":"europe"}"#,
            r#"{"preferred_region":null,"available_regions":[],"choices":[{"kind":"region"}]}"#,
            r#"{"preferred_region":null,"available_regions":[],"choices":[{"id":"x","kind":"region","name":"X"}]}"#,
        ] {
            assert!(
                serde_json::from_str::<UserRegionResponse>(json).is_err(),
                "{json} must not decode"
            );
        }
    }

    #[test]
    fn region_contract_ignores_legacy_provider_in_ipc() {
        let mut json: serde_json::Value =
            serde_json::from_str(include_str!("../../../tests/fixtures/region-api-country.json")).unwrap();
        json["available_regions"][0]["provider"] = serde_json::json!("Never Display Storage Provider");
        let response: UserRegionResponse = serde_json::from_value(json).unwrap();
        let ipc = serde_json::to_value(response).unwrap();
        assert!(ipc["regions"][0].get("provider").is_none());
    }

    #[test]
    fn region_contract_rejects_malformed_success_bodies() {
        for json in [
            r#"{}"#,
            r#"{"error":"unavailable"}"#,
            r#"{"preferred_region":null,"available_regions":null}"#,
            r#"{"available_regions":[{"continent":"europe"}]}"#,
            r#"{"available_regions":"europe"}"#,
        ] {
            assert!(
                serde_json::from_str::<UserRegionResponse>(json).is_err(),
                "accepted {json}"
            );
        }
        let empty: UserRegionResponse =
            serde_json::from_str(r#"{"preferred_region":null,"available_regions":[]}"#).unwrap();
        assert!(empty.regions.is_empty());
    }

    #[test]
    fn user_region_parses_server_available_regions_alias() {
        let json = r#"{
            "preferred_region": null,
            "available_regions": [
                {
                    "continent": "europe",
                    "display_name": "Europe",
                    "example_city": "Falkenstein",
                    "example_country": "Germany",
                    "provider": "Never Display Storage Provider",
                    "is_default": true
                }
            ]
        }"#;
        let r: UserRegionResponse = serde_json::from_str(json).unwrap();
        assert_eq!(r.preferred_region, None);
        assert_eq!(r.regions.len(), 1);
        assert_eq!(r.regions[0].country.as_deref(), Some("Germany"));
        assert!(r.regions[0].is_default);
        assert!(!serde_json::to_string(&r).unwrap().contains("Falkenstein"));
    }
