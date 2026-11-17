export type TriggerSource = "timer" | "manual";
export type CloseAction = "save" | "dismiss" | "switch";

export const INITIAL_TRIGGER_SOURCE: TriggerSource = "timer";

export function restartsTimer(action: CloseAction, source: TriggerSource): boolean {
  if (action === "save") return true;
  return source === "timer";
}
