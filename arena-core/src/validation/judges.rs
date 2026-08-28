pub fn validate_rating_scale(value: &serde_json::Value) -> Result<(), String> {
    let obj = value
        .as_object()
        .ok_or_else(|| "rating_scale must be an object".to_string())?;

    let min = get_number(obj, "min")?;
    let max = get_number(obj, "max")?;
    let step = get_number(obj, "step")?;

    if min >= max {
        return Err("rating_scale.min must be < max".to_string());
    }
    if step <= 0.0 {
        return Err("rating_scale.step must be > 0".to_string());
    }

    let steps = (max - min) / step;
    if (steps - steps.round()).abs() >= 1e-6 {
        return Err("rating_scale range must be evenly divisible by step".to_string());
    }

    Ok(())
}

fn get_number(obj: &serde_json::Map<String, serde_json::Value>, key: &str) -> Result<f64, String> {
    let v = obj
        .get(key)
        .ok_or_else(|| format!("rating_scale.{} is required", key))?;
    v.as_f64()
        .ok_or_else(|| format!("rating_scale.{} must be a number", key))
}
