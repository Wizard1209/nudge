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
 */
export async function visualAssert(
    screenshot: Buffer,
    assertion: string
): Promise<Judgment> {
    const base64 = screenshot.toString("base64")

    const response = await client.chat.completions.create({
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

    const text = response.choices[0].message.content ?? "{}"
    return JSON.parse(text) as Judgment
}
