//! Probe engine: fixture sampling, command rendering, answer template evaluation.

pub mod answer;
pub mod error;
pub mod fixtures;
pub mod js;
pub mod js_wrap;
pub mod render;

pub use answer::*;
pub use error::*;
pub use fixtures::*;
pub use js::*;
pub use js_wrap::*;
pub use render::*;

/// Shared fixture builders for this module's unit tests.
#[cfg(test)]
pub(crate) mod test_util {
    use crate::probe_engine::fixtures::FixtureSample;
    use crate::task_template::{FixtureDef, FixtureKind};
    use std::collections::HashMap;

    pub(crate) fn numeric(name: &str, min: i64, max: i64) -> FixtureDef {
        FixtureDef {
            name: name.to_string(),
            kind: FixtureKind::NumericRange { min, max },
        }
    }

    pub(crate) fn kv(name: &str, pairs: &[(&str, &str)]) -> FixtureDef {
        FixtureDef {
            name: name.to_string(),
            kind: FixtureKind::KeyValue {
                pairs: pairs
                    .iter()
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect(),
            },
        }
    }

    pub(crate) fn fixed(name: &str, value: &str) -> HashMap<String, FixtureSample> {
        let mut m = HashMap::new();
        m.insert(
            name.to_string(),
            FixtureSample {
                key: None,
                value: value.to_string(),
            },
        );
        m
    }
}
