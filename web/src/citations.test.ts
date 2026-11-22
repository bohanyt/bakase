import assert from "node:assert/strict";
import { test } from "node:test";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { Streamdown, defaultRemarkPlugins } from "streamdown";
import { remarkCitations } from "./citations.ts";

function render(text: string, streaming = false, numbers = [1, 2, 3, 4]) {
  return renderToStaticMarkup(createElement(Streamdown, {
    mode: "static",
    remarkPlugins: [...Object.values(defaultRemarkPlugins), [remarkCitations, { numbers, streaming }]],
    components: { a: ({ href, children }) => createElement("a", { href }, children) },
  }, text));
}

test("satu chip per paragraf atau butir, mengikuti urutan kutipan", () => {
  const html = render("Awal **tebal [2]** lalu salin [1][2]. Akhir [3].\n\n- Satu [1 - 3]\n  - Turunan [3]\n- Dua `[2]` [2, 1] [9]");
  assert.match(html, /tebal<\/span> lalu salin\. Akhir\. <a[^>]+href="#sumber-2,1,3\//);
  assert.match(html, /Satu <a[^>]+href="#sumber-1,2,3\/[^]*Turunan <a[^>]+href="#sumber-3\//);
  assert.match(html, /<code[^>]*>\[2\]<\/code> \[9\] <a[^>]+href="#sumber-2,1\//);
});

test("posisi Markdown asli membedakan sitasi, ekspresi literal, dan tautan referensi", () => {
  const html = render(String.raw`array[4], x[1][2], \[4\], &#91;4],` + " `[1]`.\n\nTeks [1]. Lanjut.[1][2]");
  assert.match(html, /array\[4\], x\[1\]\[2\], \[4\], \[4\], <code[^>]*>\[1\]<\/code>/);
  assert.match(html, /Teks\. Lanjut\. <a[^>]+href="#sumber-1,2\//);
  assert.equal([...html.matchAll(/href="#sumber-/g)].length, 1);

  const link = render("[Formulir][1]\n\n[1]: https://example.com/\n[1]: https://example.net/");
  assert.match(link, /href="https:\/\/example.com\/">Formulir<\/a>/);
  assert.doesNotMatch(link, /href="#sumber-/);
});

test("anggota sitasi yang masih tersedia tetap membentuk chip", () => {
  const html = render("Sebagian [1, 2]. Semua hilang [3, 4].", false, [2]);
  assert.match(html, /Sebagian\. Semua hilang \[3, 4\]\. <a[^>]+href="#sumber-2\//);
  assert.doesNotMatch(render("Semua hilang [1, 2].", false, []), /#sumber-/);
});

test("sitasi boleh menempel setelah CJK, emoji, dan tanda kurung", () => {
  const html = render("array[4]\n\n確認済み[1]\n\n⚠️[1]\n\n(selesai)[1]");
  assert.match(html, /<p>array\[4\]<\/p>/);
  for (const line of ["確認済み", "⚠️", "(selesai)"]) assert.ok(html.includes(line + ' <a href="#sumber-1/'));
  assert.equal([...html.matchAll(/href="#sumber-/g)].length, 3);
});

test("URL polos GFM tetap tampil sebagai tautan tanpa chip", () => {
  const urls = "https://example.com/?ids=[4]&aktif=1 www.example.com/?ids=[4] HTTP://localhost/?ids=[4]";
  const html = render(urls + " Bukti [4].");
  assert.match(html, /href="https:\/\/example.com\/\?ids=%5B4%5D&amp;aktif=1"/);
  assert.match(html, /href="http:\/\/www.example.com\/\?ids=%5B4"/);
  assert.ok(html.includes("HTTP://localhost/?ids=[4"));
  assert.match(html, /Bukti\. <a href="#sumber-4\//);
  assert.equal([...html.matchAll(/href="#sumber-/g)].length, 1);

  const outside = render("https://exa_mple.com/?ids=[4]\n\nhttps://example.com/a]([4])");
  assert.equal([...outside.matchAll(/href="#sumber-4\//g)].length, 2);
});

test("sitasi tabel muncul sesudah tabel; isi kode dan tautan tetap utuh", () => {
  const html = render("| Nama | Catatan |\n| --- | --- |\n| Uji [1] | [2](https://example.com) |\n\n~~~js\narray[2]\n~~~\n\n[3]");
  assert.match(html, /Uji<\/td>[^]*>2<\/a>[^]*<\/table>[^]*href="#sumber-1\//);
  assert.match(html, /array\[2\][^]*href="#sumber-3\//);
});

test("bagian terakhir menunggu penutup; paragraf dan butir sebelumnya langsung mendapat chip", () => {
  const html = render("Awal [1].\n\n- Lengkap [2].\n- Masih ditulis [3]", true);
  assert.match(html, /Awal\. <a[^>]+href="#sumber-1\//);
  assert.match(html, /Lengkap\. <a[^>]+href="#sumber-2\//);
  assert.match(html, /Masih ditulis<\/li>/);
  assert.doesNotMatch(html, /#sumber-3/);
  assert.match(render("Awal [1].\n\n", true), /#sumber-1/);
  assert.doesNotMatch(render("Awal [1].", true), /#sumber-1/);
});

test("identitas sitasi butir tetap sama ketika paragrafnya dibungkus ulang", () => {
  const tight = render("- Awal [1]\n- Berikut [2]", true);
  const loose = render("- Awal\n\n  Lanjut [1]\n- Berikut [2]", true);
  const chip = (html: string) => html.match(/href="(#sumber-1\/[^"]+)"/)?.[1];
  assert.ok(chip(tight));
  assert.equal(chip(loose), chip(tight));
  assert.match(loose, /<p>Awal/);
});
