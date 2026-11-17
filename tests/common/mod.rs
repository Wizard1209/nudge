use base64::Engine as _;

fn api_key() -> Option<String> {
    if let Ok(k) = std::env::var("OPENAI_API_KEY")
        && !k.trim().is_empty()
    {
        return Some(k);
    }
    let dotenv = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(".env");
    let text = std::fs::read_to_string(dotenv).ok()?;
    text.lines()
        .filter_map(|line| line.trim().strip_prefix("OPENAI_API_KEY="))
        .map(|v| v.trim().trim_matches(['"', '\'']).to_string())
        .find(|v| !v.is_empty())
}

pub fn vision_verdict(png: &[u8], prompt: &str) -> Option<(bool, String)> {
    let api_key = match api_key() {
        Some(k) => k,
        None => {
            eprintln!(
                "[llm-judge] no OPENAI_API_KEY in the environment or in .env — \
                 skipping (no-op pass)"
            );
            return None;
        }
    };

    let model = std::env::var("OPENAI_JUDGE_MODEL").unwrap_or_else(|_| "gpt-6-luna".to_string());

    // GPT-5.x / GPT-6.x / o-series are *reasoning* models: on
    // /v1/chat/completions they REJECT `max_tokens` (require
    // `max_completion_tokens`), spend hidden reasoning tokens out of that
    // budget before the JSON, and don't take a custom `temperature`. Legacy
    // gpt-4o* use the old `max_tokens` + `temperature` shape. Branch on the
    // family so OPENAI_JUDGE_MODEL "just works" either way.
    let is_reasoning = model
        .strip_prefix("gpt-")
        .and_then(|rest| rest.chars().next())
        .is_some_and(|major| major >= '5')
        || model.starts_with('o');
    let effort = std::env::var("OPENAI_JUDGE_EFFORT").unwrap_or_else(|_| "low".to_string());
    let detail = std::env::var("OPENAI_JUDGE_DETAIL").unwrap_or_else(|_| "low".to_string());
    eprintln!(
        "[llm-judge] model={model} reasoning={is_reasoning} effort={effort} detail={detail} png_bytes={}",
        png.len()
    );

    let data_url = format!(
        "data:image/png;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(png)
    );
    let mut request = serde_json::json!({
        "model": model,
        "response_format": { "type": "json_object" },
        "messages": [{
            "role": "user",
            "content": [
                { "type": "text", "text": prompt },
                { "type": "image_url", "image_url": { "url": data_url, "detail": detail } }
            ]
        }]
    });
    if is_reasoning {
        request["max_completion_tokens"] = serde_json::json!(4000);
        request["reasoning_effort"] = serde_json::json!(effort);
    } else {
        request["max_tokens"] = serde_json::json!(200);
        request["temperature"] = serde_json::json!(0);
    }

    let response = ureq::post("https://api.openai.com/v1/chat/completions")
        .set("Authorization", &format!("Bearer {api_key}"))
        .send_json(request)
        .expect("OpenAI request failed");
    let body: serde_json::Value = response.into_json().expect("parse OpenAI response");

    let content = body["choices"][0]["message"]["content"]
        .as_str()
        .unwrap_or_else(|| panic!("no content in OpenAI response: {body}"));
    eprintln!("[llm-judge] verdict: {content}");

    let verdict: serde_json::Value =
        serde_json::from_str(content).expect("judge did not return JSON");
    Some((
        verdict["pass"].as_bool().unwrap_or(false),
        verdict["reason"]
            .as_str()
            .unwrap_or("(no reason given)")
            .to_string(),
    ))
}
