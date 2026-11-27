import type { SourceRef } from "./messages.ts";

export type ScanPageLink = { source: SourceRef; page: number; unchecked: boolean };

// Sumber yang dikirim untuk jawaban sudah disaring oleh server sesuai sitasi.
export function scanPageLinks(sources: readonly SourceRef[]): ScanPageLink[] {
  const documents = new Map<number, ScanPageLink>();
  for (const source of sources) {
    if (!source.scan) continue;
    const handwriting = source.handwriting;
    const unchecked = handwriting === "belum_dicek";
    const first = Array.isArray(handwriting) ? handwriting[0] : undefined;
    if (!unchecked && first === undefined) continue;
    if (Array.isArray(handwriting) && source.page != null && !handwriting.includes(source.page)) continue;
    const previous = documents.get(source.doc_id);
    if (!previous || (previous.unchecked && !unchecked)) {
      documents.set(source.doc_id, { source, page: source.page ?? first ?? 1, unchecked });
    }
  }
  return [...documents.values()].slice(0, 3);
}
