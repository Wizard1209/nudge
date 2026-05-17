import OpenAI from "openai"
import { config } from "dotenv"
import path from "node:path"
import { fileURLToPath } from "node:url"
import { PNG } from "pngjs"

const __dirname = path.dirname(fileURLToPath(import.meta.url))
config({ path: path.resolve(__dirname, "../../.env") })

const client = new OpenAI()

export interface Judgment {
    pass: boolean
    comment: string
}

export interface Region {
    x: number
    y: number
    w: number
    h: number
}

// Canonical regions for the 800×600 viewport. Cropping before sending to the
// judge keeps the relevant pixels at full resolution while letting us use
// `detail: "low"` for cheap, rate-limit-safe API calls.
export const CARD_REGION: Region = { x: 130, y: 130, w: 540, h: 180 }
export const BOTTOM_RIGHT_REGION: Region = { x: 600, y: 460, w: 200, h: 140 }

function cropPng(buf: Buffer, r: Region): Buffer {
    const src = PNG.sync.read(buf)
    const out = new PNG({ width: r.w, height: r.h })
    for (let y = 0; y < r.h; y++) {
        for (let x = 0; x < r.w; x++) {
            const si = ((r.y + y) * src.width + (r.x + x)) * 4
            const di = (y * r.w + x) * 4
            out.data[di] = src.data[si]
            out.data[di + 1] = src.data[si + 1]
            out.data[di + 2] = src.data[si + 2]
            out.data[di + 3] = src.data[si + 3]
        }
    }
    return PNG.sync.write(out)
}

/**
 * Send a screenshot to gpt-4o-mini and judge whether it satisfies the assertion.
 *
 * Stability comes from three things:
 *   1) Prompts assert ONE atomic visual fact (no "X visible AND no Y" compounds).
 *   2) Optional `region` crops the screenshot to the relevant area — the cropped
 *      PNG stays small enough to send at `detail: "low"` without rate-limit pain
 *      while preserving fine details (dividers, small pill) inside the crop.
 *   3) No silent retries: if the judge says no, the test fails immediately.
 */
export async function visualAssert(
    screenshot: Buffer,
    assertion: string,
    opts: { region?: Region } = {}
): Promise<Judgment> {
    const cropped = opts.region ? cropPng(screenshot, opts.region) : screenshot
    const base64 = cropped.toString("base64")

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
                            detail: "low",
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
    try {
        return JSON.parse(text) as Judgment
    } catch {
        return { pass: false, comment: `invalid JSON: ${text}` }
    }
}

// ────────────────────────────────────────────────────────────────────
// Reusable atomic prompts. Pair each with the region indicated.
// ────────────────────────────────────────────────────────────────────

/** Full screen — no region. */
export const PROMPT_CARD_PRESENT =
    "A single large rounded rectangular card with a dark translucent fill is clearly visible in the screenshot, occupying a substantial portion of the window area (much larger than a small icon or pill)."

/** Full screen — no region. */
export const PROMPT_CARD_ABSENT =
    "The screenshot shows only the wallpaper / background and at most a small pill or icon in a corner. There is NO large rectangular card or panel anywhere in the screenshot."

/** Full screen — no region. */
export const PROMPT_CARD_FROSTED =
    "The card has visibly rounded corners and a soft shadow around it. The card's fill is dark but translucent — wallpaper colour bleeds through slightly, like frosted glass — rather than a flat opaque block."

/** Use with `region: BOTTOM_RIGHT_REGION`. */
export const PROMPT_PILL_PRESENT =
    "A small pill-shaped element (a rounded rectangle clearly smaller than a third of the window width) is visible in the bottom-right corner of the screenshot."

/** Use with `region: CARD_REGION`. */
export const PROMPT_THREE_ROWS =
    "Inside the dark card there are exactly three horizontal rows stacked vertically, separated by thin horizontal divider lines. Each row is a single strip of one text-line in height (not two)."

/** Use with `region: CARD_REGION`. Substitute the typed text into the prompt. */
export function promptTypedText(text: string): string {
    return `The text '${text}' is visible somewhere in the image, written in a text input field.`
}
