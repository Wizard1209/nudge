import OpenAI from "openai"
import { config } from "dotenv"
import path from "node:path"
import { fileURLToPath } from "node:url"

const __dirname = path.dirname(fileURLToPath(import.meta.url))
config({ path: path.resolve(__dirname, "../../.env") })

const client = new OpenAI()

export interface Judgment {
    pass: boolean
    comment: string
}

/**
 * Send a screenshot to gpt-4o-mini and judge whether it satisfies the assertion.
 *
 * The LLM judge flakes on repeated runs (content moderation on Russian slang,
 * non-determinism). We retry up to 3 times and return on first `pass: true`.
 */
export async function visualAssert(
    screenshot: Buffer,
    assertion: string,
    opts: { attempts?: number } = {}
): Promise<Judgment> {
    const attempts = opts.attempts ?? 3
    const base64 = screenshot.toString("base64")
    let last: Judgment = { pass: false, comment: "no attempts" }

    for (let i = 0; i < attempts; i++) {
        let response
        try {
            response = await client.chat.completions.create({
                model: "gpt-4o-mini",
                temperature: 0,
                response_format: { type: "json_object" },
                max_tokens: 200,
                messages: [
                    {
                        role: "user",
                        content: [
                            {
                                type: "image_url",
                                image_url: {
                                    url: `data:image/png;base64,${base64}`,
                                },
                            },
                            {
                                type: "text",
                                text: `Look at this screenshot of a desktop application. Does it satisfy this assertion?

Assertion: ${assertion}

Respond ONLY with valid JSON: {"pass": true/false, "comment": "brief explanation"}`,
                            },
                        ],
                    },
                ],
            })
        } catch (e: any) {
            // OpenAI 429 rate-limit → wait and retry
            if (e?.status === 429) {
                const wait = 20_000
                console.log(`[judge] 429 rate limit, sleeping ${wait}ms before retry ${i + 1}/${attempts}`)
                await new Promise((r) => setTimeout(r, wait))
                continue
            }
            throw e
        }

        const text = response.choices[0].message.content ?? "{}"
        try {
            last = JSON.parse(text) as Judgment
        } catch {
            last = { pass: false, comment: `invalid JSON: ${text}` }
        }
        if (last.pass) return last
    }
    return last
}
