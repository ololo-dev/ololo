//! Fixture sampling: draw one concrete value per fixture definition.

use std::collections::HashMap;

use rand::Rng;

use crate::probe_engine::error::ProbeEngineError;
use crate::task_template::{FixtureDef, FixtureKind};

/// The result of sampling one fixture.
///
/// For `NumericRange` fixtures, `key` is `None` and `value` holds the sampled number
/// as a decimal string.  For `KeyValue` fixtures, `key` holds the randomly selected
/// map key (e.g. `"France"`) and `value` holds the associated value (e.g. `"Paris"`).
#[derive(Debug, Clone)]
pub struct FixtureSample {
    pub key: Option<String>,
    pub value: String,
}

/// Sample one concrete value per fixture definition.
///
/// Returns a map of variable-name → [`FixtureSample`].
pub fn sample_fixtures(
    defs: &[FixtureDef],
    rng: &mut impl Rng,
) -> Result<HashMap<String, FixtureSample>, ProbeEngineError> {
    let mut out = HashMap::with_capacity(defs.len());
    for def in defs {
        let sample = match &def.kind {
            FixtureKind::NumericRange { min, max } => {
                let v = rng.gen_range(*min..=*max);
                FixtureSample {
                    key: None,
                    value: v.to_string(),
                }
            }
            FixtureKind::KeyValue { pairs } => {
                if pairs.is_empty() {
                    return Err(ProbeEngineError::EmptyPool(def.name.clone()));
                }
                let keys: Vec<&String> = pairs.keys().collect();
                let idx = rng.gen_range(0..keys.len());
                let k = keys[idx].clone();
                let v = pairs[&k].clone();
                FixtureSample {
                    key: Some(k),
                    value: v,
                }
            }
        };
        out.insert(def.name.clone(), sample);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::probe_engine::test_util::{kv, numeric};

    #[test]
    fn numeric_sample_has_no_key_and_parses_in_range() {
        let defs = vec![numeric("n", 1, 10)];
        let mut rng = rand::thread_rng();
        let map = sample_fixtures(&defs, &mut rng).unwrap();
        let s = &map["n"];
        assert!(s.key.is_none());
        let v: i64 = s.value.parse().unwrap();
        assert!((1..=10).contains(&v));
    }

    #[test]
    fn kv_sample_exposes_key_and_value() {
        let defs = vec![kv("country", &[("France", "Paris")])];
        let mut rng = rand::thread_rng();
        let map = sample_fixtures(&defs, &mut rng).unwrap();
        assert_eq!(map["country"].key.as_deref(), Some("France"));
        assert_eq!(map["country"].value, "Paris");
    }

    #[test]
    fn empty_kv_pool_errors() {
        let defs = vec![kv("empty", &[])];
        let mut rng = rand::thread_rng();
        assert!(sample_fixtures(&defs, &mut rng).is_err());
    }
}
