import type { ComponentProps } from "react";

/** Gambar di jawaban model tidak dimuat: URL-nya bisa membawa isi chat ke situs luar. */
export function BlockedImage({ alt }: ComponentProps<"img">) {
  return (
    <span className="img-blocked" title="Gambar dari jawaban tidak ditampilkan">
      {alt ? `[Gambar: ${alt}]` : "[Gambar]"}
    </span>
  );
}
