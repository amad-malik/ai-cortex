//! CONTRACTS §2's selection check for the RFC 8785 crate (not a BASE case): `canonical_json`
//! reproduces nemo's four JCS vectors byte for byte. The vectors are a copy of lane B's frozen
//! `FIXTURE-VECTORS.json` (sha256 8ca1f16a44d33ffb4ed72bdb69efa1a6249318aad5a72839d5706fe9281c60f2,
//! `/path/to/docs/lane-B-frozen-57b9b2a3/`).
//! nemo's M-1 consumer test and I-1's audit test assert the same vectors on their side.

#[test]
fn canonical_json_reproduces_nemos_four_jcs_vectors() {
    let raw = include_str!("fixtures/FIXTURE-VECTORS.json");
    let doc: serde_json::Value = serde_json::from_str(raw).expect("FIXTURE-VECTORS.json parses");
    let vectors = doc["jcs"].as_array().expect("a `jcs` array");
    assert_eq!(vectors.len(), 4, "nemo's file has four JCS vectors");
    for v in vectors {
        let id = v["id"].as_str().expect("id");
        let input: serde_json::Value =
            serde_json::from_str(v["input_json"].as_str().expect("input_json"))
                .expect("input parses");
        let want = v["canonical_utf8"]
            .as_str()
            .expect("canonical_utf8")
            .as_bytes();
        let got = cortex_types::canonical_json(&input).expect("canonical");
        assert_eq!(
            got,
            want,
            "vector {id}: {} != {}",
            String::from_utf8_lossy(&got),
            String::from_utf8_lossy(want)
        );
    }
}
