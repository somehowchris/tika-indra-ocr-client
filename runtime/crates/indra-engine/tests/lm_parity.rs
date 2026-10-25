//! Bit-exact parity of the language model and beam search with indra_ocr.lm 1.1.2.
//!
//! The fixtures in `data/lm/` were produced by `data/lm/generate.py` running the
//! unmodified reference on the v1.1.0 archives: sampled `logp_keys` values for
//! every model, and every `beam_search` call the pipeline made on five pages
//! (smoke, two-page TIFF, the synthetic math page, the two Hub example pages).
//! JSON numbers are Python float reprs; serde_json's `float_roundtrip` keeps them exact.
use indra_engine::gates::{BeamScratch, LanguageModel, LmParams, MAX_TOKENS, beam_search};

#[allow(dead_code)]
mod support;

static LOGP: [(&str, &str); 3] = [
    ("lm.npz", include_str!("data/lm/lm-logp-lm.json")),
    ("lm_cjk.npz", include_str!("data/lm/lm-logp-lm_cjk.json")),
    (
        "lm_world.npz",
        include_str!("data/lm/lm-logp-lm_world.json"),
    ),
];
static BEAM: &str = include_str!("data/lm/lm-beam-cases.json");

fn model(name: &str) -> LanguageModel {
    LanguageModel::load(&support::directory().join(name)).expect("verified language model")
}

#[test]
#[ignore = "requires INDRA_TEST_MODEL_DIR with the v1.1.0 language models"]
fn logp_matches_logp_keys_bit_for_bit() {
    for (name, fixture) in LOGP {
        let fixture: serde_json::Value = serde_json::from_str(fixture).expect("fixture");
        let model = model(name);
        assert_eq!(
            model.order(),
            fixture["order"].as_u64().expect("order") as usize
        );
        assert_eq!(model.bits(), fixture["bits"].as_u64().expect("bits") as u32);
        assert_eq!(
            model.max_logp().to_bits(),
            fixture["max_logp"].as_f64().expect("max_logp").to_bits()
        );
        let keys = fixture["keys"].as_array().expect("keys");
        let values = fixture["values"].as_array().expect("values");
        assert_eq!(keys.len(), values.len());
        for (key, value) in keys.iter().zip(values) {
            let key =
                u64::from_str_radix(key.as_str().expect("hex key").trim_start_matches("0x"), 16)
                    .expect("key");
            let expected = value.as_f64().expect("value");
            assert_eq!(
                model.logp(key).to_bits(),
                expected.to_bits(),
                "{name} key {key:#x}: {} vs {expected}",
                model.logp(key)
            );
        }
        println!("{name}: {} logp keys bit-exact", keys.len());
    }
}

#[test]
#[ignore = "requires INDRA_TEST_MODEL_DIR with the v1.1.0 language models"]
fn beam_search_matches_every_recorded_pipeline_call() {
    let cases: Vec<serde_json::Value> = serde_json::from_str(BEAM).expect("fixture");
    // The weights each recogniser's searches must have been recorded with:
    // pipeline.py's LM_ALPHA and LM_BETA for the base model and meta.json
    // lm_params for the CJK and world models.
    let models = [
        ("lm.npz", model("lm.npz"), LmParams::BASE),
        ("lm_cjk.npz", model("lm_cjk.npz"), LmParams::GROUP),
        ("lm_world.npz", model("lm_world.npz"), LmParams::GROUP),
    ];
    let mut scratch = BeamScratch::new();
    let mut calls = [0_usize; 3];
    for case in &cases {
        let name = case["model"].as_str().expect("model");
        let index = models
            .iter()
            .position(|(n, _, _)| *n == name)
            .expect("known model");
        let (_, model, weights) = &models[index];
        calls[index] += 1;
        assert_eq!(case["alpha"].as_f64(), Some(weights.alpha), "{name} alpha");
        assert_eq!(case["beta"].as_f64(), Some(weights.beta), "{name} beta");
        assert_eq!(case["beam"].as_u64(), Some(8));
        assert_eq!(case["blank_skip"].as_f64(), Some(0.999));
        assert_eq!(case["cand_floor"].as_f64(), Some(3e-3));
        let floats = |key: &str| -> Vec<f32> {
            case[key]
                .as_array()
                .expect(key)
                .iter()
                .flat_map(|row| match row.as_array() {
                    Some(row) => row
                        .iter()
                        .map(|v| v.as_f64().expect("f32") as f32)
                        .collect(),
                    None => vec![row.as_f64().expect("f32") as f32],
                })
                .collect()
        };
        let blank = floats("blank");
        let topv = floats("topv");
        let topi: Vec<i64> = case["topi"]
            .as_array()
            .expect("topi")
            .iter()
            .flat_map(|row| {
                row.as_array()
                    .expect("row")
                    .iter()
                    .map(|v| v.as_i64().expect("id"))
            })
            .collect();
        let params = LmParams {
            alpha: case["alpha"].as_f64().expect("alpha"),
            beta: case["beta"].as_f64().expect("beta"),
            gate: 1.0,
        };
        let mut ids = [0_i32; MAX_TOKENS];
        let mut steps = [0_u16; MAX_TOKENS];
        let count = beam_search(
            &blank,
            &topv,
            &topi,
            model,
            params,
            &mut scratch,
            &mut ids,
            &mut steps,
        )
        .expect("search");
        let expected_ids: Vec<i32> = case["ids"]
            .as_array()
            .expect("ids")
            .iter()
            .map(|v| v.as_i64().expect("id") as i32)
            .collect();
        let expected_steps: Vec<u16> = case["steps"]
            .as_array()
            .expect("steps")
            .iter()
            .map(|v| v.as_i64().expect("step") as u16)
            .collect();
        assert_eq!(
            &ids[..count],
            &expected_ids[..],
            "{} {name} ids",
            case["image"]
        );
        assert_eq!(
            &steps[..count],
            &expected_steps[..],
            "{} {name} steps",
            case["image"]
        );
    }
    // generate.py hooks beam_search inside the reference pipeline; if the hook
    // stops intercepting it records nothing. The five pages make 15 calls:
    // 8 with lm.npz, 4 with lm_cjk.npz and 3 with lm_world.npz.
    assert_eq!(calls, [8, 4, 3], "recorded beam searches per model");
    println!("{} recorded beam searches identical", cases.len());
}
