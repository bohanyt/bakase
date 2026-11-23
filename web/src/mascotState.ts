import type { ThreadMessage } from "@assistant-ui/react";
import type { AnswerMeta } from "./messages.ts";

export function workingMascotState(content: ThreadMessage["content"], custom?: AnswerMeta) {
  if (custom?.compacting) return "tired";
  // Langkah tool dari server disimpan di metadata, lewat event SSE step.
  if (custom?.steps?.some((step) => !step.done) || content.some(
    (part) => part.type === "tool-call" && (part.isPreliminary || (part.result === undefined && !part.isError)),
  )) return "focus";
  const lastTool = content.findLastIndex((part) => part.type === "tool-call");
  const hasText = custom?.hasTextAfterTool ?? content.slice(lastTool + 1).some(
    (part) => part.type === "text" && part.text.trim(),
  );
  return hasText ? "bicara" : "mikir";
}
