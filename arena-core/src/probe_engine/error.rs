//! Probe-engine error type and the JS-fixture-script marker parser.

use serde::Deserialize;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ProbeEngineError {
    #[error("undefined variable in template: {0}")]
    UndefinedVariable(String),
    #[error("division by zero in answer template")]
    DivisionByZero,
    #[error("answer template syntax error: {0}")]
    SyntaxError(String),
    #[error("fixture pool is empty for variable: {0}")]
    EmptyPool(String),
    #[error("js evaluation error: {0}")]
    JsEval(String),
    #[error("js fixtures must evaluate to an object")]
    JsFixturesNotObject,
    #[error("js validation must evaluate to a boolean")]
    JsValidationNotBool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct JsFixtureScript {
    pub kind: String,
    pub script: String,
}

pub fn parse_js_fixture_script(raw: &str) -> Option<String> {
    let parsed: JsFixtureScript = serde_json::from_str(raw).ok()?;
    if parsed.kind == "js" {
        Some(parsed.script)
    } else {
        None
    }
}
