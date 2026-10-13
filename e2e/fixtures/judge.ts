import OpenAI from "openai"
import { config } from "dotenv"
import path from "node:path"
import { fileURLToPath } from "node:url"
import { PNG } from "pngjs"
import { mkdirSync, writeFileSync } from "node:fs"

const __dirname = path.dirname(fileURLToPath(import.meta.url))
config({ path: path.resolve(__dirname, "../../.env") })

// Lazy: `new OpenAI()` throws without OPENAI_API_KEY, and this module is
// imported by keyless standard-suite runs too (pixels.ts re-uses CARD_REGION).
// A judge test without a key still fails loudly — just at visualAssert time,
// not at import time in unrelated files.
let client: OpenAI | undefined
function getClient(): OpenAI {
    return (client ??= new OpenAI())
}

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
 * Send a screenshot to the judge model (default gpt-6-luna) and judge whether
 * it satisfies the assertion.
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
    // OPENAI_JUDGE_DUMP=<dir>: keep every image exactly as the judge saw it,
    // so a surprising verdict can be checked against the pixels it was given.
    if (process.env.OPENAI_JUDGE_DUMP) {
        const dir = process.env.OPENAI_JUDGE_DUMP
        mkdirSync(dir, { recursive: true })
        const name = assertion.slice(0, 40).replace(/[^a-z0-9]+/gi, "-").toLowerCase()
        writeFileSync(path.join(dir, `${Date.now()}-${name}.png`), cropped)
    }

    // gpt-6-luna is the default ($0.10→$0.50 /1M, the cheapest current-gen
    // vision model). Override with OPENAI_JUDGE_MODEL.
    //
    // GPT-5.x / GPT-6.x / o-series are *reasoning* models: on chat/completions
    // they reject `max_tokens` (need `max_completion_tokens`), burn hidden
    // reasoning tokens out of that budget, and don't take a custom
    // `temperature`. Legacy gpt-4o* use the old shape. Branch so either family
    // works.
    const model = process.env.OPENAI_JUDGE_MODEL ?? "gpt-6-luna"
    const isReasoning = /^(gpt-[5-9]|o)/.test(model)
    // Reasoning models run at temperature=1 (they REJECT temperature:0), so
    // their verdict has some sampling jitter; effort is the only stability /
    // quality lever. Default "low"; raise via OPENAI_JUDGE_EFFORT.
    const effort = (process.env.OPENAI_JUDGE_EFFORT ?? "low") as
        | "low"
        | "medium"
        | "high"

    const response = await getClient().chat.completions.create({
        model,
        response_format: { type: "json_object" },
        // High cap so reasoning tokens don't starve the JSON; billed only for
        // tokens actually generated.
        ...(isReasoning
            ? { max_completion_tokens: 2000, reasoning_effort: effort }
            : { max_tokens: 200, temperature: 0 }),
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

// Prompts describe what the product actually looks like, in measurable
// terms. A current-gen judge takes every word literally: "occupying a
// substantial portion of the window" false-failed a correct 480×130 card in an
// 800×600 window (13% of its area), and "rows separated by divider lines"
// was read as one line per row, so three rows with two lines between them
// failed. Older, more lenient judges let those claims through, which is how
// they survived.

/** Full screen — no region. */
export const PROMPT_CARD_PRESENT =
    "A dark rounded rectangular card with text rows inside it is visible, horizontally centred in the upper half of the screenshot. It spans roughly half the screenshot's width (it is a panel with several lines of text, not a small icon or pill)."

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
    "The dark card contains exactly three horizontal rows of text stacked vertically, one line of text per row, with a thin horizontal line between neighbouring rows. (Count the rows of text, not the lines between them.)"

/** Use with `region: CARD_REGION`. Substitute the typed text into the prompt. */
export function promptTypedText(text: string): string {
    return `The text '${text}' is visible somewhere in the image, written in a text input field.`
}
