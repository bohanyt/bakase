import type { ComponentProps } from "react";
import { defaultRemarkPlugins, type Streamdown } from "streamdown";

type Node = { type: string; children?: Node[] };
type RemarkPlugins = NonNullable<ComponentProps<typeof Streamdown>["remarkPlugins"]>;

/** Plugin remark: HTML di dalam markdown (`<Review-01>`, `<br>`) dibaca sebagai teks biasa. Dikerjakan
 * di pohon markdown, jadi isi code span tidak ikut di-escape (`R&D <Review-01>` tetap utuh). */
export function htmlAsText() {
  return (tree: Node) => {
    const walk = (node: Node) => {
      if (node.type === "html") node.type = "text";
      node.children?.forEach(walk);
    };
    walk(tree);
  };
}

/** Plugin bawaan Streamdown (GFM, dll.) ditambah `htmlAsText`, untuk isi dokumen kerja yang tidak
 * boleh dibaca sebagai HTML. */
export const DOCUMENT_REMARK_PLUGINS: RemarkPlugins = [...Object.values(defaultRemarkPlugins), htmlAsText];
