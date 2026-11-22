import { useSyncExternalStore } from "react";

export type Theme = "light" | "dark";

const KEY = "app.theme";

function stored(): Theme | null {
  try {
    const v = localStorage.getItem(KEY);
    return v === "light" || v === "dark" ? v : null;
  } catch {
    return null;
  }
}

/** Tema yang sedang berlaku: pilihan user, atau mengikuti sistem kalau belum memilih. */
export function currentTheme(): Theme {
  return stored() ?? (window.matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light");
}

/** Dipanggil sebelum render supaya halaman tidak berkedip dengan tema yang salah. */
export function applyStoredTheme() {
  const t = stored();
  if (t) document.documentElement.dataset.theme = t;
}

export function setTheme(t: Theme) {
  document.documentElement.dataset.theme = t;
  try {
    localStorage.setItem(KEY, t);
  } catch {
    /* penyimpanan browser tidak tersedia */
  }
}

function subscribeTheme(onChange: () => void) {
  const attr = new MutationObserver(onChange);
  attr.observe(document.documentElement, { attributes: true, attributeFilter: ["data-theme"] });
  const media = window.matchMedia("(prefers-color-scheme: dark)");
  media.addEventListener("change", onChange);
  return () => {
    attr.disconnect();
    media.removeEventListener("change", onChange);
  };
}

/** Tema yang sedang tampil: atribut data-theme kalau ada, kalau tidak mengikuti sistem. */
function shownTheme(): Theme {
  const t = document.documentElement.dataset.theme;
  if (t === "light" || t === "dark") return t;
  return window.matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light";
}

/** Tema yang tampil, ikut berubah saat user mengganti tema atau tema sistem berubah. Untuk
 * gambar yang warnanya dibaca dari token (grafik, diagram). */
export function useTheme(): Theme {
  return useSyncExternalStore(subscribeTheme, shownTheme);
}
