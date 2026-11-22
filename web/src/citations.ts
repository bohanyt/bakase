import { decodeString } from "micromark-util-decode-string";

// Bentuk sitasi dan batas kiri mengikuti pemindaian Markdown di agent.rs.
const CHAIN = /(?:\[[0-9,\p{White_Space}–-]+\])+/gu;
const IDENTIFIER_END = /[A-Za-z0-9_\]]/;
const encoder = new TextEncoder();
export const SOURCE_LINK = "#sumber-";

/** Nomor di dalam `[1, 2]`, `[1-3]`, atau `[1 - 3]`. */
function citedNumbers(inner: string): number[] {
  const out: number[] = [];
  if (encoder.encode(inner).length > 30) return out;
  const compact = inner.replace(/\p{White_Space}*[–-]\p{White_Space}*/gu, "-");
  for (const part of compact.split(/[,\p{White_Space}]+/u)) {
    const range = part.match(/^(\d+)-(\d+)$/);
    if (range) {
      const start = Number(range[1]);
      const end = Number(range[2]);
      if (end <= 0xffffffff) for (let n = start; n <= end && n < start + 20; n++) out.push(n);
    } else if (/^\d+$/.test(part) && Number(part) <= 0xffffffff) {
      out.push(Number(part));
    }
  }
  return out;
}

/** Bentuk minimal node mdast yang dipakai di sini. */
type MdNode = {
  type: string;
  value?: string;
  url?: string;
  children?: MdNode[];
  position?: { start: { offset?: number }; end: { offset?: number } };
};

/** Node yang isinya tidak boleh diubah jadi sitasi. */
const SKIP = new Set(["code", "inlineCode", "link", "linkReference", "html", "definition", "list", "image", "imageReference"]);

function collectText(value: string, text: string, start: number, end: number, known: Set<number>, found: Set<number>) {
  const raw = text.slice(start, end);
  if (!raw.includes("[")) return value;
  const edits: [number, number][] = [];
  // mdast menyimpan teks tanpa escape, entitas, dan awalan baris lanjutan daftar/kutipan.
  const decode = raw === value || decodeString(raw) === value ? decodeString :
    (part: string) => decodeString(part.replace(/\r\n?/g, "\n").replace(/\n(?: {0,3}>[ \t]?)*[ \t]*/g, "\n"));
  if (raw !== value && decode(raw) !== value) return value;
  const offset = raw === value ? (at: number) => at : (at: number) => decode(raw.slice(0, at)).length;
  for (const chain of raw.matchAll(CHAIN)) {
    let at = chain.index;
    const opening = start + at;
    let escape = opening;
    while (text[escape - 1] === "\\") escape--;
    if (IDENTIFIER_END.test(text.charAt(opening - 1)) || (opening - escape) % 2) continue;
    for (const group of chain[0].matchAll(/\[([^\]]+)\]/g)) {
      const stop = at + group[0].length;
      const numbers = citedNumbers(group[1]).filter((number) => known.has(number));
      if (numbers.length) {
        for (const n of numbers) found.add(n);
        edits.push([offset(at), offset(stop)]);
      }
      at = stop;
    }
  }
  for (const [from, to] of edits.reverse()) {
    const next = value[to];
    const before = value.slice(0, from);
    value = (next && !/[\s[.,;:!?)]/.test(next) ? before : before.replace(/[ \t]+$/, "")) + value.slice(to);
  }
  return value;
}

function collect(node: MdNode, text: string, known: Set<number>, found: Set<number>) {
  if (SKIP.has(node.type)) return;
  const start = node.position?.start.offset;
  const end = node.position?.end.offset;
  if (node.type === "text" && node.value && start !== undefined && end !== undefined) {
    node.value = collectText(node.value, text, start, end, known, found);
  }
  for (const child of node.children ?? []) collect(child, text, known, found);
}

function addChip(node: MdNode, numbers: Set<number>, groupId: string) {
  const children = node.children ?? (node.children = []);
  const last = children.at(-1);
  if (last?.type === "text") last.value = last.value?.trimEnd();
  if (children.some((child) => child.type !== "text" || child.value?.trim())) {
    children.push({ type: "text", value: " " });
  }
  children.push({ type: "link", url: `${SOURCE_LINK}${[...numbers].join(",")}/${groupId}`, children: [] });
}

/** Streamdown menyimpan prosesor berdasarkan nama dan opsi plugin. */
export function remarkCitations(options?: { numbers?: number[]; streaming?: boolean }) {
  const known = new Set(options?.numbers ?? []);
  return (tree: MdNode, file: { toString(): string }) => {
    if (!known.size) return;
    const text = file.toString();
    const end = text.trimEnd().length;
    const blankLine = /\n[ \t\r]*\n[ \t\r]*$/.test(text);
    const complete = (node: MdNode, inList = false) =>
      !options?.streaming || (node.position?.end.offset ?? end) < end || (!inList && blankLine);

    function group(node: MdNode, parent = "") {
      const children = node.children ?? [];
      const blocks: MdNode[] = [];
      for (const [index, child] of children.entries()) {
        const groupId = `${parent}${index}`;
        blocks.push(child);
        if (child.type === "list" || child.type === "blockquote") {
          group(child, `${groupId}.`);
          continue;
        }
        const numbers = new Set<number>();
        if (child.type === "listItem") {
          const content = child.children ?? [];
          const own = content.filter((part) => part.type !== "list" && part.type !== "blockquote");
          own.forEach((part) => collect(part, text, known, numbers));
          content.forEach((part, i) => {
            if (!own.includes(part)) group(part, `${groupId}.${i}.`);
          });
          if (!numbers.size || !complete(child, true)) continue;
          const last = own.at(-1);
          if (last?.type === "paragraph") {
            addChip(last, numbers, groupId);
          } else {
            const row = { type: "paragraph" };
            addChip(row, numbers, groupId);
            content.splice(last ? content.indexOf(last) + 1 : 0, 0, row);
          }
        } else {
          collect(child, text, known, numbers);
          if (!numbers.size || !complete(child)) continue;
          const row = child.type === "table" ? { type: "paragraph" } : child;
          addChip(row, numbers, groupId);
          if (row !== child) blocks.push(row);
        }
      }
      node.children = blocks;
    }

    group(tree);
  };
}
