import { test } from "node:test";
import assert from "node:assert/strict";
import { allowedDiagram } from "./diagramGuard.ts";

test("flowchart teks biasa boleh digambar", () => {
  assert.ok(allowedDiagram("flowchart LR\n  A[Operator lapor] --> B{Perlu sparepart?}\n  B -- Ya --> C[Ajukan PR]"));
  assert.ok(allowedDiagram("%% alur perbaikan\ngraph TD\n  A[Baris satu<br/>baris dua] --> B"));
});

test("jenis lain, gambar, tautan, HTML, dan direktif ditolak", () => {
  const bad = [
    "sequenceDiagram\n  A->>B: halo",
    "flowchart LR\n  A --> B[https://contoh.com/x.png]",
    'flowchart LR\n  A@{ img: "/x.png", label: "x" }',
    "flowchart LR\n  A --> B\n  click A href \"/x\"",
    'flowchart LR\n  A --> B\n  click A call alert("x")',
    '%%{init: {"securityLevel": "loose"}}%%\nflowchart LR\n  A --> B',
    "flowchart LR\n  A[<img src=x>] --> B",
    "flowchart LR\n  A --> B\n  style A fill:url(/x.png)",
    "flowchart LR\n  A --> B\n  classDef x fill:#f00\n  class A x",
    "flowchart LR\n  A:::x --> B",
    "flowchart LR\n  A --> B\n  linkStyle 0 stroke:#f00",
    "flowchart LR; A --> B; classDef x display:none; class A x",
    "",
  ];
  for (const source of bad) assert.equal(allowedDiagram(source), false, source);
});
