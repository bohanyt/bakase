type TextOffset = { item: number; offset: number };

const DIGIT = /\p{Nd}/u;
const DASH = /[\u2010-\u2012\u2015\u2212\uFE63\uFF0D]/gu;
// The only spaces left in normalized text are between groups of digits.
const NUMBER_SIGN = /[ .,/%±-]/;

function normalize(items: string[]) {
  const characters = items.flatMap((text, item) => {
    let offset = 0;
    return Array.from(text, (character) => {
      const start = { item, offset };
      offset += character.length;
      const value = character.replace(DASH, "-").normalize("NFKD").toLowerCase().replaceAll("\u2044", "/");
      return { value, start, end: { item, offset } };
    });
  });
  const starts: TextOffset[] = [];
  const ends: TextOffset[] = [];
  const parts: string[] = [];
  let previous = "";
  let next = 0;
  for (const [index, { value, start, end }] of characters.entries()) {
    next = Math.max(next, index + 1);
    while (next < characters.length && /^\s$/u.test(characters[next].value)) next++;
    const beforeDigit = DIGIT.test(characters[next]?.value ?? "");
    const afterDigit = DIGIT.test(previous);
    const space = /^\s$/u.test(value);
    let part = value.replace(/[^\p{L}\p{N}]/gu, "");
    if (/^\d+\/\d*$/.test(value)) {
      const mixed = /^\p{Nd}+$/u.test(characters[index - 1]?.value ?? "");
      part = mixed ? ` ${value}` : value;
    }
    if ((beforeDigit && value === "-") || (/^[–—]$/.test(value) && DIGIT.test(characters[index + 1]?.value ?? ""))) part = "-";
    if (beforeDigit && /^[.,]$/.test(value)) part = value;
    if ((beforeDigit || afterDigit) && value === "/") part = value;
    if (value === "%" || value === "±") part = value;
    if (space && afterDigit && beforeDigit && !/^\s$/u.test(characters[index - 1]?.value ?? "")) part = " ";
    if (!space) previous = value;
    parts.push(part);
    for (let i = 0; i < part.length; i++) {
      starts.push(start);
      ends.push(end);
    }
  }
  return { text: parts.join(""), starts, ends };
}

// Keep UTF-16 offsets for the selectable text layer after normalizing extraction differences.
export function findPdfHighlight(items: string[], excerpt: string) {
  const { text: needle } = normalize([excerpt]);
  if (!/[\p{L}\p{N}]/u.test(needle)) return null;
  const { text: haystack, starts, ends } = normalize(items);
  for (let at = haystack.indexOf(needle); at >= 0; at = haystack.indexOf(needle, at + 1)) {
    const stop = at + needle.length;
    if (DIGIT.test(needle[0]) && NUMBER_SIGN.test(haystack[at - 1] ?? "")) continue;
    if (DIGIT.test(needle.at(-1) ?? "") && NUMBER_SIGN.test(haystack[stop] ?? "")) continue;
    const start = starts[at];
    const end = ends[stop - 1];
    const before = items[start.item].slice(0, start.offset);
    const after = items[end.item].slice(end.offset);
    if (!/[\p{L}\p{N}]$/u.test(before) && !/^[\p{L}\p{N}]/u.test(after)) return { start, end };
  }
  return null;
}
