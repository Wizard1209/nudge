import OpenAI from "openai"
import { config } from "dotenv"
import path from "node:path"
import { fileURLToPath } from "node:url"
import { PNG } from "pngjs"
import { mkdirSync, writeFileSync } from "node:fs"

const __dirname = path.dirname(fileURLToPath(import.meta.url))
config({ path: path.resolve(__dirname, "../../.env") })

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

export const CARD_REGION: Region = { x: 130, y: 130, w: 540, h: 180 }
export const BOTTOM_RIGHT_REGION: Region = { x: 600, y: 460, w: 200, h: 140 }
export const CARD_ERROR_REGION: Region = { x: 130, y: 130, w: 540, h: 230 }
export const SETTINGS_REGION: Region = { x: 0, y: 0, w: 800, h: 320 }

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

export async function visualAssert(
    screenshot: Buffer,
    assertion: string,
    opts: { region?: Region } = {}
): Promise<Judgment> {
    const cropped = opts.region ? cropPng(screenshot, opts.region) : screenshot
    const base64 = cropped.toString("base64")
    if (process.env.OPENAI_JUDGE_DUMP) {
        const dir = process.env.OPENAI_JUDGE_DUMP
        mkdirSync(dir, { recursive: true })
        const name = assertion.slice(0, 40).replace(/[^a-z0-9]+/gi, "-").toLowerCase()
        writeFileSync(path.join(dir, `${Date.now()}-${name}.png`), cropped)
    }

    // GPT-6 models do not support `temperature` unless reasoning effort is `none`.
    // proof: https://developers.openai.com/api/docs/guides/latest-model/gpt-6-astra.md
    const model = process.env.OPENAI_JUDGE_MODEL ?? "gpt-6-luna"
    const isReasoning = /^(gpt-[5-9]|o)/.test(model)
    const effort = (process.env.OPENAI_JUDGE_EFFORT ?? "low") as
        | "low"
        | "medium"
        | "high"

    const response = await getClient().chat.completions.create({
        model,
        response_format: { type: "json_object" },
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

export const PROMPT_CARD_PRESENT =
    "A dark rounded rectangular card with text rows inside it is visible, horizontally centred in the upper half of the screenshot. It spans roughly half the screenshot's width (it is a panel with several lines of text, not a small icon or pill)."

export const PROMPT_CARD_ABSENT =
    "The screenshot shows only the wallpaper / background and at most a small pill or icon in a corner. There is NO large rectangular card or panel anywhere in the screenshot."

export const PROMPT_PLACEHOLDERS =
    "The dark card has three rows of text. Read top to bottom they are: 'Что я делаю?' in grey, 'Хуйня?' in grey, and '10' in white. (The first row may show a thin text cursor before its text.)"

export function promptTypedText(text: string): string {
    return `The top row of the dark card shows the text '${text}'.`
}

export const PROMPT_SUBMIT_ERROR =
    "The dark card is visible with its three text rows, and below the third row, still inside the card, there is one line of small red text that begins with the word 'Interval'."

export const PROMPT_ROUNDED_CORNERS =
    "The dark card is a rounded rectangle: all four of its corners are visibly rounded, none of them is a sharp 90-degree corner."

export const PROMPT_PILL_PRESENT =
    "A small pill-shaped element (a rounded rectangle clearly smaller than a third of the window width) is visible in the bottom-right corner of the screenshot."

export const PROMPT_SETTINGS_HOTKEY_ROW =
    "There is a text field labelled 'Global hotkey:' and the field contains the text 'Ctrl+Shift+Space'. Directly under that field is a line of smaller, dimmer text that begins with the word 'Modifiers'."

export const PROMPT_SETTINGS_FIELDS =
    "There is a text field labelled 'Default interval' containing '10', with 'min' right after the field, and below it a checkbox whose label reads 'Launch with Windows'."

export const PROMPT_SETTINGS_ARROWS_ARE_WORDS =
    "In the line of small dim text under the hotkey field, between the words 'Backspace,' and 'Home,' the line reads 'Left, Right, Up, Down' — four words, not arrow symbols and not empty boxes."
