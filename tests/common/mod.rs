//! Shared by the two native harnesses: `native_render.rs` (Windows) and
//! `macos_native.rs`.
//!
//! Only the vision judge lives here. Everything else in those files is
//! platform API — `BitBlt` against `screencapture`, `EnumWindows` against
//! `CGWindowListCopyWindowInfo` — and has nothing to share.

use base64::Engine as _;

/// The judge's credential, from the environment or from the repo's `.env`.
///
/// `.env` is where the browser tier already keeps it
/// (`e2e/fixtures/judge.ts` loads exactly this file), and it is gitignored.
/// Reading it here too means one key in one place for both judge tiers,
/// instead of the native one silently no-op-passing on a machine that is in
/// fact configured.
///
/// Parsed rather than pulled in with a crate: one `KEY=value` line does not
/// justify a dependency in the test tree, and the file this reads is written
/// by hand.
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

/// Ask an LLM vision model whether a screenshot renders correctly.
///
/// This tier exists because the pixel heuristics next to it are all
/// *specific*: each one measures a quantity someone already knew to measure.
/// A render can satisfy every one of them and still be visibly wrong in a way
/// nobody wrote an assertion for — the judge is the only check that can fail
/// for a reason its author did not anticipate.
///
/// So prompts here should describe the *bug class* and leave the model free
/// about how it recognises one; a prompt that lists what to look at buys
/// nothing the heuristics do not already give.
///
/// Calibrate before relying on one. Measured on the macOS card (see
/// `card_passes_the_vision_judge`): a torn-open surface is caught, a
/// few-point misalignment of two nested rounded rectangles is not — and not
/// at any model or detail setting tried, including a prompt naming that exact
/// fault. Treat a green verdict as "nothing gross", never as "looks right".
///
/// Returns `None` when `OPENAI_API_KEY` is unset, so a machine without
/// credentials skips rather than fails. Callers treat that as a pass; the
/// alternative is a suite that is red on every fresh checkout.
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

    // ── Judge model menu (OpenAI API, current as of 2026-10) ─────────────
    // Per-call cost is a fraction of a cent on ANY of these (one small
    // screenshot + tiny JSON, run rarely), so pick for vision quality, not
    // price. The bug that set this floor was a ~20px text inset on a
    // transparent card over a busy desktop — fine enough that gpt-4o-mini's
    // vision CAN'T resolve it and false-fails a CORRECT render at full/half/low
    // res alike (a capability ceiling, not a cost knob — observed). gpt-4o
    // cleared it; so did gpt-5.4-mini and gpt-6-luna.
    //
    //   OPENAI_JUDGE_MODEL    in→out $/1M     notes
    //   gpt-6-sol             $2.00 → $10.00   current-gen flagship tier
    //   gpt-6-luna    (def)   $0.10 →  $0.50   current-gen small; cleared the floor
    //   gpt-5.4-mini          $0.75 →  $4.50   previous default
    //   gpt-5.4-nano          $0.20 →  $1.25   coarse checks only
    //   gpt-4o                $2.50 → $10.00   legacy, cleared that bug
    //   gpt-4o-mini           legacy           too weak — false-fails (see above)
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
    // OPENAI_JUDGE_DETAIL "low" (default) sends one flat ~512px / ~85-token
    // tile — judged correctly and ~13x cheaper than "high".
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
        // Generous cap so reasoning tokens don't starve the JSON answer; you're
        // billed only for tokens actually generated, so a high cap is free.
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
