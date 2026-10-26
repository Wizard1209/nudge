import { readFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

// OPENAI_API_KEY comes from the environment or the repo-root .env.
function apiKey() {
    if (process.env.OPENAI_API_KEY) return process.env.OPENAI_API_KEY;
    const envPath = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../../../../.env');
    const line = readFileSync(envPath, 'utf-8').split(/\r?\n/).find((l) => l.startsWith('OPENAI_API_KEY='));
    if (!line) throw new Error(`OPENAI_API_KEY is neither set nor in ${envPath}`);
    return line.slice('OPENAI_API_KEY='.length).trim().replace(/^["']|["']$/g, '');
}

/**
 * Ask a vision model whether a screenshot satisfies ONE atomic visual fact.
 * Default gpt-5.4-mini: these are fine geometric judgments (gaps, which side
 * of an edge), beyond what nano does reliably. Override: OPENAI_JUDGE_MODEL,
 * OPENAI_JUDGE_EFFORT.
 */
export async function visualAssert(pngBase64, assertion) {
    const model = process.env.OPENAI_JUDGE_MODEL ?? 'gpt-5.4-mini';
    const isReasoning = model.startsWith('gpt-5') || model.startsWith('o');
    const res = await fetch('https://api.openai.com/v1/chat/completions', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json', Authorization: `Bearer ${apiKey()}` },
        body: JSON.stringify({
            model,
            response_format: { type: 'json_object' },
            // Reasoning models reject max_tokens/temperature and spend hidden
            // reasoning tokens from this cap.
            ...(isReasoning
                ? { max_completion_tokens: 2000, reasoning_effort: process.env.OPENAI_JUDGE_EFFORT ?? 'low' }
                : { max_tokens: 200, temperature: 0 }),
            messages: [{
                role: 'user',
                content: [
                    { type: 'image_url', image_url: { url: `data:image/png;base64,${pngBase64}`, detail: 'high' } },
                    {
                        type: 'text',
                        text: `Look at this screenshot of a desktop popup. Does it satisfy this assertion?\n\nAssertion: ${assertion}\n\nRespond ONLY with valid JSON: {"pass": true/false, "comment": "brief explanation"}`,
                    },
                ],
            }],
        }),
    });
    if (!res.ok) throw new Error(`judge HTTP ${res.status}: ${await res.text()}`);
    const text = (await res.json()).choices[0].message.content ?? '{}';
    try {
        return JSON.parse(text);
    } catch {
        return { pass: false, comment: `invalid JSON: ${text}` };
    }
}
