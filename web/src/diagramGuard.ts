// Diagram Mermaid dari model: hanya flowchart teks tanpa CSS. Node gambar, ikon, tautan, klik,
// HTML, CSS, dan direktif %%{...}%% bisa membuat browser memuat alamat luar atau mengubah aturan keamanan
// Mermaid, jadi ditolak sebelum digambar. Diagram yang ditolak tampil sebagai kode.

const BLOCKED: readonly RegExp[] = [
  /:\/\//, // alamat lengkap
  /\b(img|icon|image)\s*:/i, // node gambar dan ikon
  /@\{/, // bentuk node baru yang bisa membawa gambar
  /\b(click|href)\b/i, // tautan dan klik, termasuk "click A call ..."
  /%%\{/, // direktif yang bisa mengganti konfigurasi
  /url\s*\(/i, // gambar lewat style
  /<(?!br\s*\/?>)[a-z!/]/i, // HTML selain <br>
  /(^|;)\s*(classDef|class|style|linkStyle)\b/im, // CSS dari model, juga sesudah pemisah ;
  /:::/, // kelas CSS singkat pada node
];

export function allowedDiagram(source: string): boolean {
  const first = source
    .split("\n")
    .map((line) => line.trim())
    .find((line) => line && !line.startsWith("%%"));
  if (!first || !/^(flowchart|graph)\b/i.test(first)) return false;
  return !BLOCKED.some((re) => re.test(source));
}
